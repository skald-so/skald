import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, test, vi } from "vitest";
import { KitPage, type Theme } from "./KitPage.tsx";

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

test("shows the color tokens and fonts without network requests", () => {
  const fetch = vi.spyOn(globalThis, "fetch");
  render(<KitPage />);

  const colors = within(screen.getByRole("region", { name: "Colors" }));
  expect(colors.getAllByRole("listitem")).toHaveLength(9);
  const fonts = within(screen.getByRole("region", { name: "Fonts" }));
  expect(
    fonts.getAllByRole("listitem").map((font) => font.firstChild?.textContent),
  ).toEqual([
    "Inter · Text",
    "Space Grotesk · Headings and numbers",
    "JetBrains Mono · Code, keys and IDs",
  ]);
  expect(fetch).not.toHaveBeenCalled();
});

test.each<[Theme | undefined, string]>([
  [undefined, "System"],
  ["light", "Light"],
  ["dark", "Dark"],
])("with the theme %s, the %s theme link is current", (theme, current) => {
  render(<KitPage theme={theme} />);

  const links = within(
    screen.getByRole("navigation", { name: "Theme" }),
  ).getAllByRole("link");
  expect(
    links.map((link) => [
      link.textContent,
      link.getAttribute("href"),
      link.getAttribute("aria-current"),
    ]),
  ).toEqual(
    [
      ["System", "kit.html"],
      ["Light", "kit.html?theme=light"],
      ["Dark", "kit.html?theme=dark"],
    ].map(([label, href]) => [label, href, label === current ? "page" : null]),
  );
});

test("shows every button kind in every state", () => {
  render(<KitPage />);

  const cards = within(
    screen.getByRole("region", { name: "Buttons" }),
  ).getAllByRole("listitem");
  const shown = cards.map((card) => ({
    name: card.firstChild?.textContent,
    buttons: within(card)
      .getAllByRole("button")
      .map((button) => [
        button.textContent,
        // The caption under the button.
        button.parentElement!.lastElementChild!.textContent,
        button.hasAttribute("disabled"),
        button.getAttribute("aria-busy") === "true",
      ]),
  }));
  const states = (label: string) => [
    [label, "Default", false, false],
    [label, "Disabled", true, false],
    [label, "Busy", false, true],
  ];
  expect(shown).toEqual([
    { name: "Primary · the main action", buttons: states("Approve") },
    { name: "Secondary · other actions", buttons: states("Reject") },
    { name: "Danger · revokes or deletes", buttons: states("Revoke") },
    {
      name: "Danger primary · the main action of a dangerous item",
      buttons: states("Ban account"),
    },
    {
      name: "Sizes · page action, cards and dialogs, table rows",
      buttons: [
        ["Connect a machine", "Large", false, false],
        ["Approve", "Medium", false, false],
        ["Manage", "Small", false, false],
      ],
    },
    {
      name: "Busy on press · for 2 seconds",
      buttons: [["Save", "", false, false]],
    },
    {
      name: "On a tinted card · as the join request",
      buttons: [
        ["Approve", "Primary", false, false],
        ["Reject", "Secondary", false, false],
      ],
    },
  ]);
});

test("shows the status labels in every tone", () => {
  render(<KitPage />);

  const table = within(screen.getByRole("region", { name: "Status labels" }));
  expect(
    table
      .getAllByRole("row")
      .map((row) =>
        [...row.querySelectorAll("th, td")].map((cell) => cell.textContent),
      ),
  ).toEqual([
    ["Tone", "Filled label", "Dot with text", "Count badge"],
    ["OK", "Active", "Online", "3 online"],
    ["Warning", "Flagged", "Not verified", "2 in dead letters"],
    ["Danger", "Banned", "Expired", "1 failed"],
    ["Accent", "Limit request", "Syncing", "2 unread"],
    ["Muted", "Revoked", "3 h ago", "0 waiting"],
  ]);
});

test("shows warning banners with and without an icon and actions", () => {
  render(<KitPage />);

  const banners = within(
    screen.getByRole("region", { name: "Warning banner" }),
  ).getAllByRole("alert");
  expect(
    banners.map((banner) => [
      banner.querySelector("svg") !== null,
      within(banner)
        .queryAllByRole("button")
        .map((button) => button.textContent),
    ]),
  ).toEqual([
    [true, ["Pause thread"]],
    [false, ["Make Lee an admin"]],
    [false, []],
  ]);
});

test("a press turns the Save and Pause thread samples busy for 2 seconds", () => {
  vi.useFakeTimers();
  try {
    render(<KitPage />);

    for (const [name, announced] of [
      ["Save", "Saving"],
      ["Pause thread", "Pausing the thread"],
    ] as const) {
      const button = screen.getByRole("button", { name });
      const status = button.nextElementSibling!;
      expect(status.getAttribute("role")).toBe("status");
      expect(button.hasAttribute("aria-busy")).toBe(false);

      // user-event waits on a timer, which the fake timers would hold.
      fireEvent.click(button);
      expect(button.getAttribute("aria-busy")).toBe("true");
      expect(status.textContent).toBe(announced);

      act(() => vi.advanceTimersByTime(1999));
      expect(button.getAttribute("aria-busy")).toBe("true");
      act(() => vi.advanceTimersByTime(1));
      expect(button.hasAttribute("aria-busy")).toBe(false);
      expect(status.textContent).toBe("");
    }
  } finally {
    vi.useRealTimers();
  }
});

test("Show a warning adds a banner and Hide the warning removes it", async () => {
  const user = userEvent.setup();
  render(<KitPage />);
  const section = within(
    screen.getByRole("region", { name: "Warning banner" }),
  );
  expect(section.getAllByRole("alert")).toHaveLength(3);

  await user.click(section.getByRole("button", { name: "Show a warning" }));
  const added = section.getAllByRole("alert");
  expect(added).toHaveLength(4);
  expect(added[3]!.textContent).toBe(
    "The lease of review-42 runs out in 1 minute.Extend the lease",
  );

  await user.click(section.getByRole("button", { name: "Hide the warning" }));
  expect(section.getAllByRole("alert")).toHaveLength(3);
});
