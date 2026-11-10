import { expect, test } from "vitest";
import { nextSort, sortRows, type Sort } from "./sort.ts";

test.each<[Sort | undefined, string, Sort]>([
  [undefined, "name", { column: "name", direction: "ascending" }],
  [
    { column: "seen", direction: "descending" },
    "name",
    { column: "name", direction: "ascending" },
  ],
  [
    { column: "name", direction: "ascending" },
    "name",
    { column: "name", direction: "descending" },
  ],
  [
    { column: "name", direction: "descending" },
    "name",
    { column: "name", direction: "ascending" },
  ],
])("after %j a click on %s gives %j", (sort, column, next) => {
  expect(nextSort(sort, column)).toEqual(next);
});

interface Row {
  name: string;
  count: number;
}

const rows: Row[] = [
  { name: "build-10", count: 10 },
  { name: "Build-2", count: 2 },
  { name: "alpha", count: 10 },
  { name: "build-1", count: 9 },
];

const value = (row: Row, column: string) =>
  column === "name" ? row.name : row.count;

const names = (sorted: Row[]) => sorted.map((row) => row.name);

test("numbers sort as numbers in both directions", () => {
  expect(
    names(sortRows(rows, { column: "count", direction: "ascending" }, value)),
  ).toEqual(["Build-2", "build-1", "build-10", "alpha"]);
  expect(
    names(sortRows(rows, { column: "count", direction: "descending" }, value)),
  ).toEqual(["build-10", "alpha", "build-1", "Build-2"]);
});

test("text sorts ignoring case, with digit runs as numbers", () => {
  expect(
    names(sortRows(rows, { column: "name", direction: "ascending" }, value)),
  ).toEqual(["alpha", "build-1", "Build-2", "build-10"]);
  expect(
    names(sortRows(rows, { column: "name", direction: "descending" }, value)),
  ).toEqual(["build-10", "Build-2", "build-1", "alpha"]);
});

test("rows with equal values keep their order in both directions", () => {
  const tied = sortRows(
    rows,
    { column: "count", direction: "descending" },
    value,
  );
  expect(names(tied).slice(0, 2)).toEqual(["build-10", "alpha"]);
  const sameName = [
    { name: "a", count: 1 },
    { name: "A", count: 2 },
  ];
  expect(
    sortRows(sameName, { column: "name", direction: "descending" }, value),
  ).toEqual(sameName);
});

test("returns a copy and leaves the rows as they were", () => {
  const before = [...rows];
  const sorted = sortRows(
    rows,
    { column: "count", direction: "ascending" },
    value,
  );
  expect(sorted).not.toBe(rows);
  expect(rows).toEqual(before);
});

test("asks for the value of the sorted column", () => {
  const columns: string[] = [];
  sortRows(rows, { column: "count", direction: "ascending" }, (row, column) => {
    columns.push(column);
    return row.count;
  });
  expect(new Set(columns)).toEqual(new Set(["count"]));
});
