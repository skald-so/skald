// The only-admin warning in the Members section of the web app, rendered whole against the API's
// mocks: the generated ones, and the answers these tests give for the session, its spaces and each
// space's members.
//
// The API, as these tests use it, beside that of web-members.test.tsx:
// - each member of `GET /spaces/{space}/members` may hold `"you": true`, on the member whose
//   account is the session's; absent on every other member.
//
// The web app, as these tests use it, beside web-members.test.tsx: when the session's account is
// the only admin of the chosen space and the space has another member, Members shows a warning
// banner, an alert, that says the account is the only admin and that if it loses all its devices
// and its recovery key, nobody can manage the space, and names the command that makes a member an
// admin, `skald make-admin`, to run on the admin's device; it has no button. For a member, for a
// space without another member, and for a space with two admins, Members names no such command.
import { cleanup, render, screen, within } from "@testing-library/react";
import userEvent, { type UserEvent } from "@testing-library/user-event";
import { HttpResponse, http } from "msw";
import { setupServer } from "msw/node";
import {
  afterAll,
  afterEach,
  beforeAll,
  beforeEach,
  expect,
  test,
} from "vitest";
import { getSkaldAPIMock } from "./api/skald.msw.ts";
import { getGetSessionUrl, getGetSpacesUrl } from "./api/skald.ts";
import { App } from "./App.tsx";

type Role = "admin" | "member";

interface Member {
  handle: string;
  role: Role;
  linked_devices: number;
  joined_at: string;
  you?: true;
}

/** The session's account: its username is its handle in the spaces it is in. */
const username = "ana";

/** `handle` as a member of `role`; marked as the session's account when it is `ana`. */
function member(handle: string, role: Role): Member {
  const listed: Member = {
    handle,
    role,
    linked_devices: 1,
    joined_at: "2026-11-01T10:00:00Z",
  };
  return handle === username ? { ...listed, you: true } : listed;
}

/** A space of the account: its name and its members, in the order the server gives them. */
interface Space {
  name: string;
  members: Member[];
  personal?: boolean;
}

/** Ana is the only admin; two members beside her. */
const onlyAdmin: Space = {
  name: "team-a",
  members: [
    member("lee-park", "member"),
    member(username, "admin"),
    member("sam-ortiz", "member"),
  ],
};
/** Ana is a member; kim-lab is the only admin. */
const asMember: Space = {
  name: "team-m",
  members: [
    member("kim-lab", "admin"),
    member(username, "member"),
    member("bo-yang", "member"),
  ],
};
/** Ana and kim-lab are admins, with a member. */
const twoAdmins: Space = {
  name: "team-two",
  members: [
    member(username, "admin"),
    member("kim-lab", "admin"),
    member("lee-park", "member"),
  ],
};
/** The account's personal space: ana its only member and admin. */
const alone: Space = {
  name: username,
  personal: true,
  members: [member(username, "admin")],
};
/** A team space of ana alone. */
const aloneInTeam: Space = {
  name: "team-solo",
  members: [member(username, "admin")],
};

const all = [onlyAdmin, asMember, twoAdmins, alone, aloneInTeam];
/** Each space's ID. */
const ids = new Map(
  all.map((space, at) => [
    space,
    `01968a2e-0000-7000-8000-0000000007${String(at).padStart(2, "0")}`,
  ]),
);

/** The account's spaces as the mocks give them now. */
let spaces: Space[] = [];

const server = setupServer(
  http.get(`*${getGetSessionUrl()}`, () =>
    HttpResponse.json({
      account: "01968a2e-0000-7000-8000-000000000001",
      username,
      github_login: "ana-gh",
      sign_in_method: "github",
    }),
  ),
  http.get(`*${getGetSpacesUrl()}`, () =>
    HttpResponse.json({
      spaces: spaces.map((space) => ({
        space: ids.get(space),
        name: space.name,
        ...(space.personal ? { personal: true } : {}),
      })),
    }),
  ),
  http.get("*/auth/providers", () =>
    HttpResponse.json({ providers: ["github"], sign_up: "open" }),
  ),
  http.get("*/spaces/:space/members", ({ params }) => {
    const space = spaces.find((space) => ids.get(space) === params.space);
    return space === undefined
      ? new HttpResponse(null, { status: 404 })
      : HttpResponse.json({ members: space.members });
  }),
  ...getSkaldAPIMock(),
);

