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
/** Tones of the design sources beyond the README tokens: soft backgrounds and text on fills. */
const sourceTones = [
  ["Accent text", "#FFFFFF", "#0F1013"],
  ["Accent soft", "#EDF0FC", "#1C2342"],
  ["OK soft", "#E6F3EB", "#12261B"],
  ["Warning soft", "#FCF0E2", "#2B1D0F"],
  ["Danger text", "#FFFFFF", "#0F1013"],
  ["Danger soft", "#FAE9E7", "#2D1513"],
  ["Idle", "#9A9CA3", "#6B6E76"],
  ["Hover", "#ECECE8", "#202127"],
];
const fonts = [
  "Inter Variable",
  "Space Grotesk Variable",
  "JetBrains Mono Variable",
];

/** A design color in `theme`, as the browser computes it: `rgb(48, 70, 200)`. */
function color(label: string, theme: Theme): string {
  const token = tokens.find(([name]) => name === label);
  const hex = token
    ? token[theme === "light" ? 2 : 3]!
    : sourceTones.find(([name]) => name === label)![theme === "light" ? 1 : 2]!;
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

const transparent = "rgba(0, 0, 0, 0)";

test.each(themes)(
  "buttons, status labels and banners have the design colors in the %s theme",
  async (theme) => {
    const c = (label: string) => color(label, theme);
    const view = await kit.open({ width: 1440, system: theme });
    const looks = await view.page.evaluate(() => {
      const look = (element: Element) => {
        const style = getComputedStyle(element);
        return {
          background: style.backgroundColor,
          color: style.color,
          border: style.borderTopColor,
        };
      };
      const cards = [...document.querySelectorAll("#kit-buttons + ul > li")];
      const rows = [
        ...document.querySelectorAll("#kit-statuses + table tbody tr"),
      ];
      const banner = document.querySelector("#kit-banners + div [role=alert]")!;
      return {
        kinds: cards.slice(0, 4).map((card) => {
          const [normal, disabled] = card.querySelectorAll("button");
          return {
            ...look(normal!),
            opacity: [
              getComputedStyle(normal!).opacity,
              getComputedStyle(disabled!).opacity,
            ],
          };
        }),
        heights: [...cards[4]!.querySelectorAll("button")].map(
          (button) => button.getBoundingClientRect().height,
        ),
        tinted: {
          card: look(cards[6]!),
          buttons: [...cards[6]!.querySelectorAll("button")].map(look),
        },
        tones: rows.map((row) => {
          const [label, dot, count] = [...row.querySelectorAll("td > span")];
          return {
            tone: row.querySelector("th")!.textContent,
            label: look(label!),
            dot: [
              look(dot!).color,
              look(dot!.querySelector("[aria-hidden]")!).background,
            ],
            count: look(count!),
          };
        }),
        banner: look(banner),
        action: look(banner.querySelector("button")!),
      };
    });

    expect(looks.kinds).toEqual(
      [
        {
          background: c("Accent"),
          color: c("Accent text"),
          border: c("Accent"),
        },
        { background: c("Background"), color: c("Text"), border: c("Line") },
        { background: transparent, color: c("Danger"), border: c("Line") },
        {
          background: c("Danger"),
          color: c("Danger text"),
          border: c("Danger"),
        },
      ].map((kind) => ({ ...kind, opacity: ["1", "0.5"] })),
    );
    expect(looks.heights).toEqual([44, 40, 36]);
    // As the join request in 08-machines.png: the secondary kind keeps the page background.
    expect(looks.tinted).toEqual({
      card: {
        background: c("Accent soft"),
        color: c("Text"),
        border: c("Accent"),
      },
      buttons: [
        {
          background: c("Accent"),
          color: c("Accent text"),
          border: c("Accent"),
        },
        { background: c("Background"), color: c("Text"), border: c("Line") },
      ],
    });
    expect(looks.tones).toEqual(
      [
        ["OK", "OK", "OK soft", "OK"],
        ["Warning", "Warning", "Warning soft", "Warning"],
        ["Danger", "Danger", "Danger soft", "Danger"],
        ["Accent", "Accent", "Accent soft", "Accent"],
        ["Muted", "Muted text", "Hover", "Idle"],
      ].map(([tone, fg, soft, mark]) => ({
        tone,
        label: { background: c(soft!), color: c(fg!), border: c(fg!) },
        dot: [c(fg!), c(mark!)],
        count: { background: c(soft!), color: c(fg!), border: c(fg!) },
      })),
    );
    expect(looks.banner).toEqual({
      background: c("Warning soft"),
      color: c("Warning"),
      border: c("Warning"),
    });
    expect(looks.action).toEqual({
      background: transparent,
      color: c("Warning"),
      border: c("Warning"),
    });
    await view.page.close();
  },
);

test.each(themes)(
  "tables and their states have the design looks in the %s theme",
  async (theme) => {
    const c = (label: string) => color(label, theme);
    const view = await kit.open({ width: 1440, system: theme });
    const looks = await view.page.evaluate(() => {
      const table = (name: string) =>
        document.querySelector(`table[aria-label="${name}"]`)!;
      const style = (element: Element) => getComputedStyle(element);
      const body = (name: string) => [
        ...table(name).querySelectorAll("tbody tr"),
      ];
      const cell = (name: string, row: number, column: number) =>
        body(name)[row]!.children[column]!;
      const text = (element: Element) => ({
        color: style(element).color,
        weight: style(element).fontWeight,
      });
      const [device, , seen, key] = [
        ...table("Devices").querySelectorAll("th"),
      ];
      const lines = (element: Element) =>
        [...element.querySelectorAll("span span")].map((line) => ({
          font: style(line).fontFamily.split(",")[0]!.replaceAll('"', ""),
          size: style(line).fontSize,
          ...text(line),
        }));
      const bars = [...document.querySelectorAll("#kit-tables ~ ul > li")]
        .filter((card) => card.querySelector("[aria-busy]"))
        .map((card) => ({
          card: style(card).backgroundColor,
          bars: [...card.querySelectorAll("[aria-busy] > [aria-hidden]")].map(
            (bar) => style(bar).backgroundColor,
          ),
        }));
      return {
        headers: {
          active: [
            text(seen!.querySelector("button")!).color,
            style(seen!.querySelector("button span")!).opacity,
          ],
          idle: [
            text(device!.querySelector("button")!).color,
            style(device!.querySelector("button span")!).opacity,
          ],
          plain: text(key!).color,
        },
        device: lines(cell("Devices", 1, 0)),
        kind: text(cell("Devices", 0, 1)),
        keys: [lines(cell("Devices", 0, 3)), lines(cell("Devices", 3, 3))],
        queue: style(
          cell("All queues", 0, 0).firstElementChild!,
        ).fontFamily.split(",")[0],
        rows: body("All queues").map((row) => style(row).backgroundColor),
        dead: [text(cell("All queues", 0, 3)), text(cell("All queues", 1, 3))],
        messages: [text(cell("Accounts", 0, 4)), text(cell("Accounts", 1, 4))],
        actions: [
          cell("Devices", 0, 4).querySelector("button")!.getBoundingClientRect()
            .width,
          cell("Accounts", 0, 7)
            .querySelector("button")!
            .getBoundingClientRect().width,
        ],
        // Every row of a table, the header included, puts its columns at the same places.
        aligned: ["Devices", "Accounts", "All queues"].map((name) => {
          const lefts = [...table(name).querySelectorAll("tr")].map((row) =>
            [...row.children].map((child) =>
              Math.round(child.getBoundingClientRect().left),
            ),
          );
          return lefts.every((row) => row.join() === lefts[0]!.join());
        }),
        bars,
      };
    });

    expect(looks.headers).toEqual({
      active: [c("Accent"), "1"],
      idle: [c("Muted text"), "0.5"],
      plain: c("Muted text"),
    });
    const mono = "JetBrains Mono Variable";
    const inter = "Inter Variable";
    expect(looks.device).toEqual([
      { font: inter, size: "14px", color: c("Text"), weight: "600" },
      { font: inter, size: "12px", color: c("Muted text"), weight: "400" },
    ]);
    expect(looks.kind).toEqual({ color: c("Muted text"), weight: "400" });
    expect(looks.keys).toEqual(
      ["Muted text", "Warning"].map((detail) => [
        { font: mono, size: "13px", color: c("Text"), weight: "400" },
        { font: inter, size: "12px", color: c(detail), weight: "400" },
      ]),
    );
    expect(looks.queue).toBe(`"${mono}"`);
    // As 07-queues.png: the queue with dead letters is highlighted.
    expect(looks.rows).toEqual([c("Danger soft"), transparent, transparent]);
    expect(looks.dead).toEqual([
      { color: c("Danger"), weight: "600" },
      { color: c("Text"), weight: "400" },
    ]);
    expect(looks.messages).toEqual([
      { color: c("Text"), weight: "400" },
      { color: c("Warning"), weight: "600" },
    ]);
    // The buttons fill the actions column: 96 px in Machines, 120 px in operator accounts.
    expect(looks.actions).toEqual([96, 120]);
    expect(looks.aligned).toEqual([true, true, true]);
    // Loading lines: the muted color, see-through, so they show on white and tinted cards alike.
    const muted = c("Muted text").match(/\d+/g)!.map(Number);
    expect(looks.bars.map(({ card }) => card)).toEqual([
      transparent,
      c("Accent soft"),
    ]);
    for (const { bars } of looks.bars) {
      expect(bars).toHaveLength(2);
      for (const bar of bars) {
        const [r, g, b, alpha] =
          /color\(srgb ([\d.]+) ([\d.]+) ([\d.]+) \/ ([\d.]+)\)/
            .exec(bar)!
            .slice(1)
            .map(Number);
        expect([r, g, b].map((channel) => Math.round(channel! * 255))).toEqual(
          muted,
        );
        expect(alpha).toBe(0.18);
      }
    }
    await view.page.close();
  },
);

const tableNames = [
  "Devices",
  "Accounts",
  "All queues",
  "Devices, empty",
  "Devices, loading",
];

/** Each table frame in the tables section: whether it scrolls and how it takes the focus. */
function tableFrames(page: Page) {
  return page.evaluate(() =>
    [...document.querySelectorAll("#kit-tables ~ div table")].map((table) => {
      const frame = table.parentElement!;
      return {
        name: table.getAttribute("aria-label"),
        scrolls: frame.scrollWidth > frame.clientWidth,
        tabIndex: frame.tabIndex,
        role: frame.getAttribute("role"),
        label: frame.getAttribute("aria-label"),
      };
    }),
  );
}

/** The table frames as they should be at `width`: on a phone they scroll and take the focus. */
function expectedFrames(width: number) {
  const phone = width < 760;
  return tableNames.map((name) => ({
    name,
    scrolls: phone,
    tabIndex: phone ? 0 : -1,
    role: phone ? "region" : null,
    label: phone ? name : null,
  }));
}

/**
 * Whether the empty and loading states are inside the visible part of their frames and centered
 * in it, with the empty frame scrolled sideways.
 */
async function stateBoxes(page: Page) {
  const frame = page
    .getByRole("table", { name: "Devices, empty" })
    .locator("..");
  await frame.evaluate((element) => element.scrollTo({ left: 200 }));
  return page.evaluate(() =>
    ["Devices, empty", "Devices, loading"].map((name) => {
      const table = document.querySelector(`table[aria-label="${name}"]`)!;
      const frame = table.parentElement!.getBoundingClientRect();
      const state = table
        .querySelector("tbody td > *")!
        .getBoundingClientRect();
      return {
        inside: state.left >= frame.left && state.right <= frame.right,
        centered:
          Math.abs(state.left - frame.left - (frame.right - state.right)) <= 2,
      };
    }),
  );
}

const statesInView = [
  { inside: true, centered: true },
  { inside: true, centered: true },
];

describe.each(widths)("tables at %i px", (width) => {
  let view: KitView;
  beforeAll(async () => {
    view = await kit.open({ width, system: "light" });
  }, 30_000);
  afterAll(() => view?.page.close());

  test(
    width < 760
      ? "scroll inside their frames, which take the focus"
      : "fit their frames and take no focus",
    async () => {
      expect(await tableFrames(view.page)).toEqual(expectedFrames(width));
    },
  );

  test("keep the empty and loading states in the visible part of the frame", async () => {
    expect(await stateBoxes(view.page)).toEqual(statesInView);
  });
});

describe.each(widths.map((from, i) => [from, widths[1 - i]!]))(
  "tables opened at %i px and resized to %i px",
  (from, to) => {
    let view: KitView;
    beforeAll(async () => {
      view = await kit.open({ width: from, system: "light" });
      await view.page.setViewportSize({ width: to, height: 900 });
    }, 30_000);
    afterAll(() => view?.page.close());

    test(
      to < 760
        ? "start to scroll inside their frames, which take the focus"
        : "stop scrolling and their frames give up the focus",
      async () => {
        await expect
          .poll(() => tableFrames(view.page))
          .toEqual(expectedFrames(to));
      },
    );

    test("keep the empty and loading states in the visible part of the frame", async () => {
      await expect.poll(() => stateBoxes(view.page)).toEqual(statesInView);
    });
  },
);

test("Chrome exposes the tables with their column headers and cells", async () => {
  const view = await kit.open({ width: 1440, system: "light" });
  const cdp = await view.page.context().newCDPSession(view.page);
  const { nodes } = await cdp.send("Accessibility.getFullAXTree");
  const byId = new Map(nodes.map((node) => [node.nodeId, node]));
  /** The roles and names under `node`, skipping ignored nodes, down to cells. */
  const tree = (node: (typeof nodes)[number]): unknown[] =>
    (node.childIds ?? []).flatMap((id) => {
      const child = byId.get(id)!;
      if (child.ignored) return tree(child);
      const role = child.role?.value as string;
      if (role === "columnheader" || role === "cell")
        return [`${role} ${child.name?.value}`];
      if (role === "row" || role === "rowgroup")
        return [{ [role]: tree(child) }];
      return [];
    });
  const table = nodes.find(
    (node) => node.role?.value === "table" && node.name?.value === "All queues",
  )!;
  expect(tree(table)).toEqual([
    {
      rowgroup: [
        {
          row: [
            "Queue",
            "Waiting",
            "Running",
            "Dead letters",
            "Oldest waiting",
          ].map((name) => `columnheader ${name}`),
        },
      ],
    },
    ...[
      ["deploy", "0", "0", "2", "—"],
      ["research", "1", "2", "0", "40 s"],
      ["review", "3", "1", "0", "2 min"],
    ].map((row) => ({ row: row.map((name) => `cell ${name}`) })),
  ]);
  await view.page.close();
});

/** The filters section of the kit page. */
const filters = "section[aria-labelledby=kit-filters]";

test.each(themes)(
  "filter buttons and search have the design looks in the %s theme",
  async (theme) => {
    const c = (label: string) => color(label, theme);
    const view = await kit.open({ width: 1440, system: theme });
    const looks = await view.page.evaluate((filters) => {
      const section = document.querySelector(filters)!;
      const look = (element: Element) => {
        const style = getComputedStyle(element);
        return {
          background: style.backgroundColor,
          color: style.color,
          border: `${style.borderTopWidth} ${style.borderTopColor}`,
          font: `${style.fontFamily.split(",")[0]!.replaceAll('"', "")} ${style.fontSize} ${style.fontWeight}`,
          height: element.getBoundingClientRect().height,
        };
      };
      const group = (name: string) =>
        section.querySelector(`[role=group][aria-label="${name}"]`)!;
      const field = (name: string) =>
        section.querySelector(`input[aria-label="${name}"]`)!;
      const gaps = (element: Element) => {
        const boxes = [...element.children].map((child) =>
          child.getBoundingClientRect(),
        );
        return boxes.slice(1).map((box, i) => box.left - boxes[i]!.right);
      };
      const joined = group("View");
      const clear = section.querySelector("button[aria-label='Clear search']")!;
      const members = field("Search members").getBoundingClientRect();
      const cross = clear.getBoundingClientRect();
      return {
        separate: [...group("Filter accounts").children].map(look),
        separateGaps: gaps(group("Filter accounts")),
        joined: {
          frame: [
            look(joined).border,
            getComputedStyle(joined).borderTopLeftRadius,
          ],
          buttons: [...joined.children].map(look),
          gaps: gaps(joined),
        },
        searches: ["Search accounts", "Search messages", "Search members"].map(
          (name) => ({
            ...look(field(name)),
            width: field(name).getBoundingClientRect().width,
            placeholder: getComputedStyle(field(name), "::placeholder").color,
          }),
        ),
        clear: {
          color: getComputedStyle(clear).color,
          background: getComputedStyle(clear).backgroundColor,
          // The cross sits inside the field at its right end, centered on its height.
          right: members.right - cross.right,
          centered:
            Math.abs(
              cross.top - members.top - (members.bottom - cross.bottom),
            ) <= 1,
          padding: getComputedStyle(field("Search members")).paddingRight,
        },
      };
    }, filters);

    const inter = "Inter Variable";
    const active = {
      background: c("Accent soft"),
      color: c("Accent"),
      border: `1px ${c("Accent soft")}`,
      font: `${inter} 13px 500`,
      height: 40,
    };
    const idle = {
      background: transparent,
      color: c("Muted text"),
      border: `1px ${c("Line")}`,
      font: `${inter} 13px 500`,
      height: 40,
    };
    // As 13-operator-accounts.png: the first choice is active.
    expect(looks.separate).toEqual([active, idle, idle, idle]);
    expect(looks.separateGaps).toEqual([4, 4, 4]);
    // As 04-messages.png: one frame, the active choice bold.
    expect(looks.joined).toEqual({
      frame: [`1px ${c("Line")}`, "8px"],
      buttons: [
        {
          ...active,
          border: `0px ${c("Accent")}`,
          font: `${inter} 13px 600`,
        },
        { ...idle, border: `0px ${c("Muted text")}` },
      ],
      gaps: [0],
    });
    expect(looks.searches).toEqual(
      [280, 240, 260].map((width) => ({
        background: c("Side panel"),
        color: c("Text"),
        border: `1px ${c("Line")}`,
        font: `${inter} 14px 400`,
        height: 40,
        width,
        placeholder: c("Muted text"),
      })),
    );
    expect(looks.clear).toEqual({
      color: c("Muted text"),
      background: transparent,
      right: 6,
      centered: true,
      padding: "40px",
    });
    await view.page.close();
  },
);

/** The colors of the pixels in a box of a PNG image, as Chrome draws them: `rgb(15, 16, 19)`. */
async function boxColors(
  png: Buffer,
  box: { x: number; y: number; width: number; height: number },
): Promise<string[]> {
  const page = await kit.browser.newPage();
  try {
    return await page.evaluate(
      async ({ src, box }) => {
        const image = new Image();
        image.src = src;
        await image.decode();
        const canvas = document.createElement("canvas");
        canvas.width = image.width;
        canvas.height = image.height;
        const context = canvas.getContext("2d")!;
        context.drawImage(image, 0, 0);
        const { data } = context.getImageData(
          box.x,
          box.y,
          box.width,
          box.height,
        );
        const colors = new Set<string>();
        for (let i = 0; i < data.length; i += 4)
          colors.add(`rgb(${data[i]}, ${data[i + 1]}, ${data[i + 2]})`);
        return [...colors];
      },
      { src: `data:image/png;base64,${png.toString("base64")}`, box },
    );
  } finally {
    await page.close();
  }
}

test("a search field with a query shows its own clear button, not Chrome's", async () => {
  const view = await kit.open({ width: 1440, system: "light", theme: "light" });
  const field = view.page.getByRole("searchbox", { name: "Search members" });
  await field.focus();
  const { width, height } = (await field.boundingBox())!;
  const png = await field.screenshot({ scale: "css" });
  // Between the text and the clear button: only the field background, where Chrome puts its own.
  const strip = { x: width - 70, y: 4, width: 28, height: height - 8 };
  expect(await boxColors(png, strip)).toEqual([color("Side panel", "light")]);
  // The clear button draws its cross.
  const cross = { x: width - 34, y: 6, width: 28, height: height - 12 };
  expect((await boxColors(png, cross)).length).toBeGreaterThan(1);
  await view.page.close();
});

describe("filter buttons and search in Chrome", () => {
  let view: KitView;
  beforeAll(async () => {
    view = await kit.open({ width: 1440, system: "light" });
  }, 30_000);
  afterAll(() => view?.page.close());

  const section = () => view.page.locator(filters);
  const search = () =>
    section().getByRole("searchbox", { name: "Search accounts" });
  /** The account names in the filtered table, or the empty state. */
  const shown = () =>
    section()
      .getByRole("table", { name: "Filtered accounts" })
      .locator("tbody tr")
      .evaluateAll((rows) =>
        rows.map(
          (row) => (row.querySelector("td span span") ?? row).textContent ?? "",
        ),
      );
  /** Each choice of the accounts filter with whether it is pressed. */
  const choices = () =>
    section()
      .getByRole("group", { name: "Filter accounts" })
      .getByRole("button")
      .evaluateAll((buttons) =>
        buttons.map((button) => [
          button.textContent,
          button.getAttribute("aria-pressed"),
        ]),
      );
  const focused = () =>
    view.page.evaluate(
      () =>
        document.activeElement?.getAttribute("aria-label") ??
        document.activeElement?.textContent,
    );
  const everyone = [
    "Northwind",
    "spam-relay",
    "Acme CI",
    "Ana Kovac",
    "Lee Park",
    "free-tokens",
  ];

  test("Tab, Space and Enter make a filter choice the only active one", async () => {
    await section().getByRole("button", { name: "All", exact: true }).focus();
    await view.page.keyboard.press("Tab");
    expect(await focused()).toBe("Flagged");
    await view.page.keyboard.press("Space");
    expect(await choices()).toEqual([
      ["All", "false"],
      ["Flagged", "true"],
      ["Banned", "false"],
      ["Enterprise", "false"],
    ]);
    expect(await shown()).toEqual(["spam-relay"]);
    expect(await focused()).toBe("Flagged");

    await view.page.keyboard.press("Tab");
    await view.page.keyboard.press("Tab");
    expect(await focused()).toBe("Enterprise");
    await view.page.keyboard.press("Enter");
    expect(await choices()).toEqual([
      ["All", "false"],
      ["Flagged", "false"],
      ["Banned", "false"],
      ["Enterprise", "true"],
    ]);
    expect(await shown()).toEqual(["Northwind"]);

    await view.page.keyboard.press("Shift+Tab");
    await view.page.keyboard.press("Shift+Tab");
    await view.page.keyboard.press("Shift+Tab");
    await view.page.keyboard.press("Enter");
    expect(await shown()).toEqual(everyone);
  });

  test("typing narrows the table and Escape clears the query, keeping the focus", async () => {
    await search().click();
    await view.page.keyboard.type("park");
    expect(await shown()).toEqual(["Lee Park"]);
    await view.page.keyboard.press("Escape");
    expect(await search().inputValue()).toBe("");
    expect(await shown()).toEqual(everyone);
    expect(await focused()).toBe("Search accounts");
  });

  test("the clear button clears the query by a click or Enter and returns the focus to the field", async () => {
    // The field's own clear button, next to it.
    const clear = search().locator("xpath=following-sibling::button");
    await search().click();
    await view.page.keyboard.type("ana");
    expect(await shown()).toEqual(["Ana Kovac"]);
    await clear.click();
    expect(await search().inputValue()).toBe("");
    expect(await shown()).toEqual(everyone);
    expect(await focused()).toBe("Search accounts");
    expect(await clear.count()).toBe(0);

    await view.page.keyboard.type("north");
    await view.page.keyboard.press("Tab");
    expect(await focused()).toBe("Clear search");
    await view.page.keyboard.press("Enter");
    expect(await search().inputValue()).toBe("");
    expect(await focused()).toBe("Search accounts");
  });

  test("Chrome exposes the choices as pressed or not and the named search field", async () => {
    await section().getByRole("button", { name: "Banned" }).click();
    const cdp = await view.page.context().newCDPSession(view.page);
    const { nodes } = await cdp.send("Accessibility.getFullAXTree");
    const byId = new Map(nodes.map((node) => [node.nodeId, node]));
    const group = nodes.find(
      (node) =>
        node.role?.value === "group" && node.name?.value === "Filter accounts",
    )!;
    const buttons = (group.childIds ?? [])
      .map((id) => byId.get(id)!)
      .map((node) => [
        node.role?.value,
        node.name?.value,
        node.properties?.find(({ name }) => name === "pressed")?.value.value,
      ]);
    expect(buttons).toEqual([
      ["button", "All", "false"],
      ["button", "Flagged", "false"],
      ["button", "Banned", "true"],
      ["button", "Enterprise", "false"],
    ]);
    expect(
      nodes
        .filter((node) => node.role?.value === "searchbox")
        .map((node) => node.name?.value),
    ).toEqual(["Search accounts", "Search messages", "Search members"]);
    await section().getByRole("button", { name: "All", exact: true }).click();
  });
});

test("in a container narrower than them, filter buttons wrap and the search field shrinks", async () => {
  const view = await kit.open({ width: 1440, system: "light" });
  const layout = await view.page.evaluate((filters) => {
    const section = document.querySelector(filters)!;
    const group = section.querySelector(
      "[role=group][aria-label='Filter accounts']",
    )!;
    const search = section.querySelector(
      "input[aria-label='Search accounts']",
    )!;
    const toolbar = group.parentElement! as HTMLElement;
    toolbar.style.width = "180px";
    const right = toolbar.getBoundingClientRect().right;
    const buttons = [...group.children].map((button) =>
      button.getBoundingClientRect(),
    );
    return {
      lines: new Set(buttons.map((button) => button.top)).size,
      inside: [...buttons, search.getBoundingClientRect()].every(
        (box) => box.right <= right,
      ),
      searchWidth: search.getBoundingClientRect().width,
    };
  }, filters);
  expect(layout).toEqual({ lines: 2, inside: true, searchWidth: 180 });
  await view.page.close();
});

/** Where the accounts filter and search sit: on one line or two, and inside the page. */
function toolbarLayout(page: Page) {
  return page.evaluate((filters) => {
    const section = document.querySelector(filters)!;
    const group = section
      .querySelector("[role=group][aria-label='Filter accounts']")!
      .getBoundingClientRect();
    const search = section
      .querySelector("input[aria-label='Search accounts']")!
      .getBoundingClientRect();
    const page = document.querySelector("main")!.getBoundingClientRect();
    return {
      oneLine: search.top === group.top,
      inside:
        Math.max(group.right, search.right) <= page.right - 16 &&
        Math.min(group.left, search.left) >= page.left + 16,
      searchWidth: search.width,
    };
  }, filters);
}

const toolbar = (width: number) => ({
  oneLine: width >= 760,
  inside: true,
  searchWidth: 280,
});

describe.each(widths)("filter buttons and search at %i px", (width) => {
  test(
    width < 760 ? "wrap and stay inside the page" : "sit on one line",
    async () => {
      const view = await kit.open({ width, system: "light" });
      expect(await toolbarLayout(view.page)).toEqual(toolbar(width));
      await view.page.close();
    },
  );
});

describe.each(widths.map((from, i) => [from, widths[1 - i]!]))(
  "filter buttons and search opened at %i px and resized to %i px",
  (from, to) => {
    test(
      to < 760 ? "wrap and stay inside the page" : "join one line",
      async () => {
        const view = await kit.open({ width: from, system: "light" });
        await view.page.setViewportSize({ width: to, height: 900 });
        await expect.poll(() => toolbarLayout(view.page)).toEqual(toolbar(to));
        await view.page.close();
      },
    );
  },
);

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

/** The top left pixel of a PNG image, as Chrome draws it: `rgb(15, 16, 19)`. */
async function topLeftPixel(png: Buffer): Promise<string> {
  const page = await kit.browser.newPage();
  try {
    return await page.evaluate(
      async (src) => {
        const image = new Image();
        image.src = src;
        await image.decode();
        const context = document.createElement("canvas").getContext("2d")!;
        context.drawImage(image, 0, 0);
        const [r, g, b] = context.getImageData(0, 0, 1, 1).data;
        return `rgb(${r}, ${g}, ${b})`;
      },
      `data:image/png;base64,${png.toString("base64")}`,
    );
  } finally {
    await page.close();
  }
}

test("the screenshot command saves the kit page in both themes at both widths", async () => {
  const dir = mkdtempSync(join(tmpdir(), "skald-kit-"));
  try {
    await promisify(execFile)(process.execPath, ["kit-screenshots.ts", dir], {
      cwd: import.meta.dirname,
    });
    const expected = themes.flatMap((theme) =>
      widths.map((width) => ({
        file: `kit-${theme}-${width}.png`,
        theme,
        width,
      })),
    );
    expect(readdirSync(dir).sort()).toEqual(
      expected.map(({ file }) => file).sort(),
    );
    for (const { file, theme, width } of expected) {
      // A PNG starts with an 8-byte signature and the IHDR chunk: width at bytes 16–19.
      const png = readFileSync(join(dir, file));
      expect(png.subarray(1, 4).toString()).toBe("PNG");
      expect(png.readUInt32BE(16), file).toBe(width);
      // The top left corner is the page background.
      expect(await topLeftPixel(png), file).toBe(color("Background", theme));
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
