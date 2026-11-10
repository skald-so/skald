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

/** The text of every cell of `table`'s body rows, row by row. */
function cells(table: HTMLElement): string[][] {
  const [, ...rows] = within(table).getAllByRole("row");
  return rows.map((row) =>
    within(row)
      .getAllByRole("cell")
      .map((cell) => cell.textContent!),
  );
}

test("shows the fixture tables in the order the screens set", () => {
  render(<KitPage />);
  const section = within(screen.getByRole("region", { name: "Tables" }));

  expect(
    section
      .getAllByRole("table")
      .map((table) => table.getAttribute("aria-label")),
  ).toEqual([
    "Devices",
    "Accounts",
    "All queues",
    "Devices, empty",
    "Devices, loading",
  ]);
  expect(cells(section.getByRole("table", { name: "Devices" }))).toEqual([
    [
      "ana-laptop",
      "Machine · macOS",
      "Online",
      "7F3A 91C2 0B5E 44D8Verified with your GitHub SSH key",
      "Revoke",
    ],
    [
      "Chromethis browser",
      "Browser · macOS",
      "Online",
      "E2C7 0A9D 3B16 F4A2Approved by ana-laptop",
      "Revoke",
    ],
    [
      "ana-cloud",
      "Machine · Linux sandbox",
      "3 h ago",
      "A06E 13FB 72C9 5E01Approved by ana-laptop",
      "Revoke",
    ],
    [
      "ana-desktop",
      "Machine · Windows",
      "1 day ago",
      "19BE 4F02 D7A3 C680Not verified · compare keys",
      "Revoke",
    ],
  ]);
  expect(
    cells(section.getByRole("table", { name: "Accounts" })).map((row) => [
      row[0],
      row[4],
    ]),
  ).toEqual([
    ["NorthwindSSO · northwind.io", "6.1M"],
    ["spam-relayGitHub · spam-relay", "980k"],
    ["Acme CIGoogle · ci@acme.dev", "920k"],
    ["Ana KovacGitHub · ana", "12k"],
    ["Lee ParkGoogle · lee.park@gmail.com", "3k"],
    ["free-tokensGitHub · free-tokens", "0"],
  ]);
  expect(cells(section.getByRole("table", { name: "All queues" }))).toEqual([
    ["deploy", "0", "0", "2", "—"],
    ["research", "1", "2", "0", "40 s"],
    ["review", "3", "1", "0", "2 min"],
  ]);
  expect(cells(section.getByRole("table", { name: "Devices, empty" }))).toEqual(
    [
      [
        "No machines yet. Connect one to send and receive messages from its agents.Connect a machine",
      ],
    ],
  );
  const loading = section.getByRole("table", { name: "Devices, loading" });
  expect(loading.getAttribute("aria-busy")).toBe("true");
  expect(cells(loading)).toEqual([["Loading devices"]]);
});

test("the fixture tables sort by a click on a header", async () => {
  const user = userEvent.setup();
  render(<KitPage />);
  const section = within(screen.getByRole("region", { name: "Tables" }));
  const accounts = section.getByRole("table", { name: "Accounts" });
  const created = () => cells(accounts).map((row) => row[1]);

  await user.click(within(accounts).getByRole("button", { name: "Created" }));
  expect(created()).toEqual([
    "Aug 1",
    "Sep 14",
    "Oct 2",
    "Oct 30",
    "Nov 4",
    "Nov 9",
  ]);
  await user.click(within(accounts).getByRole("button", { name: "Created" }));
  expect(created()).toEqual([
    "Nov 9",
    "Nov 4",
    "Oct 30",
    "Oct 2",
    "Sep 14",
    "Aug 1",
  ]);

  const queues = section.getByRole("table", { name: "All queues" });
  await user.click(
    within(queues).getByRole("button", { name: "Oldest waiting" }),
  );
  expect(cells(queues).map((row) => row[4])).toEqual(["—", "40 s", "2 min"]);
  await user.click(
    within(queues).getByRole("button", { name: "Oldest waiting" }),
  );
  expect(cells(queues).map((row) => row[4])).toEqual(["2 min", "40 s", "—"]);

  const devices = section.getByRole("table", { name: "Devices" });
  await user.click(within(devices).getByRole("button", { name: "Last seen" }));
  expect(cells(devices).map((row) => row[2])).toEqual([
    "1 day ago",
    "3 h ago",
    "Online",
    "Online",
  ]);
});

test("shows the empty and loading states on a card and on a tinted card", () => {
  render(<KitPage />);
  const section = screen.getByRole("region", { name: "Tables" });

  const cards = within(section).getAllByRole("listitem");
  expect(cards.map((card) => card.textContent)).toEqual([
    "Empty · on a cardNo tasks are running.Open the queues",
    "Loading · on a cardLoading tasks",
    "Empty · on a tinted cardNo tasks are running.Open the queues",
    "Loading · on a tinted cardLoading tasks",
  ]);
});

