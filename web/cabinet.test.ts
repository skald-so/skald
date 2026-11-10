// @vitest-environment node
// The cabinet screen of the kit page in headless Chrome: its looks, the side menu at phone width,
// resizing, focus and what a screen reader gets.
import type { Page } from "playwright-core";
import { afterAll, afterEach, beforeAll, describe, expect, test } from "vitest";
import {
  startKit,
  themes,
  type Kit,
  type KitView,
  type Screen,
  type Theme,
} from "./kit-browser.ts";

/** Colors of the design, as Chrome computes them. */
const palette: Record<string, [light: string, dark: string]> = {
  Background: ["#FFFFFF", "#0F1013"],
  "Side panel": ["#F6F6F4", "#15161A"],
  Line: ["#E4E4E0", "#2A2C33"],
  Text: ["#17181B", "#ECEDEF"],
  "Muted text": ["#5C5F66", "#A2A5AE"],
  Accent: ["#3046C8", "#93A3FF"],
  "Accent soft": ["#EDF0FC", "#1C2342"],
  Warning: ["#94470A", "#F2A65A"],
  "Warning soft": ["#FCF0E2", "#2B1D0F"],
  Hover: ["#ECECE8", "#202127"],
};

function color(label: string, theme: Theme): string {
  const hex = palette[label]![theme === "light" ? 0 : 1];
  const [r, g, b] = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16));
  return `rgb(${r}, ${g}, ${b})`;
}

const transparent = "rgba(0, 0, 0, 0)";
const phone = 360;
const desktop = 1440;

let kit: Kit;
beforeAll(async () => {
  kit = await startKit();
}, 60_000);
// Under load Chrome can take over 10 s to quit.
afterAll(() => kit?.close(), 60_000);

let view: KitView | undefined;
afterEach(async () => {
  await view?.page.close();
  view = undefined;
});

async function open(
  width: number,
  screen: Screen = "cabinet",
  theme: Theme = "light",
) {
  view = await kit.open({ width, system: theme, screen });
  return view.page;
}

const menuButton = (page: Page) => page.getByRole("button", { name: "Menu" });
const sections = (page: Page) =>
  page.getByRole("navigation", { name: "Sections" });

/** The side menu as shown: the panel box, the page box, and whether the page is out of reach. */
function layout(page: Page) {
  return page.evaluate(() => {
    const box = (element: Element | null) => {
      if (!element) return null;
      const { left, top, width, height } = element.getBoundingClientRect();
      if (width === 0 && height === 0) return null;
      return { left, top, width, height };
    };
    const nav = document.querySelector("nav[aria-label=Sections]")!;
    const panel = nav.parentElement!;
    const main = document.querySelector("main")!;
    const button = document.querySelector("button[aria-label=Menu]")!;
    return {
      panel: box(panel),
      main: box(main),
      menuButton: box(button),
      expanded: button.getAttribute("aria-expanded"),
      inert: main.inert,
    };
  });
}

/** The focused element: its text, or its accessible name for an icon button. */
const focused = (page: Page) =>
  page.evaluate(
    () =>
      document.activeElement?.getAttribute("aria-label") ??
      document.activeElement?.textContent,
  );

