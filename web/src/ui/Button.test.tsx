import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, test, vi } from "vitest";
import { Button } from "./Button.tsx";

afterEach(cleanup);

/** Presses the focused element with each activation input: a click, Enter and Space. */
async function pressAllWays(button: HTMLElement) {
  const user = userEvent.setup();
  await user.click(button);
  button.focus();
  await user.keyboard("{Enter}");
  await user.keyboard(" ");
}

test("acts on a click, Enter and Space", async () => {
  const onClick = vi.fn();
  render(<Button onClick={onClick}>Approve</Button>);

  await pressAllWays(screen.getByRole("button", { name: "Approve" }));
  expect(onClick).toHaveBeenCalledTimes(3);
});

test("is reached with Tab", async () => {
  render(<Button>Approve</Button>);

  await userEvent.setup().tab();
  expect(document.activeElement).toBe(
    screen.getByRole("button", { name: "Approve" }),
  );
});

test("does not act or take focus while disabled", async () => {
  const onClick = vi.fn();
  render(
    <Button disabled onClick={onClick}>
      Approve
    </Button>,
  );

  const button = screen.getByRole("button", { name: "Approve" });
  await pressAllWays(button);
  expect(onClick).not.toHaveBeenCalled();
  await userEvent.setup().tab();
  expect(document.activeElement).toBe(document.body);
});

test("does not act while busy and keeps the focus", async () => {
  const onClick = vi.fn();
  render(
    <Button busy onClick={onClick}>
      Approve
    </Button>,
  );

  const button = screen.getByRole("button", { name: "Approve" });
  await pressAllWays(button);
  expect(onClick).not.toHaveBeenCalled();
  expect(document.activeElement).toBe(button);
  expect(button.getAttribute("aria-disabled")).toBe("true");
  expect(button.getAttribute("aria-busy")).toBe("true");
  // A spinner shows it; a screen reader skips the spinner.
  expect(
    [...button.children].map((child) => child.getAttribute("aria-hidden")),
  ).toEqual(["true"]);
});

test("acts again once it is no longer busy", async () => {
  const onClick = vi.fn();
  const { rerender } = render(
    <Button busy onClick={onClick}>
      Approve
    </Button>,
  );
  rerender(
    <Button busy={false} onClick={onClick}>
      Approve
    </Button>,
  );

  const button = screen.getByRole("button", { name: "Approve" });
  await pressAllWays(button);
  expect(onClick).toHaveBeenCalledTimes(3);
  expect(button.hasAttribute("aria-disabled")).toBe(false);
  expect(button.hasAttribute("aria-busy")).toBe(false);
  expect(button.children).toHaveLength(0);
});

test("announces the busy state in a live region present before it", () => {
  const { rerender } = render(<Button busy={false}>Approve</Button>);
  const status = screen.getByRole("status");
  expect(status.textContent).toBe("");

  rerender(<Button busy>Approve</Button>);
  expect(screen.getByRole("status")).toBe(status);
  expect(status.textContent).toBe("Busy");

  rerender(
    <Button busy busyLabel="Approving the device">
      Approve
    </Button>,
  );
  expect(status.textContent).toBe("Approving the device");

  rerender(<Button busy={false}>Approve</Button>);
  expect(status.textContent).toBe("");
  // The announcement is not part of the button's name.
  expect(screen.getByRole("button").textContent).toBe("Approve");
});

test("has no live region when it never turns busy", () => {
  render(<Button>Approve</Button>);
  expect(screen.queryByRole("status")).toBeNull();
});

test("does not submit its form unless it is a submit button", async () => {
  const user = userEvent.setup();
  const onSubmit = vi.fn((event: SubmitEvent) => event.preventDefault());
  const { rerender } = render(
    <form onSubmit={(event) => onSubmit(event.nativeEvent as SubmitEvent)}>
      <Button>Approve</Button>
    </form>,
  );
  await user.click(screen.getByRole("button"));
  expect(onSubmit).not.toHaveBeenCalled();

  rerender(
    <form onSubmit={(event) => onSubmit(event.nativeEvent as SubmitEvent)}>
      <Button type="submit">Approve</Button>
    </form>,
  );
  await user.click(screen.getByRole("button"));
  expect(onSubmit).toHaveBeenCalledTimes(1);
});

test("does not submit its form while busy", async () => {
  const onSubmit = vi.fn((event: SubmitEvent) => event.preventDefault());
  render(
    <form onSubmit={(event) => onSubmit(event.nativeEvent as SubmitEvent)}>
      <Button type="submit" busy>
        Approve
      </Button>
    </form>,
  );

  await pressAllWays(screen.getByRole("button"));
  expect(onSubmit).not.toHaveBeenCalled();
});
