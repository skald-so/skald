// Deleting an account in the web app, rendered whole against the API's mocks: the generated ones,
// and the answers these tests give for the session, its spaces, the account's sign-in methods,
// what a deletion deletes and blocks, the deletion itself and a space's members.
//
// The API, as these tests use it, beside the spec:
// - `GET /account/deletion`: `{"deletes": [{"space", "name"}], "blocks": [{"space", "name"}]}`;
// - `DELETE /account`: `204` once the account is deleted; a `403` problem
//   `https://skald.so/problems/sign-in-too-old` for a sign-in older than the server's setting; a
//   `409` problem `https://skald.so/problems/deletion-blocked` naming the blocking spaces;
// - `GET /spaces/{space}/members`: a member whose account is deleted has `"deleted": true`.
//
// The web app, as these tests use it, beside `web-sign-in-methods.test.tsx` and
// `web-members.test.tsx`: the Account page names each space the deletion deletes, and each space
// that blocks it with the command that hands over admin rights on the admin's device,
// `skald hand-over-admin <space> …`. While a space blocks, the page has no button whose name starts
// with "Delete"; otherwise it has one, which opens a dialog, role `dialog` or `alertdialog`. The
// dialog says what goes at once and what 30 days later, and holds a textbox for the username and a
// button whose name starts with "Delete", which deletes the account only once the textbox holds the
// account's username. After the deletion the web app shows the sign-in page, which says the
// account was deleted. With a sign-in older than the setting, the page asks to sign in again with a
// link or button whose text says "sign in", and stays in the cabinet. In Members, the row of a
// member the server reports deleted says "deleted".
import { afterAll, beforeAll, beforeEach, expect, test } from "vitest";
import { http, HttpResponse } from "msw";
import { setupServer } from "msw/node";
import {
  cleanup,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent, { type UserEvent } from "@testing-library/user-event";
import { getSkaldAPIMock } from "./api/skald.msw.ts";
import {
  getDeleteAccountUrl,
  getGetAccountDeletionUrl,
  getGetSessionUrl,
  getGetSpacesUrl,
  getListSignInMethodsUrl,
} from "./api/skald.ts";
import { App } from "./App.tsx";

interface Space {
  space: string;
  name: string;
}

/** The account's username. */
const username = "ana";
/** Its personal space, a team space of only it, and a team space where it is the only admin. */
const personal: Space = {
  space: "01968a2e-0000-7000-8000-000000000701",
  name: "ana",
};
const solo: Space = {
  space: "01968a2e-0000-7000-8000-000000000702",
  name: "solo-lab",
};
const team: Space = {
  space: "01968a2e-0000-7000-8000-000000000703",
  name: "team-a",
};

/** What the mocks say a deletion deletes and what blocks it. */
let deletion: { deletes: Space[]; blocks: Space[] } = {
  deletes: [],
  blocks: [],
};
/** How the mocks answer `DELETE /account`. */
let answer: "deleted" | "too-old" | "blocked" = "deleted";
/** Whether the account is deleted. */
let deleted = false;
/** The path of each `DELETE` request the web app sent. */
let deletions: string[] = [];

const problem = (status: number, type: string, extra: object = {}) =>
  HttpResponse.json(
    { type, title: "Refused", status, detail: "Refused.", ...extra },
    { headers: { "content-type": "application/problem+json" }, status },
  );

const unauthorized = () => problem(401, "about:blank");

/** Answers `body()` while the account lasts, and a `401` problem once it is deleted. */
const whileThere = (body: () => object) => () =>
  deleted ? unauthorized() : HttpResponse.json(body());

/** The members of `team-a`: the account, `bo-yang` and `kim-lab`, whose account is deleted. */
const members = [
  {
    handle: "ana",
    role: "admin",
    linked_devices: 2,
    joined_at: "2026-10-01T10:00:00Z",
  },
  {
    handle: "bo-yang",
    role: "member",
    linked_devices: 1,
    joined_at: "2026-10-02T10:00:00Z",
  },
  {
    handle: "kim-lab",
    role: "member",
    linked_devices: 0,
    joined_at: "2026-10-03T10:00:00Z",
    deleted: true,
  },
];

const server = setupServer(
  http.get(
    `*${getGetSessionUrl()}`,
    whileThere(() => ({
      account: "01968a2e-0000-7000-8000-000000000001",
      username,
      sign_in_method: "github",
      github_login: "ana-gh",
    })),
  ),
  http.get(
    `*${getGetSpacesUrl()}`,
    whileThere(() => ({
      spaces: [{ ...personal, personal: true }, solo, team],
    })),
  ),
  http.get(
    `*${getListSignInMethodsUrl()}`,
    whileThere(() => ({
      methods: [
        {
          method: "01968a2e-0000-7000-8000-0000000007a1",
          provider: "github",
          linked_at: "2026-10-01T10:00:00Z",
          github_login: "ana-gh",
          current: true,
        },
      ],
    })),
  ),
  http.get("*/auth/providers", () =>
    HttpResponse.json({ providers: ["github"], sign_up: "open" }),
  ),
  http.get(
    `*${getGetAccountDeletionUrl()}`,
    whileThere(() => deletion),
  ),
  http.delete(`*${getDeleteAccountUrl()}`, () => {
    if (deleted) return unauthorized();
    if (answer === "too-old")
      return problem(403, "https://skald.so/problems/sign-in-too-old");
    if (answer === "blocked") {
      // A revision made the account the only admin of team-a with another member meanwhile.
      deletion = { deletes: [personal, solo], blocks: [team] };
      return problem(409, "https://skald.so/problems/deletion-blocked", {
        spaces: [team],
      });
    }
    deleted = true;
    return new HttpResponse(null, { status: 204 });
  }),
  http.get(
    "*/spaces/:space/members",
    whileThere(() => ({ members })),
  ),
  ...getSkaldAPIMock(),
);
server.events.on("request:start", ({ request }) => {
  if (request.method === "DELETE")
    deletions.push(new URL(request.url).pathname);
});

beforeEach(() => {
  deletion = { deletes: [personal, solo], blocks: [] };
  answer = "deleted";
  deleted = false;
  deletions = [];
});
beforeAll(() => server.listen({ onUnhandledFrame: "error" }));
afterAll(() => server.close());

/** The dialogs open on the page. */
const dialogs = () => [
  ...screen.queryAllByRole("dialog"),
  ...screen.queryAllByRole("alertdialog"),
];

/** The buttons whose name starts with "Delete" outside the dialogs. */
const deletes = () =>
  screen
    .queryAllByRole("button", { name: /^delete\b/i })
    .filter((button) => !button.closest("[role=dialog], [role=alertdialog]"));

/** The page's text. */
const text = () => document.body.textContent ?? "";

/** The command that hands over admin rights in `space`, as the page may show it. */
const handOver = (space: string) =>
  new RegExp(`skald hand-over-admin\\s+["']?${space}\\b`);

/**
 * Renders the web app signed in, afresh, and opens the Account page from the account menu; waits
 * until it names each space the deletion deletes and blocks.
 */
async function openAccount(user: UserEvent) {
  window.history.replaceState(null, "", "/");
  cleanup();
  render(<App />);
  const menu = await screen.findByRole("button", { name: "Account" });
  await user.click(menu);
  const item = await screen.findByRole("menuitem", { name: "Account" });
  await user.click(item);
  for (const space of [solo, team].filter((space) =>
    [...deletion.deletes, ...deletion.blocks].includes(space),
  ))
    await waitFor(() => expect(text()).toContain(space.name));
}

/** Clicks Delete on the Account page; gives the dialog it opens. */
async function askToDelete(user: UserEvent): Promise<HTMLElement> {
  const found = await waitFor(() => {
    const buttons = deletes();
    expect(buttons, "one Delete").toHaveLength(1);
    return buttons[0]!;
  });
  await user.click(found);
  return waitFor(() => {
    const open = dialogs();
    expect(open).toHaveLength(1);
    return open[0]!;
  });
}

/** Types `typed` in the dialog's textbox, replacing what it held, and clicks its Delete. */
async function confirmWith(
  user: UserEvent,
  dialog: HTMLElement,
  typed: string,
) {
  const field = within(dialog).getByRole("textbox");
  await user.clear(field);
  if (typed) await user.type(field, typed);
  await user.click(within(dialog).getByRole("button", { name: /^delete\b/i }));
}

test("The Account page names the spaces the deletion deletes, and each blocking one with the command that hands over admin rights", async () => {
  deletion = { deletes: [personal, solo], blocks: [team] };
  await openAccount(userEvent.setup());

  expect(text()).toContain(solo.name);
  expect(text()).toMatch(handOver(team.name));
  expect(text()).not.toMatch(handOver(solo.name));
  expect(text()).not.toMatch(handOver(personal.name));
});

test("No Delete shows while a space blocks the deletion, and one shows once none does", async () => {
  const user = userEvent.setup();
  deletion = { deletes: [personal, solo], blocks: [team] };
  await openAccount(user);
  expect(deletes()).toHaveLength(0);

  deletion = { deletes: [personal, solo], blocks: [] };
  await openAccount(user);
  await waitFor(() => expect(deletes()).toHaveLength(1));
  expect(text()).not.toMatch(/hand-over-admin/);
  expect(deletions).toEqual([]);
});

test("Delete asks first, saying what goes at once and what 30 days later", async () => {
  const user = userEvent.setup();
  await openAccount(user);

  const dialog = await askToDelete(user);

  const said = dialog.textContent ?? "";
  expect(said).toMatch(/\b30 days\b/i);
  expect(said).toMatch(/\b(at once|immediately|right away|right now)\b/i);
  expect(within(dialog).getAllByRole("textbox")).toHaveLength(1);
  expect(deletions).toEqual([]);
});

test("Delete deletes nothing until the typed username matches, then deletes the account once", async () => {
  const user = userEvent.setup();
  await openAccount(user);
  const dialog = await askToDelete(user);

  for (const typed of ["", "an", "ana-gh", "anna", `${username}x`]) {
    await confirmWith(user, dialog, typed);
    await new Promise((resolve) => setTimeout(resolve, 100));
    expect(deletions, `typed ${JSON.stringify(typed)}`).toEqual([]);
  }

  await confirmWith(user, dialog, username);
  await waitFor(() => expect(deletions).toEqual([getDeleteAccountUrl()]));
});

test("After the deletion the web app shows the sign-in page, which says the account was deleted", async () => {
  const user = userEvent.setup();
  await openAccount(user);
  const dialog = await askToDelete(user);

  await confirmWith(user, dialog, username);

  await screen.findByText(/creates? an account/i);
  expect(screen.queryByRole("button", { name: "Account" })).toBeNull();
  await waitFor(() =>
    expect(text()).toMatch(/\baccount\b[^.]*\b(was|is|has been)\s+deleted\b/i),
  );
  expect(deletions).toEqual([getDeleteAccountUrl()]);
});

test("With a sign-in older than the setting, Delete asks to sign in again and the page stays in the cabinet", async () => {
  const user = userEvent.setup();
  answer = "too-old";
  await openAccount(user);
  const prompts = () =>
    [
      ...screen.queryAllByRole("button"),
      ...screen.queryAllByRole("link"),
    ].filter((element) => /sign in/i.test(element.textContent ?? "")).length;
  const before = prompts();
  const dialog = await askToDelete(user);

  await confirmWith(user, dialog, username);

  await waitFor(() => expect(prompts()).toBeGreaterThan(before));
  expect(screen.getByRole("button", { name: "Account" })).toBeTruthy();
  expect(screen.queryByText(/creates? an account/i)).toBeNull();
  expect(deleted).toBe(false);
});

test("When the server refuses the deletion as blocked, the page names the blocking space with the command and offers no Delete", async () => {
  const user = userEvent.setup();
  answer = "blocked";
  await openAccount(user);
  const dialog = await askToDelete(user);

  await confirmWith(user, dialog, username);

  await waitFor(() => expect(text()).toMatch(handOver(team.name)));
  await waitFor(() => expect(dialogs()).toHaveLength(0));
  expect(deletes()).toHaveLength(0);
  expect(screen.getByRole("button", { name: "Account" })).toBeTruthy();
  expect(deleted).toBe(false);
});

test("Members marks a member the server reports deleted, and no other", async () => {
  const user = userEvent.setup();
  cleanup();
  window.history.replaceState(null, "", "/");
  render(<App />);
  await user.click(await screen.findByRole("link", { name: "Members" }));
  await user.click(await screen.findByRole("button", { name: /^Space\b/ }));
  await user.click(await screen.findByRole("option", { name: /\bteam-a\b/ }));
  const rowOf = (handle: string) =>
    screen
      .queryAllByRole("row")
      .filter((row) => row.textContent?.includes(handle));
  await waitFor(() =>
    expect(members.map((member) => rowOf(member.handle).length)).toEqual([
      1, 1, 1,
    ]),
  );

  expect(rowOf("kim-lab")[0]!.textContent).toMatch(/\bdeleted\b/i);
  expect(rowOf("bo-yang")[0]!.textContent).not.toMatch(/\bdeleted\b/i);
  expect(rowOf("ana")[0]!.textContent).not.toMatch(/\bdeleted\b/i);
});
