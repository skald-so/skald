import { cleanup, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, test } from "vitest";
import type { SignInMethod } from "../api/skald.schemas.ts";
import { AccountScreen } from "./Account.tsx";

afterEach(cleanup);

const now = Date.parse("2026-11-10T15:00:00Z");

/** Two GitHub methods of one account; this web session signed in with the first. */
const methods: SignInMethod[] = [
  {
    method: "01968a2e-0000-7000-8000-0000000003b1",
    provider: "github",
    linked_at: "2026-11-01T10:00:00Z",
    github_login: "ana-work",
    current: true,
  },
  {
    method: "01968a2e-0000-7000-8000-0000000003b2",
    provider: "github",
    linked_at: "2026-11-05T10:00:00Z",
    github_login: "ana-home",
    current: false,
  },
];

/** Clicks Remove of the method named `name` and gives the text of the dialog it opens. */
async function asked(name: string): Promise<string> {
  const user = userEvent.setup();
  render(
    <AccountScreen
      session={{
        account: "01968a2e-0000-7000-8000-000000000001",
        username: "ana",
        github_login: "ana-work",
        sign_in_method: "github",
      }}
      methods={methods}
      providers={["github"]}
      now={now}
      onRemove={async () => {}}
    />,
  );
  await user.click(screen.getByRole("button", { name: `Remove ${name}` }));
  const dialog = screen.getByRole("alertdialog");
  const text = dialog.textContent ?? "";
  await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
  return text;
}

test("Of two methods of one provider, only Remove of this session's says it signs this browser out", async () => {
  expect(await asked("GitHub ana-work")).toMatch(/this browser/i);
  cleanup();
  expect(await asked("GitHub ana-home")).not.toMatch(/this browser/i);
});
