import { cleanup, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { afterEach, expect, test, vi } from "vitest";
import { EmptyState } from "./EmptyState.tsx";
import { sortRows, type Sort } from "./sort.ts";
import { Table, type Column, type TableProps } from "./Table.tsx";

afterEach(cleanup);

interface Device {
  name: string;
  minutes: number;
  key: string;
}

const devices: Device[] = [
  { name: "ana-laptop", minutes: 0, key: "7F3A" },
  { name: "ana-cloud", minutes: 180, key: "A06E" },
  { name: "Chrome", minutes: 5, key: "E2C7" },
];

const columns: Column<Device>[] = [
  { key: "name", label: "Device", sortable: true, cell: (d) => d.name },
  {
    key: "minutes",
    label: "Last seen",
    sortable: true,
    cell: (d) => `${d.minutes} min`,
  },
  { key: "key", label: "Key", cell: (d) => d.key },
  {
    key: "actions",
    label: "Actions",
    actions: true,
    cell: (d) => <button>Revoke {d.name}</button>,
  },
];

const empty = <EmptyState>No machines yet.</EmptyState>;

function renderTable(props: Partial<TableProps<Device>> = {}) {
  return render(
    <Table
      label="Devices"
      columns={columns}
      rows={devices}
      rowKey={(d) => d.name}
      empty={empty}
      {...props}
    />,
  );
}

/** A screen that sorts the rows in the browser, from `initial`. */
function Sorted({ initial }: { initial?: Sort }) {
  const [sort, setSort] = useState(initial);
  return (
    <Table
      label="Devices"
      columns={columns}
      rows={
        sort
          ? sortRows(devices, sort, (d, column) =>
              column === "name" ? d.name : d.minutes,
            )
          : devices
      }
      rowKey={(d) => d.name}
      sort={sort}
      onSort={setSort}
      empty={empty}
    />
  );
}

/** The first cell of every body row. */
function names(): string[] {
  const [, ...rows] = screen.getAllByRole("row");
  return rows.map((row) => within(row).getAllByRole("cell")[0]!.textContent!);
}

/** Each column header: its text with the arrow, and `aria-sort`. */
function headers(): [string, string | null][] {
  return screen
    .getAllByRole("columnheader")
    .map((header) => [header.textContent!, header.getAttribute("aria-sort")]);
}

test("a click on a header reports its column ascending, a click on the active one the other direction", async () => {
  const user = userEvent.setup();
  const onSort = vi.fn();
  const { rerender } = renderTable({
    sort: { column: "minutes", direction: "descending" },
    onSort,
  });

  await user.click(screen.getByRole("button", { name: "Device" }));
  expect(onSort).toHaveBeenLastCalledWith({
    column: "name",
    direction: "ascending",
  });
  await user.click(screen.getByRole("button", { name: "Last seen" }));
  expect(onSort).toHaveBeenLastCalledWith({
    column: "minutes",
    direction: "ascending",
  });

  rerender(
    <Table
      label="Devices"
      columns={columns}
      rows={devices}
      rowKey={(d) => d.name}
      empty={empty}
      sort={{ column: "minutes", direction: "ascending" }}
      onSort={onSort}
    />,
  );
  await user.click(screen.getByRole("button", { name: "Last seen" }));
  expect(onSort).toHaveBeenLastCalledWith({
    column: "minutes",
    direction: "descending",
  });
  expect(onSort).toHaveBeenCalledTimes(3);
});

test("the screen sets the initial order, shown by the arrow, aria-sort and the status", () => {
  renderTable({ sort: { column: "minutes", direction: "descending" } });

  expect(headers()).toEqual([
    ["Device↕", null],
    ["Last seen↓", "descending"],
    ["Key", null],
    ["Actions", null],
  ]);
  expect(screen.getByRole("status").textContent).toBe(
    "Sorted by Last seen, descending",
  );
  // The arrows are for the eye: the button is named by the column.
  expect(
    screen.getAllByRole("button", { name: /^(Device|Last seen)$/ }),
  ).toHaveLength(2);
});

test("without an order no column is marked as sorted", () => {
  renderTable();

  expect(headers()).toEqual([
    ["Device↕", null],
    ["Last seen↕", null],
    ["Key", null],
    ["Actions", null],
  ]);
  expect(screen.getByRole("status").textContent).toBe("");
});

test("clicks sort the rows in both directions, with the arrow and the announcement following", async () => {
  const user = userEvent.setup();
  render(<Sorted />);
  expect(names()).toEqual(["ana-laptop", "ana-cloud", "Chrome"]);

  await user.click(screen.getByRole("button", { name: "Device" }));
  expect(names()).toEqual(["ana-cloud", "ana-laptop", "Chrome"]);
  expect(headers().slice(0, 2)).toEqual([
    ["Device↑", "ascending"],
    ["Last seen↕", null],
  ]);
  expect(screen.getByRole("status").textContent).toBe(
    "Sorted by Device, ascending",
  );

  await user.click(screen.getByRole("button", { name: "Device" }));
  expect(names()).toEqual(["Chrome", "ana-laptop", "ana-cloud"]);
  expect(headers()[0]).toEqual(["Device↓", "descending"]);
  expect(screen.getByRole("status").textContent).toBe(
    "Sorted by Device, descending",
  );

  await user.click(screen.getByRole("button", { name: "Last seen" }));
  expect(names()).toEqual(["ana-laptop", "Chrome", "ana-cloud"]);
  expect(headers().slice(0, 2)).toEqual([
    ["Device↕", null],
    ["Last seen↑", "ascending"],
  ]);
});

test("headers sort from the keyboard with Enter and Space", async () => {
  const user = userEvent.setup();
  render(<Sorted initial={{ column: "minutes", direction: "ascending" }} />);

  await user.tab();
  expect(document.activeElement).toBe(
    screen.getByRole("button", { name: "Device" }),
  );
  await user.keyboard("{Enter}");
  expect(names()).toEqual(["ana-cloud", "ana-laptop", "Chrome"]);
  expect(headers()[0]).toEqual(["Device↑", "ascending"]);

  await user.keyboard(" ");
  expect(names()).toEqual(["Chrome", "ana-laptop", "ana-cloud"]);
  expect(headers()[0]).toEqual(["Device↓", "descending"]);
  expect(screen.getByRole("status").textContent).toBe(
    "Sorted by Device, descending",
  );

  await user.tab();
  await user.keyboard("{Enter}");
  expect(names()).toEqual(["ana-laptop", "Chrome", "ana-cloud"]);
  expect(headers()[1]).toEqual(["Last seen↑", "ascending"]);
});

test("a column that is not sortable has no button and does not sort", async () => {
  const user = userEvent.setup();
  const onSort = vi.fn();
  // Even an order naming it does not mark it.
  renderTable({ sort: { column: "key", direction: "ascending" }, onSort });

  const [, , key, actions] = screen.getAllByRole("columnheader");
  expect(within(key!).queryByRole("button")).toBeNull();
  expect(within(actions!).queryByRole("button")).toBeNull();
  await user.click(key!);
  await user.click(actions!);
  expect(onSort).not.toHaveBeenCalled();
  expect(headers().map(([, sort]) => sort)).toEqual([null, null, null, null]);
  expect(screen.getByRole("status").textContent).toBe("");
});

test("the empty state replaces the rows", () => {
  renderTable({ rows: [] });

  const [header, ...rows] = screen.getAllByRole("row");
  expect(within(header!).getAllByRole("columnheader")).toHaveLength(4);
  expect(rows).toHaveLength(1);
  const cell = within(rows[0]!).getByRole("cell");
  expect(cell.textContent).toBe("No machines yet.");
  expect(cell.getAttribute("colspan")).toBe("4");
});

test("the loading state replaces the rows and marks the table busy until it ends", () => {
  const { rerender } = renderTable({
    loading: true,
    loadingLabel: "Loading devices",
  });

  const table = screen.getByRole("table", { name: "Devices" });
  expect(table.getAttribute("aria-busy")).toBe("true");
  const [, ...rows] = screen.getAllByRole("row");
  expect(rows).toHaveLength(1);
  const cell = within(rows[0]!).getByRole("cell");
  expect(cell.getAttribute("colspan")).toBe("4");
  expect(within(cell).getByRole("status").textContent).toBe("Loading devices");
  expect(screen.queryByText("ana-laptop")).toBeNull();
  expect(screen.queryByText("No machines yet.")).toBeNull();

  rerender(
    <Table
      label="Devices"
      columns={columns}
      rows={devices}
      rowKey={(d) => d.name}
      empty={empty}
      loading={false}
    />,
  );
  expect(table.hasAttribute("aria-busy")).toBe(false);
  expect(names()).toEqual(["ana-laptop", "ana-cloud", "Chrome"]);
});

test("loading without rows shows the loading state, not the empty one", () => {
  renderTable({ rows: [], loading: true });

  expect(screen.queryByText("No machines yet.")).toBeNull();
  expect(screen.getByText("Loading")).toBeDefined();
});

test("rows show every cell in the column order, with the actions in the last", () => {
  renderTable();

  const [, first] = screen.getAllByRole("row");
  expect(
    within(first!)
      .getAllByRole("cell")
      .map((cell) => cell.textContent),
  ).toEqual(["ana-laptop", "0 min", "7F3A", "Revoke ana-laptop"]);
});
