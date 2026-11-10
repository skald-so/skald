import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, test, vi } from "vitest";
import { Button } from "./Button.tsx";
import { EmptyState } from "./EmptyState.tsx";
import { Loading } from "./Loading.tsx";

afterEach(cleanup);

test("the empty state shows its message and its action acts", async () => {
  const connect = vi.fn();
  render(
    <EmptyState action={<Button onClick={connect}>Connect a machine</Button>}>
      <strong>No machines yet.</strong> Connect one.
    </EmptyState>,
  );

  expect(screen.getByText(/Connect one\./).textContent).toBe(
    "No machines yet. Connect one.",
  );
  await userEvent
    .setup()
    .click(screen.getByRole("button", { name: "Connect a machine" }));
  expect(connect).toHaveBeenCalledTimes(1);
});

test("the empty state without an action has no buttons", () => {
  render(<EmptyState>No tasks are running.</EmptyState>);

  expect(screen.getByText("No tasks are running.")).toBeDefined();
  expect(screen.queryByRole("button")).toBeNull();
});

test("a screen reader hears the loading state as busy, with its label", () => {
  const { container } = render(<Loading label="Loading tasks" lines={2} />);

  const placeholder = container.firstElementChild!;
  expect(placeholder.getAttribute("aria-busy")).toBe("true");
  expect(screen.getByRole("status").textContent).toBe("Loading tasks");
  // The lines are for the eye only.
  const lines = [...placeholder.children].filter(
    (child) => child.getAttribute("role") !== "status",
  );
  expect(lines.map((line) => line.getAttribute("aria-hidden"))).toEqual([
    "true",
    "true",
  ]);
  expect(placeholder.textContent).toBe("Loading tasks");
});

test("the loading state says Loading unless labelled", () => {
  render(<Loading />);

  expect(screen.getByRole("status").textContent).toBe("Loading");
});
