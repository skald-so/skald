// Sizes and periods as the Usage section shows the server's limits.

/** `count` of `unit`, with an "s" unless it is 1: "1 day", "7 days". */
function plural(count: number, unit: string): string {
  return `${count.toLocaleString("en-US")} ${unit}${count === 1 ? "" : "s"}`;
}

/** A size in bytes in the largest unit it fills, to a tenth: "64 KB", "1.5 MB", "900 bytes". */
export function size(bytes: number): string {
  const units = ["KB", "MB", "GB", "TB"];
  if (bytes < 1024) return plural(bytes, "byte");
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${Number(value.toFixed(1)).toLocaleString("en-US")} ${units[unit]}`;
}

/** A period in seconds in the largest whole unit: "7 days", "12 hours", "90 seconds". */
export function period(seconds: number): string {
  const units: [number, string][] = [
    [86_400, "day"],
    [3_600, "hour"],
    [60, "minute"],
  ];
  for (const [length, unit] of units)
    if (seconds % length === 0) return plural(seconds / length, unit);
  return plural(seconds, "second");
}
