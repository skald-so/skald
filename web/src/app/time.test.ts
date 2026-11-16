import { expect, test } from "vitest";
import { ago, date, until } from "./time.ts";

const now = Date.parse("2026-11-16T12:00:00Z");
const at = (offset: number) => new Date(now + offset * 60_000).toISOString();

test("ago gives now, then minutes, hours and days, rounded down", () => {
  expect(
    [0, -0.5, -1, -59, -60, -179, -1439, -1440, -2879, -2880].map((offset) =>
      ago(at(offset), now),
    ),
  ).toEqual([
    "Now",
    "Now",
    "1 min ago",
    "59 min ago",
    "1 h ago",
    "2 h ago",
    "23 h ago",
    "1 day ago",
    "1 day ago",
    "2 days ago",
  ]);
});

test("until gives days, hours or minutes, to the nearest", () => {
  expect(
    [30 * 1440 - 1, 12 * 1440 + 60, 1440, 1439, 90, 60, 59, 0.2].map((offset) =>
      until(at(offset), now),
    ),
  ).toEqual([
    "in 30 days",
    "in 12 days",
    "in 1 day",
    "in 24 h",
    "in 2 h",
    "in 1 h",
    "in 59 min",
    "in 1 min",
  ]);
});

test("date gives the month and day, and the year only when it is another", () => {
  expect(date("2026-11-10T12:00:00Z", now)).toBe("Nov 10");
  expect(date("2025-10-02T12:00:00Z", now)).toBe("Oct 2, 2025");
});
