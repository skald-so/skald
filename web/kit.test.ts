// @vitest-environment node
// The kit page in headless Chrome, the screenshot command and the production build.
import { execFile } from "node:child_process";
import { mkdtempSync, readdirSync, readFileSync, rmSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { promisify } from "node:util";
import type { Page } from "playwright-core";
import { build, type Rolldown } from "vite";
import { afterAll, beforeAll, describe, expect, test } from "vitest";
import {
  startKit,
  themes,
  widths,
  type Kit,
  type KitView,
  type Theme,
} from "./kit-browser.ts";

/** "Tokens" in the design README, with the CSS custom property of each. */
const tokens = [
  ["Background", "--color-bg", "#FFFFFF", "#0F1013"],
  ["Side panel", "--color-side", "#F6F6F4", "#15161A"],
  ["Line", "--color-line", "#E4E4E0", "#2A2C33"],
  ["Text", "--color-text", "#17181B", "#ECEDEF"],
  ["Muted text", "--color-muted", "#5C5F66", "#A2A5AE"],
  ["Accent", "--color-accent", "#3046C8", "#93A3FF"],
  ["OK", "--color-ok", "#1D7A4C", "#5BC991"],
  ["Warning", "--color-warning", "#94470A", "#F2A65A"],
  ["Danger", "--color-danger", "#B3312A", "#F28B80"],
];
const fonts = [
  "Inter Variable",
  "Space Grotesk Variable",
  "JetBrains Mono Variable",
];

/** A design token's color in `theme`, as the browser computes it: `rgb(48, 70, 200)`. */
function color(label: string, theme: Theme): string {
  const token = tokens.find(([name]) => name === label)!;
  const hex = token[theme === "light" ? 2 : 3]!;
  const [r, g, b] = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16));
  return `rgb(${r}, ${g}, ${b})`;
}

/** Path prefixes of the API, as the dev server proxies them. */
const api = Object.keys(
  JSON.parse(
    readFileSync(new URL("../api/openapi.json", import.meta.url), "utf8"),
  ).paths,
).map((path) => `/${path.split("/")[1]}`);

const other = (theme: Theme): Theme => (theme === "light" ? "dark" : "light");

let kit: Kit;
beforeAll(async () => {
  kit = await startKit();
}, 60_000);
afterAll(() => kit?.close());

async function pageColors(page: Page) {
  return page.evaluate(() => {
    const style = getComputedStyle(document.body);
    return { background: style.backgroundColor, text: style.color };
  });
}

test("the kit page shows every color token of the design in both themes", async () => {
  const view = await kit.open({ width: 1440, system: "light" });
  const cards = await view.page
    .getByRole("region", { name: "Colors" })
    .getByRole("listitem")
    .allTextContents();
  expect(cards).toEqual(
    tokens.map(
      ([label, token, light, dark]) =>
        `${label}${token}Light${light}Dark${dark}`,
    ),
  );
  await view.page.close();
});

test.each(themes)(
  "without the URL parameter the kit page follows the %s system theme",
  async (theme) => {
    const view = await kit.open({ width: 1440, system: theme });
    expect(await pageColors(view.page)).toEqual({
      background: color("Background", theme),
      text: color("Text", theme),
    });
    await view.page.close();
  },
);

