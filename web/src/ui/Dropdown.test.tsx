import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { afterEach, expect, test, vi } from "vitest";
import { Dropdown, type DropdownOption } from "./Dropdown.tsx";

afterEach(cleanup);

type From = "anyone" | "claude-code" | "codex";

const options: DropdownOption<From>[] = [
  { value: "anyone", label: "anyone" },
  { value: "claude-code", label: "claude-code" },
  { value: "codex", label: "codex" },
];

/** A screen that keeps the value and reports each choice to `onChange`. */
function Screen({
  initial = "anyone",
  onChange = () => {},
}: {
  initial?: From;
  onChange?: (value: From) => void;
}) {
  const [value, setValue] = useState<From>(initial);
  return (
    <>
      <button type="button">Before</button>
      <Dropdown
        label="From"
        options={options}
        value={value}
        defaultValue="anyone"
        onChange={(next) => {
          onChange(next);
          setValue(next);
        }}
      />
      <button type="button">Elsewhere</button>
    </>
  );
}

const button = () => screen.getByRole("button", { name: /^From / });
const list = () => screen.queryByRole("listbox", { name: "From" });
/** The options with whether each is the chosen one. */
const shown = () =>
  within(list()!)
    .getAllByRole("option")
    .map((option) => [
      option.textContent,
      option.getAttribute("aria-selected"),
    ]);
/** The label of the option the arrows are on. */
const active = () =>
  document.getElementById(list()!.getAttribute("aria-activedescendant")!)
    ?.textContent;

test("a closed button names the filter and its value, and has a list to open", () => {
  render(<Screen />);

  expect(button().textContent).toBe("From anyone");
  expect(button().getAttribute("aria-haspopup")).toBe("listbox");
  expect(button().getAttribute("aria-expanded")).toBe("false");
  expect(button().hasAttribute("data-set")).toBe(false);
  expect(list()).toBeNull();
});

test("a click opens the list with the current value chosen and the focus in it", async () => {
  render(<Screen initial="codex" />);

  await userEvent.setup().click(button());
  expect(button().getAttribute("aria-expanded")).toBe("true");
  expect(button().getAttribute("aria-controls")).toBe(list()!.id);
  expect(shown()).toEqual([
    ["anyone", "false"],
    ["claude-code", "false"],
    ["codex", "true"],
  ]);
  expect(active()).toBe("codex");
  expect(document.activeElement).toBe(list());
});

test("a click on an option reports it, shows it on the button and closes the list", async () => {
  const user = userEvent.setup();
  const onChange = vi.fn();
  render(<Screen onChange={onChange} />);

  await user.click(button());
  await user.click(screen.getByRole("option", { name: "codex" }));
  expect(onChange.mock.calls).toEqual([["codex"]]);
  expect(button().textContent).toBe("From codex");
  expect(button().getAttribute("aria-expanded")).toBe("false");
  expect(list()).toBeNull();
  expect(document.activeElement).toBe(button());
});

test("a value other than the default is highlighted", async () => {
  const user = userEvent.setup();
  render(<Screen />);

  await user.click(button());
  await user.click(screen.getByRole("option", { name: "claude-code" }));
  expect(button().hasAttribute("data-set")).toBe(true);
  await user.click(button());
  await user.click(screen.getByRole("option", { name: "anyone" }));
  expect(button().hasAttribute("data-set")).toBe(false);
});

test("choosing the current value reports nothing", async () => {
  const user = userEvent.setup();
  const onChange = vi.fn();
  render(<Screen onChange={onChange} />);

  await user.click(button());
  await user.click(screen.getByRole("option", { name: "anyone" }));
  await user.click(button());
  await user.keyboard("{Enter}");
  expect(onChange).not.toHaveBeenCalled();
  expect(list()).toBeNull();
});

test.each(["{Enter}", " ", "{ArrowDown}", "{ArrowUp}"])(
  "%s on the button opens the list",
  async (key) => {
    const user = userEvent.setup();
    render(<Screen initial="claude-code" />);

    act(() => button().focus());
    await user.keyboard(key);
    expect(button().getAttribute("aria-expanded")).toBe("true");
    expect(active()).toBe("claude-code");
    expect(document.activeElement).toBe(list());
  },
);

test("other keys on the button do not open the list", async () => {
  const user = userEvent.setup();
  render(<Screen />);

  act(() => button().focus());
  await user.keyboard("{ArrowLeft}{ArrowRight}a");
  expect(list()).toBeNull();
});

test("arrows, Home and End move in the list and Enter chooses", async () => {
  const user = userEvent.setup();
  const onChange = vi.fn();
  render(<Screen onChange={onChange} />);

  act(() => button().focus());
  await user.keyboard("{Enter}");
  expect(active()).toBe("anyone");
  await user.keyboard("{ArrowUp}");
  expect(active()).toBe("anyone");
  await user.keyboard("{ArrowDown}");
  expect(active()).toBe("claude-code");
  await user.keyboard("{ArrowDown}{ArrowDown}");
  expect(active()).toBe("codex");
  await user.keyboard("{Home}");
  expect(active()).toBe("anyone");
  await user.keyboard("{End}");
  expect(active()).toBe("codex");
  await user.keyboard("{ArrowUp}");
  expect(active()).toBe("claude-code");
  // Moving changes nothing until a choice.
  expect(onChange).not.toHaveBeenCalled();
  expect(button().textContent).toBe("From anyone");

  await user.keyboard("{Enter}");
  expect(onChange.mock.calls).toEqual([["claude-code"]]);
  expect(button().textContent).toBe("From claude-code");
  expect(list()).toBeNull();
  expect(document.activeElement).toBe(button());
});