/** The kit's filtered accounts: the first line of each row's account cell. */
function filteredAccounts(): string[] {
  const table = screen.getByRole("table", { name: "Filtered accounts" });
  return cells(table).map((row) =>
    row.length === 1
      ? row[0]!
      : row[0]!.replace(/(SSO|GitHub|Google) · .*/, ""),
  );
}

test("the filter buttons and search narrow the fixture accounts", async () => {
  const user = userEvent.setup();
  render(<KitPage />);
  const section = within(
    screen.getByRole("region", { name: "Filter buttons and search" }),
  );
  const filters = within(
    section.getByRole("group", { name: "Filter accounts" }),
  );
  const search = section.getByRole("searchbox", { name: "Search accounts" });
  const everyone = [
    "Northwind",
    "spam-relay",
    "Acme CI",
    "Ana Kovac",
    "Lee Park",
    "free-tokens",
  ];
  expect(
    filters
      .getAllByRole("button")
      .map((button) => [
        button.textContent,
        button.getAttribute("aria-pressed"),
      ]),
  ).toEqual([
    ["All", "true"],
    ["Flagged", "false"],
    ["Banned", "false"],
    ["Enterprise", "false"],
  ]);
  expect(filteredAccounts()).toEqual(everyone);

  for (const [choice, shown] of [
    ["Flagged", ["spam-relay"]],
    ["Banned", ["free-tokens"]],
    ["Enterprise", ["Northwind"]],
    ["All", everyone],
  ] as const) {
    await user.click(filters.getByRole("button", { name: choice }));
    expect(filteredAccounts(), choice).toEqual(shown);
  }

  // Search by name, login or email, in any case.
  await user.type(search, "LEE");
  expect(filteredAccounts()).toEqual(["Lee Park"]);
  await user.clear(search);
  await user.type(search, "acme.dev");
  expect(filteredAccounts()).toEqual(["Acme CI"]);
  await user.clear(search);
  await user.type(search, "  spam ");
  expect(filteredAccounts()).toEqual(["spam-relay"]);

  // Filter and search together; nothing found shows the empty state, whose action resets both.
  await user.click(filters.getByRole("button", { name: "Banned" }));
  expect(filteredAccounts()).toEqual(["No accounts match.Show all accounts"]);
  await user.click(section.getByRole("button", { name: "Show all accounts" }));
  expect(filteredAccounts()).toEqual(everyone);
  expect(search).toHaveProperty("value", "");
  expect(
    filters.getByRole("button", { name: "All" }).getAttribute("aria-pressed"),
  ).toBe("true");

  await user.type(search, "ana");
  expect(filteredAccounts()).toEqual(["Ana Kovac"]);
  await user.click(
    section.getAllByRole("button", { name: "Clear search" })[0]!,
  );
  expect(filteredAccounts()).toEqual(everyone);
});

test("shows the joined view switch and a search with a query", async () => {
  const user = userEvent.setup();
  render(<KitPage />);
  const section = within(
    screen.getByRole("region", { name: "Filter buttons and search" }),
  );
  const [joined, query] = section.getAllByRole("listitem");
  expect(joined!.firstChild?.textContent).toBe(
    "Joined · as the view of Messages",
  );
  expect(query!.firstChild?.textContent).toBe(
    "With a query · a button clears it",
  );

  const view = within(within(joined!).getByRole("group", { name: "View" }));
  expect(
    view
      .getAllByRole("button")
      .map((button) => [
        button.textContent,
        button.getAttribute("aria-pressed"),
      ]),
  ).toEqual([
    ["Thread", "true"],
    ["Graph", "false"],
  ]);
  expect(joined!.textContent).toContain("Showing the thread");
  await user.click(view.getByRole("button", { name: "Graph" }));
  expect(
    view.getByRole("button", { name: "Graph" }).getAttribute("aria-pressed"),
  ).toBe("true");
  expect(
    view.getByRole("button", { name: "Thread" }).getAttribute("aria-pressed"),
  ).toBe("false");
  expect(joined!.textContent).toContain("Showing the graph");
  await user.type(
    within(joined!).getByRole("searchbox", { name: "Search messages" }),
    "lease",
  );
  expect(joined!.textContent).toContain(
    "Showing the graph · searching for “lease”",
  );

  const members = within(query!).getByRole("searchbox", {
    name: "Search members",
  });
  expect(members).toHaveProperty("value", "ana");
  expect(query!.textContent).toContain("Query “ana”");
  await user.click(
    within(query!).getByRole("button", { name: "Clear search" }),
  );
  expect(members).toHaveProperty("value", "");
  expect(query!.textContent).toContain("No query");
});

/** The kit's filtered messages: the time of each row, or the empty state. */
function filteredMessages(): string[] {
  const table = screen.getByRole("table", { name: "Filtered messages" });
  return cells(table).map((row) => row.at(-1)!);
}