describe.each(themes)("the cabinet at 1440 px in the %s theme", (theme) => {
  test("has the side menu of 08-machines.png", async () => {
    const c = (label: string) => color(label, theme);
    const page = await open(desktop, "cabinet", theme);
    const looks = await page.evaluate(() => {
      const look = (element: Element) => {
        const style = getComputedStyle(element);
        const { left, top, width, height } = element.getBoundingClientRect();
        return {
          box: [left, top, width, height].map(Math.round),
          background: style.backgroundColor,
          color: style.color,
          font: `${style.fontFamily.split(",")[0]!.replaceAll('"', "")} ${style.fontSize} ${style.fontWeight}`,
        };
      };
      const side = document.querySelector("header")!;
      const link = (name: string) =>
        [...side.querySelectorAll("a")].find((a) =>
          a.textContent!.startsWith(name),
        )!;
      const account = side.querySelector("button[aria-label=Account]")!
        .parentElement!.parentElement!;
      return {
        side: {
          ...look(side),
          border: `${getComputedStyle(side).borderRightWidth} ${getComputedStyle(side).borderRightColor}`,
        },
        logo: look(link("skald").lastElementChild!),
        switcher: {
          ...look(side.querySelector("button[aria-haspopup=listbox]")!),
          border: getComputedStyle(
            side.querySelector("button[aria-haspopup=listbox]")!,
          ).borderTopColor,
        },
        current: look(link("Machines")),
        section: look(link("Members")),
        item: look(link("# review")),
        badge: look(link("Queues").lastElementChild!),
        unread: look(link("# deploy").lastElementChild!),
        account: {
          ...look(account),
          border: getComputedStyle(account).borderTopColor,
        },
        avatar: look(account.firstElementChild!),
        heading: look(document.querySelector("main h1")!),
        subtitle: look(document.querySelector("main h1 + p")!),
        menuButton: getComputedStyle(
          document.querySelector("button[aria-label=Menu]")!,
        ).display,
      };
    });

    const inter = "Inter Variable";
    expect(looks.side).toMatchObject({
      box: [0, 0, 240, 900],
      background: c("Side panel"),
      border: `1px ${c("Line")}`,
    });
    expect(looks.logo).toMatchObject({
      color: c("Text"),
      font: "Space Grotesk Variable 19px 600",
    });
    expect(looks.switcher).toMatchObject({
      box: [14, expect.any(Number), 211, 48],
      background: c("Background"),
      border: c("Line"),
    });
    expect(looks.current).toMatchObject({
      box: [14, expect.any(Number), 211, 40],
      background: c("Accent soft"),
      color: c("Accent"),
      font: `${inter} 14px 600`,
    });
    expect(looks.section).toMatchObject({
      box: [14, expect.any(Number), 211, 40],
      background: transparent,
      color: c("Text"),
      font: `${inter} 14px 500`,
    });
    expect(looks.item).toMatchObject({
      box: [24, expect.any(Number), 201, 34],
      background: transparent,
      color: c("Muted text"),
      font: `${inter} 14px 400`,
    });
    expect(looks.badge).toMatchObject({
      background: c("Warning soft"),
      color: c("Warning"),
      font: `${inter} 12px 600`,
    });
    expect(looks.unread).toMatchObject({
      background: transparent,
      color: c("Accent"),
      font: `${inter} 12px 600`,
    });
    // The account footer sits at the bottom of the window, under a line.
    expect(looks.account.box[1]! + looks.account.box[3]!).toBe(900 - 18);
    expect(looks.account.border).toBe(c("Line"));
    expect(looks.avatar).toMatchObject({
      box: [expect.any(Number), expect.any(Number), 32, 32],
      background: c("Accent soft"),
      color: c("Accent"),
      font: `${inter} 13px 600`,
    });
    expect(looks.heading).toMatchObject({
      box: [240 + 28, 24, expect.any(Number), expect.any(Number)],
      color: c("Text"),
      font: "Space Grotesk Variable 26px 600",
    });
    expect(looks.subtitle).toMatchObject({
      color: c("Muted text"),
      font: `${inter} 14px 400`,
    });
    expect(looks.menuButton).toBe("none");
  });
});

test("Chrome exposes the landmarks, the current section and the counts", async () => {
  const page = await open(desktop);
  const cdp = await page.context().newCDPSession(page);
  const { nodes } = await cdp.send("Accessibility.getFullAXTree");
  const byId = new Map(nodes.map((node) => [node.nodeId, node]));
  /** The landmarks and links under `node`, skipping everything else. */
  const tree = (node: (typeof nodes)[number]): unknown[] =>
    (node.childIds ?? []).flatMap((id) => {
      const child = byId.get(id)!;
      const role = child.role?.value as string;
      if (["banner", "navigation", "main"].includes(role))
        return [{ [`${role} ${child.name?.value ?? ""}`.trim()]: tree(child) }];
      if (role === "link") return [`link ${child.name?.value}`];
      return tree(child);
    });
  const root = nodes.find((node) => node.role?.value === "RootWebArea")!;
  expect(tree(root)).toEqual([
    {
      banner: [
        "link skald",
        {
          "navigation Sections": [
            "link Messages",
            "link # review",
            "link # deploy 2 unread",
            "link # research",
            "link Queues 2 waiting",
            "link Machines",
            "link Members",
            "link Access",
            "link Usage",
          ],
        },
      ],
    },
    // The Machines screen has no links.
    { main: [] },
  ]);
  // Chrome passes aria-current on to a screen reader; the tree of CDP leaves it out.
  expect(
    await sections(page)
      .getByRole("link")
      .evaluateAll((links) =>
        links.flatMap((link) =>
          link.hasAttribute("aria-current")
            ? [`${link.textContent} ${link.getAttribute("aria-current")}`]
            : [],
        ),
      ),
  ).toEqual(["Machines page"]);
});

