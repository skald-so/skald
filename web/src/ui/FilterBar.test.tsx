import { cleanup, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { afterEach, expect, test, vi } from "vitest";
import { Dropdown, type DropdownOption } from "./Dropdown.tsx";
import { FilterBar } from "./FilterBar.tsx";

afterEach(cleanup);

type Status = "any" | "read" | "stored";
type Order = "oldest" | "newest";

const statuses: DropdownOption<Status>[] = [
  { value: "any", label: "any" },
  { value: "read", label: "read" },
  { value: "stored", label: "stored" },
];

const orders: DropdownOption<Order>[] = [
  { value: "oldest", label: "Oldest first" },
  { value: "newest", label: "Newest first" },
];

/** A screen with two filters and a sort order; `onClear` hears every press on "Clear". */
function Screen({ onClear = () => {} }: { onClear?: () => void }) {
  const [first, setFirst] = useState<Status>("any");
  const [second, setSecond] = useState<Status>("any");
  const [order, setOrder] = useState<Order>("oldest");
  return (
    <FilterBar
      label="Filter messages"
      filtered={first !== "any" || second !== "any"}
      onClear={() => {
        onClear();
        setFirst("any");
        setSecond("any");
      }}
      sort={
        <Dropdown
          label="Sort"
          options={orders}
          value={order}
          defaultValue="oldest"
          onChange={setOrder}
        />
      }
    >
      <Dropdown
        label="Status"
        options={statuses}
        value={first}
        defaultValue="any"
        onChange={setFirst}
      />
      <Dropdown
        label="Delivery"
        options={statuses}
        value={second}
        defaultValue="any"
        onChange={setSecond}
      />
    </FilterBar>
  );
}

const filters = () =>
  within(screen.getByRole("group", { name: "Filter messages" }));
const clear = () => screen.queryByRole("button", { name: "Clear" });
/** The text of every button in the bar. */
const buttons = () =>
  screen.getAllByRole("button").map((button) => button.textContent);

async function choose(name: RegExp, option: string) {
  const user = userEvent.setup();
  await user.click(screen.getByRole("button", { name }));
  await user.click(screen.getByRole("option", { name: option }));
}

test("names the filters and shows the sort order after them", () => {
  render(<Screen />);

  expect(
    filters()
      .getAllByRole("button")
      .map((button) => button.textContent),
  ).toEqual(["Status any", "Delivery any"]);
  expect(buttons()).toEqual([
    "Status any",
    "Delivery any",
    "Sort Oldest first",
  ]);
});

test("shows Clear only while a filter is set, after the filters", async () => {
  render(<Screen />);
  expect(clear()).toBeNull();

  await choose(/^Delivery /, "stored");
  expect(buttons()).toEqual([
    "Status any",
    "Delivery stored",
    "Clear",
    "Sort Oldest first",
  ]);
  await choose(/^Delivery /, "any");
  expect(clear()).toBeNull();
});

test("a sort order other than the default does not show Clear", async () => {
  render(<Screen />);

  await choose(/^Sort /, "Newest first");
  expect(clear()).toBeNull();
});

test("Clear restores the default filters, keeps the sort order and moves the focus to the first filter", async () => {
  const onClear = vi.fn();
  render(<Screen onClear={onClear} />);

  await choose(/^Status /, "read");
  await choose(/^Delivery /, "stored");
  await choose(/^Sort /, "Newest first");
  await userEvent.setup().click(clear()!);
  expect(onClear).toHaveBeenCalledTimes(1);
  expect(buttons()).toEqual([
    "Status any",
    "Delivery any",
    "Sort Newest first",
  ]);
  expect(document.activeElement).toBe(
    screen.getByRole("button", { name: "Status any" }),
  );
});

test("Clear works from the keyboard", async () => {
  const user = userEvent.setup();
  render(<Screen />);

  await choose(/^Status /, "read");
  await user.tab();
  expect(document.activeElement).toBe(
    screen.getByRole("button", { name: /^Delivery / }),
  );
  await user.tab();
  expect(document.activeElement).toBe(clear());
  await user.keyboard("{Enter}");
  expect(buttons()).toEqual([
    "Status any",
    "Delivery any",
    "Sort Oldest first",
  ]);
  expect(document.activeElement).toBe(
    screen.getByRole("button", { name: "Status any" }),
  );
});
