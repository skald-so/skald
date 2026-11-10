import { cleanup, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, test, vi } from "vitest";
import { KitCabinet } from "./KitCabinet.tsx";

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

const heading = () =>
  within(screen.getByRole("main")).getByRole("heading", { level: 1 });
const current = () =>
  within(screen.getByRole("navigation", { name: "Sections" }))
    .getAllByRole("link")
    .filter((link) => link.getAttribute("aria-current") === "page")
    .map((link) => link.textContent);

test("shows the cabinet around Machines without network requests", () => {
  const fetch = vi.spyOn(globalThis, "fetch");
  render(<KitCabinet />);

  expect(heading().textContent).toBe("Machines");
  expect(current()).toEqual(["Machines"]);
  expect(screen.getByRole("button", { name: /^Space / }).textContent).toBe(
    "Space team-a",
  );
  const main = within(screen.getByRole("main"));
  expect(main.getByRole("button", { name: "Connect a machine" })).toBeTruthy();
  expect(main.getByRole("region", { name: "Connect a machine" })).toBeTruthy();
  expect(
    main.getByRole("region", { name: "New device waiting for approval" }),
  ).toBeTruthy();
  expect(main.getAllByRole("row")).toHaveLength(5);
  expect(fetch).not.toHaveBeenCalled();
});

test("choosing a section marks it current and shows its heading", async () => {
  const user = userEvent.setup();
  render(<KitCabinet />);

  await user.click(screen.getByRole("link", { name: "Members" }));
  expect(current()).toEqual(["Members"]);
  expect(heading().textContent).toBe("Members");
  expect(screen.queryByRole("table")).toBeNull();
  await user.click(screen.getByRole("link", { name: "Machines" }));
  expect(heading().textContent).toBe("Machines");
});

test("choosing a space shows it in the switcher", async () => {
  const user = userEvent.setup();
  render(<KitCabinet />);

  await user.click(screen.getByRole("button", { name: /^Space / }));
  await user.click(screen.getByRole("option", { name: "ana" }));
  expect(screen.getByRole("button", { name: /^Space / }).textContent).toBe(
    "Space ana",
  );
});