test("the side menu stays in the window while the page scrolls", async () => {
  const page = await open(desktop);
  await page.setViewportSize({ width: desktop, height: 500 });
  await page.evaluate(() => window.scrollTo(0, document.body.scrollHeight));
  expect(await page.evaluate(() => window.scrollY)).toBeGreaterThan(100);
  const side = await page.evaluate(() => {
    const { top, bottom } = document
      .querySelector("header")!
      .getBoundingClientRect();
    return { top, bottom };
  });
  expect(side).toEqual({ top: 0, bottom: 500 });
});

test("choosing a section marks it current; choosing a space shows it", async () => {
  const page = await open(desktop);
  await sections(page).getByRole("link", { name: "Members" }).click();
  expect(await page.locator("main h1").textContent()).toBe("Members");
  expect(
    await sections(page)
      .getByRole("link", { name: "Members" })
      .getAttribute("aria-current"),
  ).toBe("page");

  const switcher = page.getByRole("button", { name: /^Space / });
  await switcher.focus();
  await page.keyboard.press("Enter");
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("Enter");
  expect(await switcher.textContent()).toBe("Space team-b");
  expect(await focused(page)).toBe("Space team-b");
});

describe("the account menu", () => {
  test("works from the keyboard and returns the focus to its button", async () => {
    const page = await open(desktop);
    const account = page.getByRole("button", { name: "Account" });
    await account.focus();
    await page.keyboard.press("Enter");
    expect(await account.getAttribute("aria-expanded")).toBe("true");
    expect(await focused(page)).toBe("Sign-in methods");
    await page.keyboard.press("ArrowDown");
    expect(await focused(page)).toBe("Sign out");
    await page.keyboard.press("Escape");
    expect(await account.getAttribute("aria-expanded")).toBe("false");
    expect(await focused(page)).toBe("Account");

    await page.keyboard.press("ArrowUp");
    expect(await focused(page)).toBe("Sign out");
    await page.keyboard.press("Enter");
    expect(await page.getByRole("menu").count()).toBe(0);
    expect(await focused(page)).toBe("Account");
  });

  test.each([desktop, phone])(
    "stays inside the window at %i px",
    async (width) => {
      const page = await open(
        width,
        width === phone ? "cabinet-menu" : "cabinet",
      );
      await page.getByRole("button", { name: "Account" }).click();
      const { menu, window } = await page.evaluate(() => ({
        menu: document
          .querySelector("[role=menu]")!
          .getBoundingClientRect()
          .toJSON(),
        window: {
          width: document.documentElement.clientWidth,
          height: document.documentElement.clientHeight,
        },
      }));
      expect(menu.left).toBeGreaterThanOrEqual(0);
      expect(menu.top).toBeGreaterThanOrEqual(0);
      expect(menu.right).toBeLessThanOrEqual(window.width);
      expect(menu.bottom).toBeLessThanOrEqual(window.height);
    },
  );
});