beforeAll(() => server.listen({ onUnhandledFrame: "error" }));
beforeEach(() => {
  window.history.replaceState(null, "", "/");
  spaces = [alone, onlyAdmin, asMember, twoAdmins, aloneInTeam];
});
afterEach(cleanup);
afterAll(() => server.close());

/** The command the warning names. */
const command = "skald make-admin";

/** The alerts in the section that name the command. */
const warnings = () =>
  within(screen.getByRole("main"))
    .queryAllByRole("alert")
    .filter((alert) => alert.textContent?.includes(command));

/** Every handle of the spaces' members, once each. */
const everyone = [
  ...new Set(all.flatMap((space) => space.members.map((m) => m.handle))),
];

/** Whether the section's rows hold the handles of `space`'s members and no other. */
function listed(space: Space): boolean {
  const rows = within(screen.getByRole("main")).queryAllByRole("row");
  const shown = everyone.filter((handle) =>
    rows.some((row) => row.textContent?.includes(handle)),
  );
  return (
    JSON.stringify(shown.sort()) ===
    JSON.stringify(space.members.map((m) => m.handle).sort())
  );
}

/** Chooses the space `space` in the switcher and waits until its members show. */
async function choose(user: UserEvent, space: Space) {
  await user.click(await screen.findByRole("button", { name: /^Space\b/ }));
  await user.click(
    await screen.findByRole("option", {
      name: new RegExp(`\\b${space.name}\\b`),
    }),
  );
  await expect.poll(() => listed(space)).toBe(true);
}

/** Renders the web app, opens Members and chooses `space`. */
async function open(user: UserEvent, space: Space) {
  render(<App />);
  await user.click(await screen.findByRole("link", { name: "Members" }));
  await choose(user, space);
}

/** Fails unless the section shows one warning, which says what it should and has no button. */
async function assertWarns() {
  await expect.poll(() => warnings().length).toBe(1);
  const warning = warnings()[0]!;
  const text = (warning.textContent ?? "").toLowerCase();
  for (const part of ["only admin", "devices", "recovery key", command])
    expect(text, `the warning says ${part}`).toContain(part);
  expect(within(warning).queryAllByRole("button")).toEqual([]);
}

/** Fails if the section names the command, in a warning or anywhere else. */
function assertNoWarning(what: string) {
  expect(warnings(), what).toEqual([]);
  expect(screen.getByRole("main").textContent, what).not.toContain(command);
}

/** The spaces whose Members shows no warning, each with what it is. */
const unwarned: [string, Space][] = [
  ["a space where the account is a member", asMember],
  ["a space with two admins", twoAdmins],
  ["the personal space, the account alone in it", alone],
  ["a team space of the account alone", aloneInTeam],
];

test("Members warns the only admin of a space with other members and names skald make-admin", async () => {
  const user = userEvent.setup();
  await open(user, onlyAdmin);

  await assertWarns();
});

test.each(unwarned)(
  "after the only admin's warning, choosing %s shows none",
  async (what, space) => {
    const user = userEvent.setup();
    await open(user, onlyAdmin);
    await assertWarns();

    await choose(user, space);
    assertNoWarning(what);
  },
);

test.each(unwarned)(
  "opening %s shows no warning; choosing the only admin's space then shows it",
  async (what, space) => {
    const user = userEvent.setup();
    await open(user, space);
    assertNoWarning(what);

    await choose(user, onlyAdmin);
    await assertWarns();
  },
);
