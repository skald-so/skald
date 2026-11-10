import { act, cleanup, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { afterEach, expect, test, vi } from "vitest";
import a11y from "./a11y.module.css";
import { PageHeader } from "./PageHeader.tsx";
import { Shell, type ShellProps, type ShellSection } from "./Shell.tsx";

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

const sections: ShellSection[] = [
  {
    key: "messages",
    label: "Messages",
    href: "#messages",
    items: [
      { key: "review", label: "# review", href: "#review" },
      {
        key: "deploy",
        label: "# deploy",
        href: "#deploy",
        badge: { count: 2, label: "unread", tone: "accent" },
      },
    ],
  },
  {
    key: "queues",
    label: "Queues",
    href: "#queues",
    badge: { count: 3, label: "waiting", tone: "warning" },
  },
  { key: "machines", label: "Machines", href: "#machines" },
];

const spaces = [
  { value: "team-a", label: "team-a" },
  { value: "team-b", label: "team-b" },
];

/** A screen that keeps the current section and space and reports each change. */
function Screen({
  initial = "machines",
  onNavigate = () => {},
  onSpaceChange = () => {},
  ...props
}: {
  initial?: string;
  onNavigate?: (key: string) => void;
  onSpaceChange?: (space: string) => void;
} & Partial<Pick<ShellProps, "defaultMenuOpen">>) {
  const [current, setCurrent] = useState(initial);
  const [space, setSpace] = useState("team-a");
  return (
    <Shell
      {...props}
      sections={sections}
      current={current}
      onNavigate={(key) => {
        onNavigate(key);
        setCurrent(key);
      }}
      spaces={spaces}
      space={space}
      onSpaceChange={(next) => {
        onSpaceChange(next);
        setSpace(next);
      }}
      account={{
        name: "Ana Kovac",
        initials: "AK",
        signIn: "GitHub · ana",
        actions: [{ label: "Sign out", onSelect: () => {} }],
      }}
    >
      <PageHeader
        title="Machines"
        subtitle="Your devices."
        actions={<button type="button">Connect a machine</button>}
      />
    </Shell>
  );
}

/**
 * Replaces matchMedia with one where the phone width query matches `phone`; `resize` changes it
 * and tells the listeners.
 */
function stubWidth(phone: boolean) {
  let matches = phone;
  const listeners = new Set<() => void>();
  vi.stubGlobal("matchMedia", (query: string) => {
    expect(query).toBe("(width < 800px)");
    return {
      get matches() {
        return matches;
      },
      addEventListener: (_: string, listener: () => void) =>
        listeners.add(listener),
      removeEventListener: (_: string, listener: () => void) =>
        listeners.delete(listener),
    };
  });
  return {
    resize(phone: boolean) {
      matches = phone;
      act(() => listeners.forEach((listener) => listener()));
    },
  };
}

const sectionsNav = () => screen.getByRole("navigation", { name: "Sections" });
/** Each link of the side menu with its `aria-current`. */
const marks = () =>
  within(sectionsNav())
    .getAllByRole("link")
    .map((link) => [link.textContent, link.getAttribute("aria-current")]);
const menuButton = () => screen.getByRole("button", { name: "Menu" });
const main = () => screen.getByRole("main");

test("the side menu and the page are landmarks", () => {
  render(<Screen />);

  const banner = screen.getByRole("banner");
  expect(within(banner).getByRole("link", { name: "skald" })).toBeTruthy();
  expect(within(banner).getByRole("button", { name: "Space team-a" })).toBe(
    screen.getByRole("button", { name: /^Space / }),
  );
  expect(banner.contains(sectionsNav())).toBe(true);
  expect(within(banner).getByRole("button", { name: "Account" })).toBeTruthy();
  expect(
    within(main()).getByRole("heading", { level: 1, name: "Machines" }),
  ).toBeTruthy();
  expect(within(main()).getByText("Your devices.")).toBeTruthy();
  expect(
    within(main()).getByRole("button", { name: "Connect a machine" }),
  ).toBeTruthy();
});

test("the current section is marked as the current page, the others are not", () => {
  render(<Screen />);

  expect(marks()).toEqual([
    ["Messages", null],
    ["# review", null],
    ["# deploy 2 unread", null],
    ["Queues 3 waiting", null],
    ["Machines", "page"],
  ]);
});

test("a current sub-item is the current page, and its section is marked current", () => {
  render(<Screen initial="deploy" />);

  expect(marks()).toEqual([
    ["Messages", "true"],
    ["# review", null],
    ["# deploy 2 unread", "page"],
    ["Queues 3 waiting", null],
    ["Machines", null],
  ]);
});

test("each link goes to its section; a count shows only its number", () => {
  render(<Screen />);

  const links = within(sectionsNav()).getAllByRole("link");
  expect(links.map((link) => link.getAttribute("href"))).toEqual([
    "#messages",
    "#review",
    "#deploy",
    "#queues",
    "#machines",
  ]);
  // The meaning of a count is for a screen reader only; Chrome reads "Queues 3 waiting".
  expect(
    links.flatMap((link) =>
      [...link.querySelectorAll("span span")].map((label) => [
        label.textContent,
        label.className,
      ]),
    ),
  ).toEqual([
    [" unread", a11y.visuallyHidden],
    [" waiting", a11y.visuallyHidden],
  ]);
});

test("choosing a section reports it and the screen marks it current", async () => {
  const onNavigate = vi.fn();
  render(<Screen onNavigate={onNavigate} />);

  await userEvent.setup().click(screen.getByRole("link", { name: "# review" }));
  expect(onNavigate.mock.calls).toEqual([["review"]]);
  expect(marks().filter(([, mark]) => mark)).toEqual([
    ["Messages", "true"],
    ["# review", "page"],
  ]);
});

test("choosing a space by click or keyboard reports it and shows it", async () => {
  const user = userEvent.setup();
  const onSpaceChange = vi.fn();
  render(<Screen onSpaceChange={onSpaceChange} />);

  const switcher = () => screen.getByRole("button", { name: /^Space / });
  expect(switcher().getAttribute("aria-haspopup")).toBe("listbox");
  // The current space is not a set filter: nothing is highlighted.
  expect(switcher().hasAttribute("data-set")).toBe(false);
  await user.click(switcher());
  await user.click(screen.getByRole("option", { name: "team-b" }));
  expect(switcher().textContent).toBe("Space team-b");
  expect(switcher().hasAttribute("data-set")).toBe(false);

  await user.keyboard("{ArrowDown}");
  await user.keyboard("{ArrowUp}");
  await user.keyboard("{Enter}");
  expect(switcher().textContent).toBe("Space team-a");
  expect(onSpaceChange.mock.calls).toEqual([["team-b"], ["team-a"]]);
});

test("the account footer shows the initials, the name and the sign-in method, and a menu", async () => {
  render(<Screen />);

  const banner = within(screen.getByRole("banner"));
  const initials = banner.getByText("AK");
  expect(initials.getAttribute("aria-hidden")).toBe("true");
  expect(banner.getByText("Ana Kovac")).toBeTruthy();
  expect(banner.getByText("GitHub · ana")).toBeTruthy();
  await userEvent
    .setup()
    .click(banner.getByRole("button", { name: "Account" }));
  expect(screen.getByRole("menu", { name: "Account" }).textContent).toBe(
    "Sign out",
  );
});

test("above phone width the side menu is always open and the page reachable", async () => {
  stubWidth(false);
  render(<Screen defaultMenuOpen />);

  expect(menuButton().getAttribute("aria-expanded")).toBe("false");
  expect(main().hasAttribute("inert")).toBe(false);
  await userEvent.setup().click(menuButton());
  expect(menuButton().getAttribute("aria-expanded")).toBe("false");
  expect(main().hasAttribute("inert")).toBe(false);
});

test("at phone width the menu button opens and closes the side menu over the page", async () => {
  stubWidth(true);
  const user = userEvent.setup();
  render(<Screen />);

  const panel = () =>
    document.getElementById(menuButton().getAttribute("aria-controls")!)!;
  expect(panel().contains(sectionsNav())).toBe(true);
  expect(menuButton().getAttribute("aria-expanded")).toBe("false");
  expect(panel().hasAttribute("data-open")).toBe(false);
  expect(main().hasAttribute("inert")).toBe(false);

  await user.click(menuButton());
  expect(menuButton().getAttribute("aria-expanded")).toBe("true");
  expect(panel().hasAttribute("data-open")).toBe(true);
  // The page under the open menu is out of reach.
  expect(main().hasAttribute("inert")).toBe(true);

  await user.click(menuButton());
  expect(menuButton().getAttribute("aria-expanded")).toBe("false");
  expect(panel().hasAttribute("data-open")).toBe(false);
  expect(main().hasAttribute("inert")).toBe(false);
});

test("at phone width Escape closes the side menu and returns the focus to the menu button", async () => {
  stubWidth(true);
  const user = userEvent.setup();
  render(<Screen />);

  await user.click(menuButton());
  act(() => screen.getByRole("link", { name: "Machines" }).focus());
  await user.keyboard("{Escape}");
  expect(menuButton().getAttribute("aria-expanded")).toBe("false");
  expect(document.activeElement).toBe(menuButton());
});

test("at phone width Escape in the space list closes only the list", async () => {
  stubWidth(true);
  const user = userEvent.setup();
  render(<Screen />);

  await user.click(menuButton());
  await user.click(screen.getByRole("button", { name: /^Space / }));
  await user.keyboard("{Escape}");
  expect(screen.queryByRole("listbox")).toBeNull();
  expect(menuButton().getAttribute("aria-expanded")).toBe("true");
  await user.keyboard("{Escape}");
  expect(menuButton().getAttribute("aria-expanded")).toBe("false");
});

test("at phone width choosing a section closes the side menu", async () => {
  stubWidth(true);
  const user = userEvent.setup();
  const onNavigate = vi.fn();
  render(<Screen onNavigate={onNavigate} />);

  await user.click(menuButton());
  await user.click(screen.getByRole("link", { name: /^Queues/ }));
  expect(onNavigate.mock.calls).toEqual([["queues"]]);
  expect(menuButton().getAttribute("aria-expanded")).toBe("false");
  expect(main().hasAttribute("inert")).toBe(false);
  expect(document.activeElement).toBe(menuButton());
});

test("at phone width a click on the page beside the side menu closes it", async () => {
  stubWidth(true);
  const user = userEvent.setup();
  render(<Screen />);

  /** The element between the side menu and the page: the backdrop over the page while open. */
  const between = () => main().previousElementSibling!;
  await user.click(menuButton());
  expect(between().getAttribute("aria-hidden")).toBe("true");
  await user.click(between());
  expect(menuButton().getAttribute("aria-expanded")).toBe("false");
  expect(between().tagName).toBe("HEADER");
});

test("widening the window past phone width closes the side menu, narrowing keeps it closed", async () => {
  const width = stubWidth(true);
  render(<Screen defaultMenuOpen />);

  expect(menuButton().getAttribute("aria-expanded")).toBe("true");
  expect(main().hasAttribute("inert")).toBe(true);
  width.resize(false);
  expect(menuButton().getAttribute("aria-expanded")).toBe("false");
  expect(main().hasAttribute("inert")).toBe(false);
  width.resize(true);
  expect(menuButton().getAttribute("aria-expanded")).toBe("false");
  expect(main().hasAttribute("inert")).toBe(false);
});

test("a heading without a subtitle or actions shows only the heading", () => {
  render(<PageHeader title="Members" />);

  const header = screen.getByRole("heading", { level: 1 }).parentElement!
    .parentElement!;
  expect(header.textContent).toBe("Members");
  expect(within(header).queryByRole("button")).toBeNull();
});
