// Members when a space's members fail to load and that space is chosen again.
import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http, HttpResponse } from "msw";
import { setupServer } from "msw/node";
import { MemoryRouter } from "react-router";
import { afterAll, afterEach, beforeAll, expect, test } from "vitest";
import { Cabinet } from "./Cabinet.tsx";
import { Members } from "./Members.tsx";

const own = { space: "01968a2e-0000-7000-8000-000000000601", name: "ana" };
const team = { space: "01968a2e-0000-7000-8000-000000000602", name: "team-b" };
const member = (handle: string) => ({
  handle,
  role: "member",
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
