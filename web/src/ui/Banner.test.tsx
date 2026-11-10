import { cleanup, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { afterEach, expect, test, vi } from "vitest";
import { Banner } from "./Banner.tsx";
import { LoopIcon } from "./icons.tsx";

afterEach(cleanup);

test("a screen reader gets the text and the actions of the banner", async () => {
  const pause = vi.fn();
  const open = vi.fn();
  render(
    <Banner
      icon={<LoopIcon />}
      actions={[
        { label: "Pause thread", onClick: pause },
        { label: "Open the thread", onClick: open },
      ]}
    >
      <strong>14 messages between agents</strong> in this thread without a
      human.
    </Banner>,
  );

  const banner = within(screen.getByRole("alert"));
  expect(screen.getByRole("alert").textContent).toBe(
    "14 messages between agents in this thread without a human.Pause threadOpen the thread",
  );
  // The icon is decoration.
  expect(
    screen.getByRole("alert").querySelector("svg")!.closest("[aria-hidden]"),
  ).not.toBeNull();

  await userEvent
    .setup()
    .click(banner.getByRole("button", { name: "Pause thread" }));
  expect(pause).toHaveBeenCalledTimes(1);
  expect(open).not.toHaveBeenCalled();
});

test("a banner without actions has no buttons", () => {
  render(<Banner>The recipient is offline.</Banner>);

  expect(screen.getByRole("alert").textContent).toBe(
    "The recipient is offline.",
  );
  expect(screen.queryByRole("button")).toBeNull();
});

test("a busy action does not act and is announced inside the banner", async () => {
  const pause = vi.fn();
  render(
    <Banner
      actions={[
        {
          label: "Pause thread",
          onClick: pause,
          busy: true,
          busyLabel: "Pausing",
        },
      ]}
    >
      Loop.
    </Banner>,
  );

  const banner = within(screen.getByRole("alert"));
  await userEvent
    .setup()
    .click(banner.getByRole("button", { name: "Pause thread" }));
  expect(pause).not.toHaveBeenCalled();
  expect(banner.getByRole("status").textContent).toBe("Pausing");
});

test("the banner appears as an alert with its text and actions in it", async () => {
  function Screen() {
    const [shown, setShown] = useState(false);
    return (
      <>
        <button onClick={() => setShown(true)}>Check</button>
        {shown && (
          <Banner actions={[{ label: "Pause thread" }]}>Agents loop.</Banner>
        )}
      </>
    );
  }
  render(<Screen />);
  expect(screen.queryByRole("alert")).toBeNull();

  // A screen reader announces an alert when it is added, with what it holds at that moment.
  const added: Node[] = [];
  const observer = new MutationObserver((records) =>
    records.forEach((record) => added.push(...record.addedNodes)),
  );
  observer.observe(document.body, { childList: true, subtree: true });
  await userEvent.setup().click(screen.getByRole("button", { name: "Check" }));
  observer.takeRecords().forEach((record) => added.push(...record.addedNodes));
  observer.disconnect();

  const alert = screen.getByRole("alert");
  expect(added).toEqual([alert]);
  expect(alert.textContent).toBe("Agents loop.Pause thread");
});
