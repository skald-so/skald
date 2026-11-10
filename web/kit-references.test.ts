// @vitest-environment node
// The kit page in the pinned Chrome against its reference screenshots in kit-references/.
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, relative } from "node:path";
import type { Page } from "playwright-core";
import { PNG } from "pngjs";
import { afterAll, beforeAll, describe, expect, test } from "vitest";
import { screenshot, startKit, type Kit } from "./kit-browser.ts";
import {
  compare,
  diffDir,
  referenceDir,
  saveDiff,
  views,
  type Comparison,
} from "./kit-references.ts";

let kit: Kit;
beforeAll(async () => {
  rmSync(diffDir, { recursive: true, force: true });
  // The first run pulls the Chrome image.
  kit = await startKit({ pinned: true });
}, 600_000);
afterAll(async () => {
  await kit?.close();
});

const light1440 = views.find(
  ({ screen, theme, width }) =>
    screen === "kit" && theme === "light" && width === 1440,
)!;

/** A screenshot of `view` after `change` alters the page. */
async function take(
  view: (typeof views)[number],
  change?: (page: Page) => Promise<unknown>,
): Promise<Buffer> {
  const { page } = await kit.open({
    width: view.width,
    system: view.theme,
    theme: view.theme,
    screen: view.screen,
  });
  try {
    await change?.(page);
    return await screenshot(page, { window: view.window });
  } finally {
    await page.close();
  }
}

const reference = (file: string) => readFileSync(join(referenceDir, file));

/** The box of the first primary button on the kit page, the "Approve" sample. */
const approve = (page: Page) =>
  page.getByRole("button", { name: "Approve", exact: true }).first();

/** Counts the red pixels of a diff image, inside and outside `box`. */
function redPixels(
  diff: Buffer,
  box: { x: number; y: number; width: number; height: number },
) {
  const image = PNG.sync.read(diff);
  let inside = 0;
  let outside = 0;
  for (let y = 0; y < image.height; y++) {
    for (let x = 0; x < image.width; x++) {
      const i = (y * image.width + x) * 4;
      const [r, g, b] = image.data.subarray(i, i + 3);
      if (r !== 255 || g !== 0 || b !== 0) continue;
      const within =
        x >= box.x &&
        x < box.x + box.width &&
        y >= box.y &&
        y < box.y + box.height;
      if (within) inside++;
      else outside++;
    }
  }
  return { inside, outside };
}

/** Saves the screenshot and the diff image of a failed view to kit-diff/; returns the message. */
function failure(file: string, actual: Buffer, result: Comparison): string {
  const saved = saveDiff(diffDir, file, actual, result);
  const sizes = result.sizes
    ? `, ${result.sizes.actual} instead of ${result.sizes.expected}`
    : "";
  return [
    `${file} differs from its reference in ${result.pixels} pixels${sizes}.`,
    `Screenshot: web/${relative(import.meta.dirname, saved.screenshot)}; diff image: web/${relative(import.meta.dirname, saved.diff)}.`,
    "If the change is intended, update the references with `npm run kit:references` in web/.",
  ].join("\n");
}

describe.each(views)(
  "the $screen screen of the kit page in the $theme theme at $width px",
  (view) => {
    test("matches its reference screenshot", async () => {
      const actual = await take(view);
      const result = compare(actual, reference(view.file));
      expect(
        result.same,
        result.same ? "" : failure(view.file, actual, result),
      ).toBe(true);
    }, 300_000);
  },
);

describe("the comparison with the references", () => {
  test("fails on a changed color of a component, marking it in the diff image", async () => {
    let box = { x: 0, y: 0, width: 0, height: 0 };
    const actual = await take(light1440, async (page) => {
      // The accent of light theme, #3046C8, with blue 8 steps lighter.
      await approve(page).evaluate((button: HTMLElement) => {
        button.style.backgroundColor = "rgb(48, 70, 208)";
      });
      box = (await approve(page).boundingBox())!;
    });
    const result = compare(actual, reference(light1440.file));
    expect(result.same).toBe(false);
    expect(result.sizes).toBeUndefined();
    const red = redPixels(result.diff, box);
    expect(red.inside).toBeGreaterThan(box.width * box.height * 0.5);
    expect(red.outside).toBe(0);
  }, 300_000);

  test("fails on a changed spacing inside a line, marking it in the diff image", async () => {
    let box = { x: 0, y: 0, width: 0, height: 0 };
    const actual = await take(light1440, async (page) => {
      // One more pixel of padding on each side widens the button and moves the ones after it.
      await approve(page).evaluate((button: HTMLElement) => {
        const padding = parseFloat(getComputedStyle(button).paddingLeft);
        button.style.paddingInline = `${padding + 1}px`;
      });
      box = (await approve(page).boundingBox())!;
    });
    const result = compare(actual, reference(light1440.file));
    expect(result.same).toBe(false);
    expect(result.sizes).toBeUndefined();
    expect(redPixels(result.diff, box).inside).toBeGreaterThan(0);
  }, 300_000);

  test("fails on a changed spacing between components, reporting the new size", async () => {
    const actual = await take(light1440, (page) =>
      approve(page).evaluate((button: HTMLElement) => {
        button.style.marginTop = "4px";
      }),
    );
    const expected = reference(light1440.file);
    const result = compare(actual, expected);
    expect(result.same).toBe(false);
    const { width, height } = PNG.sync.read(expected);
    expect(result.sizes).toEqual({
      actual: `${width}×${height + 4}`,
      expected: `${width}×${height}`,
    });
    expect(result.pixels).toBeGreaterThan(0);
  }, 300_000);

  test("passes once the references show the change", async () => {
    const change = (page: Page) =>
      approve(page).evaluate((button: HTMLElement) => {
        button.style.backgroundColor = "rgb(48, 70, 208)";
      });
    const updated = await take(light1440, change);
    expect(compare(await take(light1440, change), updated).same).toBe(true);
  }, 300_000);

  test("saves the screenshot and the diff image of a failed view", () => {
    const image = (color: number[]) => {
      const png = new PNG({ width: 4, height: 4 });
      for (let i = 0; i < png.data.length; i += 4) png.data.set(color, i);
      return PNG.sync.write(png);
    };
    const actual = image([255, 255, 255, 255]);
    const result = compare(actual, image([0, 0, 0, 255]));
    expect(result).toMatchObject({ same: false, pixels: 16 });
    const dir = mkdtempSync(join(tmpdir(), "skald-kit-diff-"));
    try {
      const file = "kit-light-1440.png";
      expect(saveDiff(dir, file, actual, result)).toEqual({
        screenshot: join(dir, file),
        diff: join(dir, "kit-light-1440.diff.png"),
      });
      expect(readFileSync(join(dir, file))).toEqual(actual);
      expect(readFileSync(join(dir, "kit-light-1440.diff.png"))).toEqual(
        result.diff,
      );
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });
});
