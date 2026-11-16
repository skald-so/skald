// Times as the cabinet's tables show them: relative to now, in whole minutes, hours or days.

const minute = 60_000;
const hour = 60 * minute;
const day = 24 * hour;

/** `count` of `unit`, as "1 day" or "3 days"; hours and minutes as "3 h" and "5 min". */
function amount(count: number, unit: "day" | "h" | "min"): string {
  if (unit !== "day") return `${count} ${unit}`;
  return count === 1 ? "1 day" : `${count} days`;
}

/** How long ago `at` was at `now`, in milliseconds since the epoch: "Now", "5 min ago", "3 h ago". */
export function ago(at: string, now: number): string {
  const past = now - Date.parse(at);
  if (past < minute) return "Now";
  if (past < hour) return `${amount(Math.floor(past / minute), "min")} ago`;
  if (past < day) return `${amount(Math.floor(past / hour), "h")} ago`;
  return `${amount(Math.floor(past / day), "day")} ago`;
}

/** How soon `at` comes after `now`, to the nearest unit: "in 30 days", "in 5 h", "in 10 min". */
export function until(at: string, now: number): string {
  const left = Date.parse(at) - now;
  if (left >= day) return `in ${amount(Math.round(left / day), "day")}`;
  if (left >= hour) return `in ${amount(Math.round(left / hour), "h")}`;
  return `in ${amount(Math.max(1, Math.round(left / minute)), "min")}`;
}

/** The day of `at`: "Nov 10", with the year when it is not that of `now`: "Nov 10, 2025". */
export function date(at: string, now: number): string {
  const when = new Date(at);
  const sameYear = when.getFullYear() === new Date(now).getFullYear();
  return when.toLocaleDateString("en-US", {
    month: "short",
    day: "numeric",
    year: sameYear ? undefined : "numeric",
  });
}
