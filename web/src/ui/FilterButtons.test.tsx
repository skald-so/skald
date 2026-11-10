import { cleanup, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { afterEach, expect, test, vi } from "vitest";
import { FilterButtons, type FilterOption } from "./FilterButtons.tsx";

afterEach(cleanup);

type Filter = "all" | "flagged" | "banned";

const options: FilterOption<Filter>[] = [
  { value: "all", label: "All" },
  { value: "flagged", label: "Flagged" },
  { value: "banned", label: "Banned" },
];

/** A screen that keeps the choice and reports each one to `onChange`. */
function Screen({
  onChange,
  joined,
}: {
  onChange: (value: Filter) => void;
  joined?: boolean;
}) {
  const [value, setValue] = useState<Filter>("all");
  return (
    <FilterButtons
      label="Filter accounts"
      options={options}
      value={value}
      onChange={(next) => {
        onChange(next);
        setValue(next);
      }}
      joined={joined}
    />
  );
}

/** Each choice of the group with whether a screen reader hears it as pressed. */
function pressed(): [string, string | null][] {
  return within(screen.getByRole("group", { name: "Filter accounts" }))
    .getAllByRole("button")
    .map((button) => [
      button.textContent!,
      button.getAttribute("aria-pressed"),
    ]);
}

test("names the group and marks only the active choice as pressed", () => {
  render(
    <FilterButtons
      label="Filter accounts"
      options={options}
      value="flagged"
      onChange={() => {}}
    />,
  );

  expect(pressed()).toEqual([
    ["All", "false"],
    ["Flagged", "true"],
    ["Banned", "false"],
  ]);
  expect(
    screen.getByRole("button", { name: "Flagged", pressed: true }),
  ).toBeTruthy();
});

test.each([false, true])(
  "a click makes a choice the only active one and reports it, joined: %s",
  async (joined) => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render(<Screen onChange={onChange} joined={joined} />);

    await user.click(screen.getByRole("button", { name: "Banned" }));
    expect(onChange.mock.calls).toEqual([["banned"]]);
    expect(pressed()).toEqual([
      ["All", "false"],
      ["Flagged", "false"],
      ["Banned", "true"],
    ]);

    await user.click(screen.getByRole("button", { name: "Flagged" }));
    expect(onChange.mock.calls).toEqual([["banned"], ["flagged"]]);
    expect(pressed()).toEqual([
      ["All", "false"],
      ["Flagged", "true"],
      ["Banned", "false"],
    ]);
  },
);

test("Tab reaches every choice, and Enter and Space make one active", async () => {
  const user = userEvent.setup();
  const onChange = vi.fn();
  render(<Screen onChange={onChange} />);

  await user.tab();
  expect(document.activeElement?.textContent).toBe("All");
  await user.tab();
  expect(document.activeElement?.textContent).toBe("Flagged");
  await user.keyboard("{Enter}");
  expect(onChange.mock.calls).toEqual([["flagged"]]);
  expect(pressed()[1]).toEqual(["Flagged", "true"]);

  await user.tab();
  expect(document.activeElement?.textContent).toBe("Banned");
  await user.keyboard(" ");
  expect(onChange.mock.calls).toEqual([["flagged"], ["banned"]]);
  expect(pressed()).toEqual([
    ["All", "false"],
    ["Flagged", "false"],
    ["Banned", "true"],
  ]);
  expect(document.activeElement?.textContent).toBe("Banned");
});

test("a press on the active choice reports nothing", async () => {
  const user = userEvent.setup();
  const onChange = vi.fn();
  render(<Screen onChange={onChange} />);

  const all = screen.getByRole("button", { name: "All" });
  await user.click(all);
  all.focus();
  await user.keyboard("{Enter} ");
  expect(onChange).not.toHaveBeenCalled();
  expect(pressed()[0]).toEqual(["All", "true"]);
});

test("the choices are buttons that submit no form", async () => {
  const onSubmit = vi.fn((event: SubmitEvent) => event.preventDefault());
  render(
    <form onSubmit={(event) => onSubmit(event.nativeEvent as SubmitEvent)}>
      <Screen onChange={() => {}} />
    </form>,
  );

  await userEvent.setup().click(screen.getByRole("button", { name: "Banned" }));
  expect(onSubmit).not.toHaveBeenCalled();
});
