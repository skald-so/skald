// Sessions and signing out in the web app, rendered whole against the API's mocks: the generated
// ones, and the answers these tests give for the account's sessions.
//
// The API, as these tests use it, beside the spec:
// - `GET /account/sessions`: `{"sessions": [{"session", "kind": "web" | "device", "started_at",
//   "last_used_at", "current"?}]}`, a web session with `browser`, `os` and `expires_at`, a device
//   session with `device_name`;
// - `DELETE /account/sessions/{session}` ends one, `POST /account/sessions/end-others` every other
//   one, and `DELETE /session` this web session; each answers `204`;
// - `GET /spaces/{space}/service-tokens`: `{"tokens": []}`, so Access has no row of a token.
//
// The web app, as these tests use it: the cabinet's side menu has the link "Access", whose table
// has a row per session. The page's own session's row says "this session" and has no "Revoke"
// button; each other row has one, which ends its session without asking. A row says "Web" or "CLI",
// the browser and OS or the device's name, and when the session expires: a web session's date or
// days left, "when revoked" for a CLI session. A click on the column header "Last used" sorts the
// rows by it; another click reverses them. "Sign out everywhere else" opens a dialog that says the
// machines are signed out too, and its button whose name says "sign out" confirms. The account
// menu, the footer's button "Account", has the item "Sign out".
import { http, HttpResponse } from "msw";
import { setupServer } from "msw/node";
import { afterAll, beforeAll, beforeEach, expect, test } from "vitest";
import {
  cleanup,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent, { type UserEvent } from "@testing-library/user-event";
import {
  getListSpaceInvitesMockHandler,
  getSkaldAPIMock,
} from "./api/skald.msw.ts";
import { getGetSessionUrl, getGetSpacesUrl } from "./api/skald.ts";
import { App } from "./App.tsx";

const sessionsUrl = "/account/sessions";
const endOthersUrl = "/account/sessions/end-others";

interface Listed {
  session: string;
  kind: "web" | "device";
  sign_in_method?: "corporate";
  browser?: string;
  os?: string;
  device_name?: string;
  started_at: string;
  last_used_at: string;
  expires_at?: string;
  current?: boolean;
}

const minute = 60_000;
const day = 24 * 60 * minute;

/** The date-time `offset` milliseconds from now. */
const fromNow = (offset: number) => new Date(Date.now() + offset).toISOString();

/** The page's own web session, in Chrome on macOS, expiring in 12 days. */
const chrome: Listed = {
  session: "01968a2e-0000-7000-8000-0000000002a1",
  kind: "web",
  browser: "Chrome",
  os: "macOS",
  started_at: fromNow(-18 * day),
  last_used_at: fromNow(-minute),
  expires_at: fromNow(12 * day + 60 * minute),
  current: true,
};
/** Another web session, in Firefox on Windows, expiring in 27 days. */
const firefox: Listed = {
  session: "01968a2e-0000-7000-8000-0000000002a2",
  kind: "web",
  browser: "Firefox",
  os: "Windows",
  started_at: fromNow(-3 * day),
  last_used_at: fromNow(-2 * day),
  expires_at: fromNow(27 * day + 60 * minute),
};
/** A CLI session of the device ana-laptop. */
const laptop: Listed = {
  session: "01968a2e-0000-7000-8000-0000000002a3",
  kind: "device",
  device_name: "ana-laptop",
  started_at: fromNow(-40 * day),
  last_used_at: fromNow(-180 * minute),
};

/** The account's sessions the mocks list now. */
let listed: Listed[] = [];
/** Whether the page's web session has signed out. */
let signedOut = false;
let signedInWith: "github" | "corporate" = "github";
/** Each request the web app sent, as `DELETE /session`. */
let sent: string[] = [];

/** A `401` problem: the API refuses the session. */
const refused = () =>
  HttpResponse.json(
    { type: "about:blank", title: "Unauthorized", status: 401 },
    { status: 401, headers: { "content-type": "application/problem+json" } },
  );

const server = setupServer(
  http.get(`*${getGetSessionUrl()}`, () =>
    signedOut
      ? refused()
      : HttpResponse.json({
          account: "01968a2e-0000-7000-8000-000000000001",
          username: "ana",
          ...(signedInWith === "github" ? { github_login: "ana-gh" } : {}),
          sign_in_method: signedInWith,
        }),
  ),
  http.get(`*${getGetSpacesUrl()}`, () =>
    signedOut
      ? refused()
      : HttpResponse.json({
          spaces: [
            {
              space: "01968a2e-0000-7000-8000-000000000101",
              name: "ana",
              personal: true,
            },
          ],
        }),
  ),
  http.get("*/auth/providers", () =>
    HttpResponse.json({ providers: ["github", "google"], sign_up: "open" }),
  ),
  http.get(`*${sessionsUrl}`, () =>
    signedOut ? refused() : HttpResponse.json({ sessions: listed }),
  ),
  http.post(`*${endOthersUrl}`, () => {
    listed = listed.filter((session) => session.current);
    return new HttpResponse(null, { status: 204 });
  }),
  http.delete(`*${sessionsUrl}/:session`, ({ params }) => {
    listed = listed.filter((session) => session.session !== params.session);
    return new HttpResponse(null, { status: 204 });
  }),
  http.delete(`*${getGetSessionUrl()}`, () => {
    signedOut = true;
    return new HttpResponse(null, { status: 204 });
  }),
  http.get("*/spaces/:space/service-tokens", () =>
    signedOut ? refused() : HttpResponse.json({ tokens: [] }),
  ),
  getListSpaceInvitesMockHandler({ invites: [] }),
  ...getSkaldAPIMock(),
);
/** Records `request` in `sent`. */
function record({ request }: { request: Request }) {
  const { pathname } = new URL(request.url);
  sent.push(`${request.method} ${pathname}`);
}
server.events.on("request:start", record);

beforeEach(() => {
  cleanup();
  window.history.replaceState(null, "", "/");
  [listed, signedOut, sent] = [[chrome, firefox, laptop], false, []];
  signedInWith = "github";
});
beforeAll(() => server.listen({ onUnhandledFrame: "error" }));
afterAll(() => server.close());

/** The rows of the Access table but its header, in order. */
function rows(): HTMLElement[] {
  return screen
    .getAllByRole("row")
    .filter((row) => within(row).queryAllByRole("columnheader").length === 0);
}

/** The one row whose text matches `text`. */
function rowOf(text: RegExp): HTMLElement {
  const found = rows().filter((row) => text.test(row.textContent ?? ""));
  expect(found, `one row of ${text}`).toHaveLength(1);
  return found[0]!;
}

/** The "Revoke" buttons in `row`. */
const revokes = (row: HTMLElement) =>
  within(row).queryAllByRole("button", { name: /^revoke\b/i });

/** Renders the web app signed in and opens Access from the side menu; waits for the rows. */
async function openAccess(user: UserEvent) {
  render(<App />);
  await screen.findByRole("button", { name: "Account" });
  await user.click(await screen.findByRole("link", { name: "Access" }));
  await screen.findByText(/this session/i);
  await waitFor(() => expect(rows()).toHaveLength(listed.length));
}

/**
 * The forms in which a row may say when `expires` comes, `days` and an hour from now: the days
 * left, rounded either way, or its date.
 */
function expiryForms(expires: string, days: number): RegExp {
  const date = new Date(expires);
  const forms = [
    // No \b after "days": the row's text runs on into its "Revoke" button.
    `\\b(${days}|${days + 1}) days`,
    date.toLocaleDateString("en-US", { month: "short", day: "numeric" }),
    date.toLocaleDateString("en-GB", { month: "short", day: "numeric" }),
    date.toISOString().slice(0, 10),
  ];
  return new RegExp(forms.join("|"), "i");
}

test("Access has a row per session: web or CLI and where; the current one marked and without Revoke", async () => {
  await openAccess(userEvent.setup());
  expect(rows()).toHaveLength(3);

  const own = rowOf(/this session/i);
  for (const part of ["Web", "Chrome", "macOS"])
    expect(own.textContent).toContain(part);
  expect(revokes(own)).toHaveLength(0);

  const other = rowOf(/Firefox/);
  for (const part of ["Web", "Windows"])
    expect(other.textContent).toContain(part);
  expect(other.textContent).not.toMatch(/this session/i);
  expect(revokes(other)).toHaveLength(1);

  const cli = rowOf(/ana-laptop/);
  expect(cli.textContent).toContain("CLI");
  expect(cli.textContent).not.toMatch(/this session/i);
  expect(revokes(cli)).toHaveLength(1);
});

test("Expires shows a web session's expiry and 'when revoked' for a CLI session", async () => {
  await openAccess(userEvent.setup());

  expect(rowOf(/ana-laptop/).textContent).toMatch(/when revoked/i);
  for (const [session, days] of [
    [chrome, 12],
    [firefox, 27],
  ] as [Listed, number][]) {
    const row = rowOf(new RegExp(session.browser!));
    expect(row.textContent).not.toMatch(/when revoked/i);
    expect(row.textContent).toMatch(expiryForms(session.expires_at!, days));
  }
});

test("Revoke ends that session without asking, and its row goes", async () => {
  const user = userEvent.setup();
  await openAccess(user);

  await user.click(revokes(rowOf(/Firefox/))[0]!);

  await waitFor(() =>
    expect(sent).toContain(`DELETE ${sessionsUrl}/${firefox.session}`),
  );
  expect(screen.queryAllByRole("dialog")).toHaveLength(0);
  expect(screen.queryAllByRole("alertdialog")).toHaveLength(0);
  await waitFor(() => expect(rows()).toHaveLength(2));
  expect(rows().some((row) => /Firefox/.test(row.textContent ?? ""))).toBe(
    false,
  );
  expect(sent).not.toContain(`DELETE ${sessionsUrl}/${laptop.session}`);
});

test("A click on the Last used column sorts the rows by it; another reverses them", async () => {
  const user = userEvent.setup();
  await openAccess(user);
  const order = () =>
    rows().map((row) =>
      /Chrome/.test(row.textContent ?? "")
        ? "chrome"
        : /Firefox/.test(row.textContent ?? "")
          ? "firefox"
          : "laptop",
    );
  const newest = ["chrome", "laptop", "firefox"];
  const oldest = [...newest].reverse();

  await user.click(screen.getByRole("button", { name: /^last used/i }));
  const first = order();
  expect([newest, oldest]).toContainEqual(first);

  await user.click(screen.getByRole("button", { name: /^last used/i }));
  expect(order()).toEqual([...first].reverse());
});

test("'Sign out everywhere else' asks first, says it signs the machines out, then leaves only this session", async () => {
  const user = userEvent.setup();
  await openAccess(user);

  await user.click(
    screen.getByRole("button", { name: /^sign out everywhere else$/i }),
  );
  const dialogs = [
    ...screen.queryAllByRole("dialog"),
    ...screen.queryAllByRole("alertdialog"),
  ];
  expect(dialogs).toHaveLength(1);
  expect(dialogs[0]!.textContent).toMatch(/machine/i);
  expect(sent).not.toContain(`POST ${endOthersUrl}`);

  await user.click(
    within(dialogs[0]!).getByRole("button", { name: /sign out/i }),
  );

  await waitFor(() => expect(sent).toContain(`POST ${endOthersUrl}`));
  await waitFor(() => expect(rows()).toHaveLength(1));
  expect(rows()[0]!.textContent).toMatch(/this session/i);
  expect(sent.filter((request) => request.startsWith("DELETE"))).toEqual([]);
});

test("Sign out in the account menu ends this web session and shows the sign-in page", async () => {
  const user = userEvent.setup();
  render(<App />);

  await user.click(await screen.findByRole("button", { name: "Account" }));
  await user.click(await screen.findByRole("menuitem", { name: "Sign out" }));

  await screen.findByText(/creates? an account/i);
  expect(sent).toContain(`DELETE ${getGetSessionUrl()}`);
  expect(screen.queryByRole("button", { name: "Account" })).toBeNull();
});

test("Access shows the current web session after corporate sign-in", async () => {
  signedInWith = "corporate";
  listed = [{ ...chrome, sign_in_method: "corporate" }];
  await openAccess(userEvent.setup());

  const row = rowOf(/this session/i);
  for (const part of ["Web", "Corporate", "Chrome", "macOS"])
    expect(row.textContent).toContain(part);
  expect(revokes(row)).toHaveLength(0);
});