test("Space chooses the active option", async () => {
  const user = userEvent.setup();
  const onChange = vi.fn();
  render(<Screen onChange={onChange} />);

  act(() => button().focus());
  await user.keyboard("{ArrowDown}{ArrowDown} ");
  expect(onChange.mock.calls).toEqual([["claude-code"]]);
  expect(document.activeElement).toBe(button());
});

test("Escape closes the list without a change and returns the focus to the button", async () => {
  const user = userEvent.setup();
  const onChange = vi.fn();
  const onOuterKey = vi.fn();
  render(
    <div onKeyDown={(event) => onOuterKey(event.key)}>
      <Screen onChange={onChange} />
    </div>,
  );

  act(() => button().focus());
  await user.keyboard("{Enter}{ArrowDown}{Escape}");
  expect(list()).toBeNull();
  expect(button().getAttribute("aria-expanded")).toBe("false");
  expect(onChange).not.toHaveBeenCalled();
  expect(button().textContent).toBe("From anyone");
  expect(document.activeElement).toBe(button());
  // Escape that closed the list does not reach the page around it, as a dialog.
  expect(onOuterKey).not.toHaveBeenCalledWith("Escape");
});

test("the next opening starts on the current value, not the last active option", async () => {
  const user = userEvent.setup();
  render(<Screen />);

  act(() => button().focus());
  await user.keyboard("{Enter}{ArrowDown}{ArrowDown}{Escape}{Enter}");
  expect(active()).toBe("anyone");
});

test("a click outside closes the list without a change", async () => {
  const user = userEvent.setup();
  const onChange = vi.fn();
  render(<Screen onChange={onChange} />);

  await user.click(button());
  await user.keyboard("{ArrowDown}");
  await user.click(document.body);
  expect(list()).toBeNull();
  expect(onChange).not.toHaveBeenCalled();
  expect(button().textContent).toBe("From anyone");
});

test("a click on the button closes an open list without a change", async () => {
  const user = userEvent.setup();
  const onChange = vi.fn();
  render(<Screen onChange={onChange} />);

  await user.click(button());
  await user.keyboard("{ArrowDown}");
  await user.click(button());
  expect(list()).toBeNull();
  expect(onChange).not.toHaveBeenCalled();
  expect(document.activeElement).toBe(button());
});

test("a press inside the list keeps it open", () => {
  render(<Screen />);

  fireEvent.click(button());
  fireEvent.pointerDown(list()!);
  expect(list()).not.toBeNull();
});

test("Tab closes the list without a change and goes on from the button", async () => {
  const user = userEvent.setup();
  const onChange = vi.fn();
  render(<Screen onChange={onChange} />);

  act(() => button().focus());
  await user.keyboard("{Enter}{ArrowDown}");
  await user.tab();
  expect(list()).toBeNull();
  expect(onChange).not.toHaveBeenCalled();
  expect(document.activeElement).toBe(
    screen.getByRole("button", { name: "Elsewhere" }),
  );
});

test("Shift+Tab closes the list and goes back past the button", async () => {
  const user = userEvent.setup();
  render(<Screen />);

  await user.click(button());
  await user.tab({ shift: true });
  expect(list()).toBeNull();
  expect(document.activeElement).toBe(
    screen.getByRole("button", { name: "Before" }),
  );
});

test("a press outside closes a list open from the first render", () => {
  render(
    <Dropdown
      label="From"
      options={options}
      value="codex"
      defaultValue="anyone"
      onChange={() => {}}
      defaultOpen
    />,
  );

  fireEvent.pointerDown(document.body);
  expect(list()).toBeNull();
  expect(button().getAttribute("aria-expanded")).toBe("false");
});

test("the focus moving out of the list closes it", async () => {
  render(<Screen />);

  await userEvent.setup().click(button());
  act(() => screen.getByRole("button", { name: "Elsewhere" }).focus());
  expect(list()).toBeNull();
});

test("the pointer over an option makes it active", async () => {
  const user = userEvent.setup();
  render(<Screen />);

  await user.click(button());
  await user.hover(screen.getByRole("option", { name: "codex" }));
  expect(active()).toBe("codex");
  await user.keyboard("{Enter}");
  expect(button().textContent).toBe("From codex");
});

test("open from the first render, the list does not take the focus", () => {
  render(
    <Dropdown
      label="From"
      options={options}
      value="codex"
      defaultValue="anyone"
      onChange={() => {}}
      defaultOpen
    />,
  );

  expect(button().getAttribute("aria-expanded")).toBe("true");
  expect(shown()[2]).toEqual(["codex", "true"]);
  expect(document.activeElement).toBe(document.body);
});

test("the button and options submit no form", async () => {
  const user = userEvent.setup();
  const onSubmit = vi.fn((event: SubmitEvent) => event.preventDefault());
  render(
    <form onSubmit={(event) => onSubmit(event.nativeEvent as SubmitEvent)}>
      <Screen />
    </form>,
  );

  await user.click(button());
  await user.keyboard("{ArrowDown}{Enter}");
  expect(button().textContent).toBe("From claude-code");
  expect(onSubmit).not.toHaveBeenCalled();
});