describe.each(
  themes.flatMap((theme) => widths.map((width) => ({ theme, width }))),
)("the kit page forced to $theme at $width px", ({ theme, width }) => {
  let view: KitView;
  beforeAll(async () => {
    // The system has the other theme: the URL parameter overrides it.
    view = await kit.open({ width, system: other(theme), theme });
  }, 30_000);
  afterAll(() => view?.page.close());

  test("has the theme colors", async () => {
    expect(await pageColors(view.page)).toEqual({
      background: color("Background", theme),
      text: color("Text", theme),
    });
  });

  test("does not scroll sideways", async () => {
    const overflow = await view.page.evaluate(
      () =>
        document.documentElement.scrollWidth -
        document.documentElement.clientWidth,
    );
    expect(overflow).toBe(0);
  });

  test("loads the bundled fonts and calls neither the API nor other hosts", async () => {
    const loaded = await view.page.evaluate(() =>
      [...document.fonts]
        .filter((font) => font.status === "loaded")
        .map((font) => font.family.replaceAll('"', "")),
    );
    expect(new Set(loaded)).toEqual(new Set(fonts));

    const urls = view.requests.map((url) => new URL(url));
    expect(urls.length).toBeGreaterThan(0);
    expect(urls.filter((url) => url.origin !== kit.origin)).toEqual([]);
    expect(
      urls.filter(({ pathname }) =>
        api.some(
          (prefix) => pathname === prefix || pathname.startsWith(`${prefix}/`),
        ),
      ),
    ).toEqual([]);
  });

  test("passes the accessibility check", async () => {
    await view.page.addScriptTag({
      path: createRequire(import.meta.url).resolve("axe-core"),
    });
    const violations = await view.page.evaluate(async () => {
      const { axe } = window as unknown as {
        axe: typeof import("axe-core");
      };
      const results = await axe.run(document);
      return results.violations.map((violation) => ({
        rule: violation.id,
        nodes: violation.nodes.map((node) => node.target.join(" ")),
      }));
    });
    expect(violations).toEqual([]);
  });

  test("outlines every element focused from the keyboard", async () => {
    const focused: { element: string; outline: string }[] = [];
    for (let i = 0; i < 500; i++) {
      await view.page.keyboard.press("Tab");
      const focus = await view.page.evaluate(() => {
        const element = document.activeElement;
        if (!element || element === document.body) return null;
        if (element.hasAttribute("data-kit-focused")) return null;
        element.setAttribute("data-kit-focused", "");
        const style = getComputedStyle(element);
        return {
          element: element.outerHTML.slice(0, 100),
          outline: `${style.outlineStyle} ${style.outlineWidth} ${style.outlineColor}`,
        };
      });
      if (!focus) break;
      focused.push(focus);
    }
    expect(focused.length).toBeGreaterThan(0);
    expect(focused).toEqual(
      focused.map(({ element }) => ({
        element,
        outline: `solid 2px ${color("Accent", theme)}`,
      })),
    );
  });
});

test("the screenshot command saves the kit page in both themes at both widths", async () => {
  const dir = mkdtempSync(join(tmpdir(), "skald-kit-"));
  try {
    await promisify(execFile)(process.execPath, ["kit-screenshots.ts", dir], {
      cwd: import.meta.dirname,
    });
    const expected = themes.flatMap((theme) =>
      widths.map((width) => ({ file: `kit-${theme}-${width}.png`, width })),
    );
    expect(readdirSync(dir).sort()).toEqual(
      expected.map(({ file }) => file).sort(),
    );
    for (const { file, width } of expected) {
      // A PNG starts with an 8-byte signature and the IHDR chunk: width at bytes 16–19.
      const png = readFileSync(join(dir, file));
      expect(png.subarray(1, 4).toString()).toBe("PNG");
      expect(png.readUInt32BE(16), file).toBe(width);
    }
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}, 60_000);

describe("the production build", () => {
  let output: Rolldown.RolldownOutput["output"];
  let css: string;
  beforeAll(async () => {
    ({ output } = (await build({
      root: import.meta.dirname,
      logLevel: "silent",
      build: { write: false },
    })) as Rolldown.RolldownOutput);
    css = output
      .map((file) =>
        file.type === "asset" && file.fileName.endsWith(".css")
          ? String(file.source)
          : "",
      )
      .join("\n");
  }, 60_000);

  test("has the app and leaves out the kit page", () => {
    const files = output.map((file) => file.fileName);
    expect(files).toContain("index.html");
    expect(files).not.toContain("kit.html");
    const modules = output.flatMap((file) =>
      file.type === "chunk" ? file.moduleIds : [],
    );
    expect(modules.some((id) => id.endsWith("/src/App.tsx"))).toBe(true);
    expect(modules.filter((id) => id.includes("/src/kit/"))).toEqual([]);
  });

  test("bundles the fonts and refers to no other host", () => {
    const files = output.map((file) => file.fileName);
    for (const font of ["inter", "space-grotesk", "jetbrains-mono"]) {
      expect(
        files.some((file) => file.includes(font) && file.endsWith(".woff2")),
        font,
      ).toBe(true);
    }
    for (const font of fonts) expect(css).toContain(font);
    expect(css).not.toMatch(/url\(\s*["']?(https?:)?\/\//);
  });

  test.each(themes)(
    "has the %s theme colors per the system setting or data-theme",
    async (theme) => {
      const cases: [system: Theme, forced?: Theme][] = [
        [theme],
        [other(theme), theme],
      ];
      for (const [system, forced] of cases) {
        const page = await kit.browser.newPage({ colorScheme: system });
        await page.setContent(
          `<html${forced ? ` data-theme="${forced}"` : ""}><head><style>${css}</style></head><body>Skald</body></html>`,
        );
        expect(await pageColors(page)).toEqual({
          background: color("Background", theme),
          text: color("Text", theme),
        });
        await page.close();
      }
    },
  );
});
