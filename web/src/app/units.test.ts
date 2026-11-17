import { expect, test } from "vitest";
import { period, size } from "./units.ts";

test("a size is in the largest unit it fills, to a tenth", () => {
  expect(
    [1, 900, 1024, 64 * 1024, 1536 * 1024, 5 * 1024 ** 3, 2048 * 1024 ** 4].map(
      size,
    ),
  ).toEqual([
    "1 byte",
    "900 bytes",
    "1 KB",
    "64 KB",
    "1.5 MB",
    "5 GB",
    "2,048 TB",
  ]);
});

test("a period is in the largest whole unit", () => {
  expect(
    [1, 90, 120, 3_600, 43_200, 86_400, 7 * 86_400, 100 * 365 * 86_400].map(
      period,
    ),
  ).toEqual([
    "1 second",
    "90 seconds",
    "2 minutes",
    "1 hour",
    "12 hours",
    "1 day",
    "7 days",
    "36,500 days",
  ]);
});
