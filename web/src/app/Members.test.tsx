// Members when a space's members fail to load and that space is chosen again, when the API
// refuses the session, and the only admin's banner.
import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http, HttpResponse } from "msw";
import { setupServer } from "msw/node";
import { MemoryRouter } from "react-router";
import { afterAll, afterEach, beforeAll, expect, test, vi } from "vitest";
import { Cabinet } from "./Cabinet.tsx";
import { Members, MembersScreen } from "./Members.tsx";

const own = { space: "01968a2e-0000-7000-8000-000000000601", name: "ana" };
const team = { space: "01968a2e-0000-7000-8000-000000000602", name: "team-b" };
/** A space whose members the API answers as to a session it refuses. */
const refused = {
  space: "01968a2e-0000-7000-8000-000000000603",
  name: "team-c",
};
const member = (handle: string) => ({
  handle,
  role: "member" as const,
  linked_devices: 1,
  joined_at: "2026-11-01T10:00:00Z",
});

/** How many times the server answered the team's members; it fails the first. */
let asked = 0;
const server = setupServer(
  http.get(`*/spaces/${own.space}/members`, () =>
    HttpResponse.json({ members: [member("ana")] }),
  ),
  http.get(`*/spaces/${team.space}/members`, () =>
    asked++ === 0
      ? new HttpResponse(null, { status: 500 })
      : HttpResponse.json({ members: [member("kim-lab")] }),
  ),
  http.get(`*/spaces/${refused.space}/members`, () =>
    HttpResponse.json(
      { type: "https://skald.so/problems/not-signed-in", status: 401 },
      { status: 401 },
    ),
  ),
);

beforeAll(() => server.listen({ onUnhandledFrame: "error" }));
afterEach(cleanup);
afterAll(() => server.close());

test("members that load after a failure clear what failed", async () => {
  const user = userEvent.setup();
  render(
    <MemoryRouter initialEntries={["/members"]}>
      <Cabinet
        session={{ account: own.space, username: "ana" }}
        spaces={[
          { ...own, personal: true },
          { ...team, personal: false },
        ]}
        defaultSpace={team.space}
        onSignedOut={() => {}}
      >
        <Members onSignedOut={() => {}} />
      </Cabinet>
    </MemoryRouter>,
  );
  const choose = async (name: string) => {
    await user.click(screen.getByRole("button", { name: /^Space\b/ }));
    await user.click(
      await screen.findByRole("option", { name: new RegExp(`\\b${name}\\b`) }),
    );
  };

  expect(await screen.findByText("The members did not load.")).toBeTruthy();
  await choose(own.name);
  expect(await screen.findByRole("row", { name: /\bana\b/ })).toBeTruthy();
  await choose(team.name);
  expect(await screen.findByRole("row", { name: /kim-lab/ })).toBeTruthy();
  expect(screen.queryByText("The members did not load.")).toBeNull();
});

test("when the API refuses the session, Members shows the sign-in page", async () => {
  const onSignedOut = vi.fn();
  render(
    <MemoryRouter initialEntries={["/members"]}>
      <Cabinet
        session={{ account: own.space, username: "ana" }}
        spaces={[{ ...refused, personal: false }]}
        onSignedOut={() => {}}
      >
        <Members onSignedOut={onSignedOut} />
      </Cabinet>
    </MemoryRouter>,
  );

  await expect.poll(() => onSignedOut.mock.calls.length).toBe(1);
  expect(screen.queryByText("The members did not load.")).toBeNull();
});

test.each(["team-a", "ops"])(
  "the only admin's banner in %s names the command for that space on the admin's device",
  (name) => {
    render(
      <MembersScreen
        space={name}
        members={[
          { ...member("ana"), role: "admin", you: true },
          member("lee"),
        ]}
        now={0}
      />,
    );
    const text = screen.getByRole("alert").textContent!.replace(/\s+/g, " ");
    expect(text).toContain(
      `run skald make-admin ${name} <handle> on your device.`,
    );
  },
);
