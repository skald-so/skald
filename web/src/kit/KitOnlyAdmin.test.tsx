// The kit page's Members view (?screen=members): the account `ana` is the only admin of `team-a`,
// with two members, so the view shows the only-admin warning, an alert that says the account is
// the only admin and that if it loses all its devices and its recovery key, nobody can manage the
// space, and names `skald make-admin`; it has no button.
import { cleanup, render, screen, within } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import { KitMembers } from "./KitApp.tsx";

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

test("the kit's Members view shows the only-admin warning with skald make-admin", () => {
  const fetch = vi.spyOn(globalThis, "fetch");
  render(<KitMembers />);

  const main = within(screen.getByRole("main"));
  const warnings = main
    .queryAllByRole("alert")
    .filter((alert) => alert.textContent?.includes("skald make-admin"));
  expect(warnings).toHaveLength(1);
  const warning = warnings[0]!;
  const text = (warning.textContent ?? "").toLowerCase();
  for (const part of ["only admin", "devices", "recovery key"])
    expect(text, `the warning says ${part}`).toContain(part);
  expect(within(warning).queryAllByRole("button")).toEqual([]);
  expect(main.getAllByRole("row").length).toBeGreaterThan(1);
  expect(fetch).not.toHaveBeenCalled();
});
