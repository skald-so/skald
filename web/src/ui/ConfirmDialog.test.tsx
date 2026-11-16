import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { afterEach, expect, test, vi } from "vitest";
import { ConfirmDialog } from "./ConfirmDialog.tsx";

afterEach(cleanup);

/** A button that opens the dialog, which reports its outcome to `onConfirm` and `onCancel`. */
function Opener({
  onConfirm = () => {},
  onCancel = () => {},
}: {
  onConfirm?: () => void;
  onCancel?: () => void;
}) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <button type="button" onClick={() => setOpen(true)}>
        Open
      </button>
      {open && (
        <ConfirmDialog
          title="End it?"
          confirm="End it"
          onConfirm={() => {
            setOpen(false);
            onConfirm();
          }}
          onCancel={() => {
            setOpen(false);
            onCancel();
          }}
        >
          <p>It ends at once.</p>
        </ConfirmDialog>
      )}
    </>
  );
}

const dialog = () => screen.queryByRole("alertdialog");
const focused = () => document.activeElement?.textContent;

type User = ReturnType<typeof userEvent.setup>;

/** Renders the opener and opens the dialog; gives the user and the outcome spies. */
async function open() {
  const user = userEvent.setup();
  const onConfirm = vi.fn();
  const onCancel = vi.fn();
  render(<Opener onConfirm={onConfirm} onCancel={onCancel} />);
  await user.click(screen.getByRole("button", { name: "Open" }));
  return { user, onConfirm, onCancel };
}

test("a modal dialog named by its title and described by its text, the focus on Cancel", async () => {
  await open();

  const shown = screen.getByRole("alertdialog", { name: "End it?" });
  expect(shown.getAttribute("aria-modal")).toBe("true");
  expect(shown.getAttribute("aria-describedby")).toBeTruthy();
  expect(
    document.getElementById(shown.getAttribute("aria-describedby")!)
      ?.textContent,
  ).toBe("It ends at once.");
  expect(focused()).toBe("Cancel");
});

test("Tab and Shift+Tab stay inside the dialog", async () => {
  const { user } = await open();

  await user.tab();
  expect(focused()).toBe("End it");
  await user.tab();
  expect(focused()).toBe("Cancel");
  await user.tab({ shift: true });
  expect(focused()).toBe("End it");
});

test.each([
  [
    "the action confirms",
    true,
    (user: User) => user.click(screen.getByRole("button", { name: "End it" })),
  ],
  [
    "Cancel cancels",
    false,
    (user: User) => user.click(screen.getByRole("button", { name: "Cancel" })),
  ],
  ["Escape cancels", false, (user: User) => user.keyboard("{Escape}")],
  [
    "a click beside it cancels",
    false,
    (user: User) => user.click(dialog()!.parentElement!),
  ],
])("%s, and the focus returns to the opener", async (_, confirms, close) => {
  const { user, onConfirm, onCancel } = await open();

  await close(user);

  expect(onConfirm).toHaveBeenCalledTimes(confirms ? 1 : 0);
  expect(onCancel).toHaveBeenCalledTimes(confirms ? 0 : 1);
  expect(dialog()).toBeNull();
  expect(focused()).toBe("Open");
});

test("a click inside the dialog cancels nothing", async () => {
  const { user, onCancel } = await open();

  await user.click(screen.getByText("It ends at once."));

  expect(onCancel).not.toHaveBeenCalled();
  expect(dialog()).not.toBeNull();
});
