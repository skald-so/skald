// The sort order of a table and a helper that sorts short lists in the browser.

export type SortDirection = "ascending" | "descending";

/** The order of a table: the key of a sortable column and the direction. */
export interface Sort {
  column: string;
  direction: SortDirection;
}

/** The order after a click on the header of `column`: the other direction if it is active, else ascending. */
export function nextSort(sort: Sort | undefined, column: string): Sort {
  if (sort?.column === column) {
    return {
      column,
      direction: sort.direction === "ascending" ? "descending" : "ascending",
    };
  }
  return { column, direction: "ascending" };
}

const collator = new Intl.Collator("en", {
  numeric: true,
  sensitivity: "base",
});

/**
 * A sorted copy of `rows`, for lists short enough to sort in the browser; the server sorts long ones.
 * `value` gives the value of a row in the sorted column: numbers compare as numbers, text
 * case-insensitively with digit runs as numbers. Rows with equal values keep their order.
 */
export function sortRows<Row>(
  rows: readonly Row[],
  sort: Sort,
  value: (row: Row, column: string) => string | number,
): Row[] {
  const sign = sort.direction === "ascending" ? 1 : -1;
  return rows
    .map((row) => ({ row, value: value(row, sort.column) }))
    .sort((a, b) => sign * compare(a.value, b.value))
    .map(({ row }) => row);
}

function compare(a: string | number, b: string | number): number {
  if (typeof a === "number" && typeof b === "number") return a - b;
  return collator.compare(String(a), String(b));
}
