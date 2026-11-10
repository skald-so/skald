import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, test, vi } from "vitest";
import { MoreIcon } from "./icons.tsx";
import { Menu } from "./Menu.tsx";

afterEach(cleanup);

const labels = ["Sign-in methods", "Linked devices", "Sign out"];

/** A menu between two buttons; each action reports its label to `onSelect`. */
function renderMenu({
  onSelect = () => {},
  defaultOpen,
}: { onSelect?: (label: string) => void; defaultOpen?: boolean } = {}) {
  render(
    <>
      <button type="button">Before</button>
      <Menu
        label="Account"
        icon={<MoreIcon />}
        actions={labels.map((label) => ({
          label,
          onSelect: () => onSelect(label),
        }))}
        defaultOpen={defaultOpen}
      />
      <button type="button">After</button>
    </>,
  );
}

const button = () => screen.getByRole("button", { name: "Account" });
const menu = () => screen.queryByRole("menu", { name: "Account" });
const focused = () => document.activeElement?.textContent;

test("a closed button is named and has a menu to open", () => {
  renderMenu();

  expect(button().getAttribute("aria-haspopup")).toBe("menu");
  expect(button().getAttribute("aria-expanded")).toBe("false");
  expect(button().hasAttribute("aria-controls")).toBe(false);
  expect(menu()).toBeNull();
});

test("a click opens the menu with the focus on the first action", async () => {
  renderMenu();

  await userEvent.setup().click(button());
  expect(button().getAttribute("aria-expanded")).toBe("true");
  expect(button().getAttribute("aria-controls")).toBe(menu()!.id);
  expect(
    screen
      .getAllByRole("menuitem")
      .map((item) => [item.textContent, item.tabIndex]),
  ).toEqual(labels.map((label) => [label, -1]));
  expect(focused()).toBe("Sign-in methods");
});

test.each([
  ["{Enter}", "Sign-in methods"],
  [" ", "Sign-in methods"],
  ["{ArrowDown}", "Sign-in methods"],
  ["{ArrowUp}", "Sign out"],
])("%s on the button opens the menu on %s", async (key, first) => {
  const user = userEvent.setup();
  renderMenu();

  act(() => button().focus());
  await user.keyboard(key);
  expect(button().getAttribute("aria-expanded")).toBe("true");
  expect(focused()).toBe(first);
});

test("arrows move around the menu, Home and End go to its ends", async () => {
  const user = userEvent.setup();
  renderMenu();

  act(() => button().focus());
  await user.keyboard("{Enter}");
  const moves: [string, string][] = [
    ["{ArrowDown}", "Linked devices"],
    ["{ArrowDown}", "Sign out"],
    ["{ArrowDown}", "Sign-in methods"],
    ["{ArrowUp}", "Sign out"],
    ["{Home}", "Sign-in methods"],
    ["{End}", "Sign out"],
  ];
  for (const [key, action] of moves) {
    await user.keyboard(key);
    expect(focused(), key).toBe(action);
  }
});

test.each(["{Enter}", " "])(
  "%s chooses the focused action, closes the menu and returns the focus",
  async (key) => {
    const user = userEvent.setup();
    const onSelect = vi.fn();
    renderMenu({ onSelect });

    act(() => button().focus());
    await user.keyboard("{Enter}");
    await user.keyboard("{ArrowDown}");
    await user.keyboard(key);
    expect(onSelect.mock.calls).toEqual([["Linked devices"]]);
    expect(menu()).toBeNull();
    expect(button().getAttribute("aria-expanded")).toBe("false");
    expect(document.activeElement).toBe(button());
  },
);

test("a click on an action chooses it", async () => {
  const user = userEvent.setup();
  const onSelect = vi.fn();
  renderMenu({ onSelect });

  await user.click(button());
  await user.click(screen.getByRole("menuitem", { name: "Sign out" }));
  expect(onSelect.mock.calls).toEqual([["Sign out"]]);
  expect(menu()).toBeNull();
  expect(document.activeElement).toBe(button());
});

test("Escape closes only the menu without a choice and returns the focus", async () => {
  const user = userEvent.setup();
  const onSelect = vi.fn();
  const outer = vi.fn();
  document.addEventListener("keydown", outer);
  renderMenu({ onSelect });

  try {
    act(() => button().focus());
    await user.keyboard("{Enter}");
    await user.keyboard("{Escape}");
    expect(menu()).toBeNull();
    expect(document.activeElement).toBe(button());
    expect(onSelect).not.toHaveBeenCalled();
    // A side menu around it, closed by Escape on the document, stays open.
    expect(outer.mock.calls.map(([event]) => event.key)).toEqual(["Enter"]);
  } finally {
    document.removeEventListener("keydown", outer);
  }
});

test("Tab closes the menu without a choice and goes on from the button", async () => {
  const user = userEvent.setup();
  const onSelect = vi.fn();
  renderMenu({ onSelect });

  act(() => button().focus());
  await user.keyboard("{Enter}");
  await user.tab();
  expect(menu()).toBeNull();
  expect(focused()).toBe("After");
  expect(onSelect).not.toHaveBeenCalled();
});

test("Shift+Tab closes the menu without a choice and goes back from the button", async () => {
  const user = userEvent.setup();
  const onSelect = vi.fn();
  renderMenu({ onSelect });

  act(() => button().focus());
  await user.keyboard("{Enter}");
  await user.tab({ shift: true });
  expect(menu()).toBeNull();
  expect(focused()).toBe("Before");
  expect(onSelect).not.toHaveBeenCalled();
});

test("the focus leaving for an outside element closes the menu without a choice", async () => {
  const user = userEvent.setup();
  const onSelect = vi.fn();
  renderMenu({ onSelect });

  act(() => button().focus());
  await user.keyboard("{Enter}");
  const after = screen.getByRole("button", { name: "After" });
  act(() => after.focus());
  expect(menu()).toBeNull();
  expect(button().getAttribute("aria-expanded")).toBe("false");
  expect(document.activeElement).toBe(after);
  expect(onSelect).not.toHaveBeenCalled();
});

test("a click outside or on the button closes the menu without a choice", async () => {
  const user = userEvent.setup();
  const onSelect = vi.fn();
  renderMenu({ onSelect });

  await user.click(button());
  await user.click(screen.getByRole("button", { name: "Before" }));
  expect(menu()).toBeNull();
  await user.click(button());
  await user.click(button());
  expect(menu()).toBeNull();
  expect(onSelect).not.toHaveBeenCalled();
});

test("a press outside closes the menu even if nothing takes the focus", () => {
  renderMenu({ defaultOpen: true });

  fireEvent.pointerDown(document.body);
  expect(menu()).toBeNull();
});

test("a menu open from the first render leaves the focus where it is", () => {
  renderMenu({ defaultOpen: true });

  expect(menu()).not.toBeNull();
  expect(button().getAttribute("aria-expanded")).toBe("true");
  expect(document.activeElement).toBe(document.body);
});

test.each([
  ["{ArrowDown}", "Sign-in methods"],
  ["{ArrowUp}", "Sign out"],
])(
  "%s on the button of a menu open from the first render focuses %s",
  async (key, action) => {
    const user = userEvent.setup();
    renderMenu({ defaultOpen: true });

    act(() => button().focus());
    await user.keyboard(key);
    expect(menu()).not.toBeNull();
    expect(focused()).toBe(action);
  },
);