test("the dropdown filters narrow and order the fixture messages", async () => {
  const user = userEvent.setup();
  render(<KitPage />);
  const section = within(
    screen.getByRole("region", { name: "Dropdown filters" }),
  );
  const group = section.getByRole("group", { name: "Filter messages" });
  const filters = within(group);
  // The filters and the sort order; the open sample has a Type filter too.
  const bar = within(group.parentElement!);
  const shownFilters = () =>
    filters
      .getAllByRole("button")
      .map((button) => [button.textContent, button.hasAttribute("data-set")]);
  const choose = async (name: string, option: string) => {
    await user.click(
      bar.getByRole("button", { name: new RegExp(`^${name} `) }),
    );
    await user.click(bar.getByRole("option", { name: option }));
  };
  const today = ["Today 10:42", "Today 10:44", "Today 10:46", "Today 10:58"];
  const all = ["Yesterday 16:20", "Yesterday 17:05", ...today];

  // As 04-messages.png: today's messages, oldest first.
  expect(shownFilters()).toEqual([
    ["From anyone", false],
    ["Status any", false],
    ["Type messages and tasks", false],
    ["Time today", true],
    ["Clear", false],
  ]);
  expect(
    section.getByRole("button", { name: "Sort Oldest first" }),
  ).toBeTruthy();
  expect(filteredMessages()).toEqual(today);

  await choose("Sort", "Newest first");
  expect(filteredMessages()).toEqual([...today].reverse());
  await choose("Sort", "Oldest first");

  await choose("From", "codex");
  expect(filteredMessages()).toEqual(["Today 10:44", "Today 10:58"]);
  await choose("Status", "stored");
  expect(filteredMessages()).toEqual(["Today 10:58"]);
  await choose("Time", "any time");
  await choose("Status", "read");
  expect(filteredMessages()).toEqual(["Yesterday 16:20", "Today 10:44"]);
  await choose("From", "anyone");
  await choose("Status", "any");
  await choose("Type", "tasks");
  expect(filteredMessages()).toEqual(["Yesterday 17:05", "Today 10:42"]);
  await choose("Type", "messages");
  await choose("Time", "yesterday");
  expect(filteredMessages()).toEqual(["Yesterday 16:20"]);

  // Clear restores every default and goes away; the sort order stays.
  await choose("Sort", "Newest first");
  await user.click(filters.getByRole("button", { name: "Clear" }));
  expect(shownFilters()).toEqual([
    ["From anyone", false],
    ["Status any", false],
    ["Type messages and tasks", false],
    ["Time any time", false],
  ]);
  expect(filteredMessages()).toEqual([...all].reverse());

  // Nothing found shows the empty state, whose action clears the filters.
  await choose("Status", "stored");
  await choose("Type", "tasks");
  expect(filteredMessages()).toEqual(["No messages match.Clear filters"]);
  await user.click(section.getByRole("button", { name: "Clear filters" }));
  expect(filteredMessages()).toEqual([...all].reverse());
  expect(filters.queryByRole("button", { name: "Clear" })).toBeNull();
});

test("shows a set dropdown filter with its list open", () => {
  render(<KitPage />);
  const section = within(
    screen.getByRole("region", { name: "Dropdown filters" }),
  );
  const [open] = section.getAllByRole("listitem");
  expect(open!.firstChild?.textContent).toBe("Open · a set filter");

  const button = within(open!).getByRole("button", { name: "Type tasks" });
  expect(button.getAttribute("aria-expanded")).toBe("true");
  expect(button.hasAttribute("data-set")).toBe(true);
  expect(
    within(open!)
      .getAllByRole("option")
      .map((option) => [
        option.textContent,
        option.getAttribute("aria-selected"),
      ]),
  ).toEqual([
    ["messages and tasks", "false"],
    ["messages", "false"],
    ["tasks", "true"],
  ]);
});

test.each<[Theme | undefined, string]>([
  [undefined, ""],
  ["dark", "theme=dark&"],
])("with the theme %s, links to the cabinet views keep it", (theme, query) => {
  render(<KitPage theme={theme} />);
  const section = within(screen.getByRole("region", { name: "Cabinet" }));

  expect(
    section
      .getAllByRole("link")
      .map((link) => [link.textContent, link.getAttribute("href")]),
  ).toEqual([
    ["The cabinet around Machines", `kit.html?${query}screen=cabinet`],
    [
      "The side menu open at phone width",
      `kit.html?${query}screen=cabinet-menu`,
    ],
  ]);
});

test("shows the space switcher and the account menu open", () => {
  render(<KitPage />);
  const section = within(screen.getByRole("region", { name: "Cabinet" }));

  const switcher = section.getByRole("button", { name: "Space team-b" });
  expect(switcher.getAttribute("aria-expanded")).toBe("true");
  expect(
    section
      .getAllByRole("option")
      .map((option) => [
        option.textContent,
        option.getAttribute("aria-selected"),
      ]),
  ).toEqual([
    ["team-a", "false"],
    ["team-b", "true"],
    ["ana", "false"],
  ]);
  expect(
    section
      .getByRole("button", { name: "Account" })
      .getAttribute("aria-expanded"),
  ).toBe("true");
  expect(
    within(section.getByRole("menu", { name: "Account" }))
      .getAllByRole("menuitem")
      .map((item) => item.textContent),
  ).toEqual(["Sign-in methods", "Sign out"]);
  expect(section.getByText("Ana Kovac")).toBeTruthy();
});