describe("at phone width", () => {
  test("the side menu hides behind the menu button", async () => {
    const page = await open(phone);
    expect(await layout(page)).toEqual({
      panel: null,
      main: expect.objectContaining({ left: 0, width: phone }),
      menuButton: expect.objectContaining({ width: 40, height: 40 }),
      expanded: "false",
      inert: false,
    });
    expect(await sections(page).isVisible()).toBe(false);
  });

  test("the bar with the menu button stays on top while the page scrolls", async () => {
    const page = await open(phone);
    await page.evaluate(() => window.scrollTo(0, 200));
    expect(await page.evaluate(() => window.scrollY)).toBeGreaterThan(0);
    expect((await menuButton(page).boundingBox())!.y).toBeGreaterThanOrEqual(0);
    expect(
      await page.evaluate(
        () => document.querySelector("header")!.getBoundingClientRect().top,
      ),
    ).toBe(0);
  });

  test("the menu button opens the side menu over the page and puts the page out of reach", async () => {
    const page = await open(phone);
    await menuButton(page).click();
    const shown = await layout(page);
    expect(shown).toMatchObject({
      panel: { left: 0, top: 56, width: 280, height: 900 - 56 },
      expanded: "true",
      inert: true,
    });
    // The side menu lies over the page, which is dimmed beside it.
    const hits = await page.evaluate(() => {
      const at = (x: number, y: number) => {
        const element = document.elementFromPoint(x, y)!;
        if (element.closest("nav")?.parentElement?.contains(element))
          return "side menu";
        if (element.closest("main")) return "page";
        return getComputedStyle(element).backgroundColor;
      };
      return { menu: at(100, 400), beside: at(330, 400) };
    });
    expect(hits).toEqual({ menu: "side menu", beside: "rgba(0, 0, 0, 0.4)" });

    // Tab goes through the bar and the side menu, never into the page.
    const reached: string[] = [];
    for (let i = 0; i < 30; i++) {
      await page.keyboard.press("Tab");
      reached.push(
        await page.evaluate(() =>
          document.activeElement?.closest("main") ? "page" : "menu",
        ),
      );
    }
    expect(reached).not.toContain("page");

    await menuButton(page).click();
    expect(await layout(page)).toMatchObject({
      panel: null,
      expanded: "false",
      inert: false,
    });
  });

  test("Escape closes the side menu and returns the focus to the menu button", async () => {
    const page = await open(phone);
    await menuButton(page).focus();
    await page.keyboard.press("Enter");
    expect((await layout(page)).expanded).toBe("true");
    // The logo, the space switcher, then the first section.
    for (let i = 0; i < 3; i++) await page.keyboard.press("Tab");
    expect(await focused(page)).toBe("Messages");
    await page.keyboard.press("Escape");
    expect(await layout(page)).toMatchObject({
      panel: null,
      expanded: "false",
      inert: false,
    });
    expect(await focused(page)).toBe("Menu");
  });

  test("Escape in the space list closes the list, then the side menu", async () => {
    const page = await open(phone, "cabinet-menu");
    const switcher = page.getByRole("button", { name: /^Space / });
    await switcher.click();
    const list = await page.evaluate(() =>
      document
        .querySelector("[role=listbox]")!
        .getBoundingClientRect()
        .toJSON(),
    );
    // The list stays inside the window.
    expect(list.left).toBeGreaterThanOrEqual(8);
    expect(list.right).toBeLessThanOrEqual(phone - 8);
    await page.keyboard.press("Escape");
    expect(await page.getByRole("listbox").count()).toBe(0);
    expect(await focused(page)).toBe("Space team-a");
    expect((await layout(page)).expanded).toBe("true");
    await page.keyboard.press("Escape");
    expect((await layout(page)).expanded).toBe("false");
    expect(await focused(page)).toBe("Menu");
  });

  test("choosing a section closes the side menu and marks it current", async () => {
    const page = await open(phone, "cabinet-menu");
    await sections(page)
      .getByRole("link", { name: /^Queues/ })
      .click();
    expect(await layout(page)).toMatchObject({
      panel: null,
      expanded: "false",
      inert: false,
    });
    expect(await page.locator("main h1").textContent()).toBe("Queues");
    expect(await focused(page)).toBe("Menu");
    await menuButton(page).click();
    expect(
      await sections(page)
        .getByRole("link", { name: /^Queues/ })
        .getAttribute("aria-current"),
    ).toBe("page");
  });

  test("a click on the page beside the side menu closes it", async () => {
    const page = await open(phone, "cabinet-menu");
    await page.mouse.click(330, 400);
    expect(await layout(page)).toMatchObject({
      panel: null,
      expanded: "false",
      inert: false,
    });
  });
});

describe("resizing the window", () => {
  test("from phone width with the side menu open shows it beside the page, then hides it closed", async () => {
    const page = await open(phone, "cabinet-menu");
    expect((await layout(page)).inert).toBe(true);
    await page.setViewportSize({ width: desktop, height: 900 });
    await expect
      .poll(() => layout(page))
      .toEqual({
        panel: expect.objectContaining({
          left: 14,
          top: expect.any(Number),
          width: 211,
        }),
        main: expect.objectContaining({ left: 240, width: desktop - 240 }),
        menuButton: null,
        expanded: "false",
        inert: false,
      });
    expect(
      await page.evaluate(
        () => document.elementFromPoint(800, 450)!.closest("main") !== null,
      ),
    ).toBe(true);

    await page.setViewportSize({ width: phone, height: 900 });
    await expect
      .poll(() => layout(page))
      .toMatchObject({
        panel: null,
        expanded: "false",
        inert: false,
      });
  });

  test("from desktop to phone width hides the side menu behind the menu button", async () => {
    const page = await open(desktop);
    expect((await layout(page)).menuButton).toBeNull();
    await page.setViewportSize({ width: phone, height: 900 });
    await expect
      .poll(() => layout(page))
      .toMatchObject({
        panel: null,
        main: expect.objectContaining({ left: 0, width: phone }),
        menuButton: expect.objectContaining({ width: 40 }),
        expanded: "false",
      });
    await menuButton(page).click();
    expect((await layout(page)).inert).toBe(true);
  });
});
