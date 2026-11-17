// Sign-in methods in the web app, rendered whole against the API's mocks: the generated ones, and
// the answers these tests give for the session, the providers and the account's methods.
//
// The API, as these tests use it, beside the spec:
// - `GET /account/sign-in-methods`: each method also has `"method": <ID>`, and `current` marks the
//   one this web session signed in with;
// - `DELETE /account/sign-in-methods/{method}` removes that method, `204`.
//
// The web app, as these tests use it: the account menu, the footer's button "Account", has the item
// "Account", which opens the Account page. It shows the username with the GitHub login next to it,
// and each method with its provider, "GitHub" or "Google", when it was linked and, for GitHub, its
// login. While the account has two or more methods, each has a button whose name starts with
// "Remove", in its table row or list item or naming its provider; it opens a dialog that asks first,
// and the dialog's button whose name starts with "Remove" removes the method. For the method this
// web session signed in with, the dialog says it signs "this browser" out. The page has a button
// "Link GitHub" or "Link Google" for each provider the server has settings for.
import { App } from "./App.tsx";
import {
  getGetSessionUrl,
  getGetSpacesUrl,
  getListSignInMethodsUrl,
} from "./api/skald.ts";
import { getSkaldAPIMock } from "./api/skald.msw.ts";
import {
  cleanup,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent, { type UserEvent } from "@testing-library/user-event";
import { http, HttpResponse } from "msw";
import { setupServer } from "msw/node";
import { afterAll, beforeAll, beforeEach, expect, test } from "vitest";

type Provider = "github" | "google";

interface Method {
  method: string;
  provider: Provider;
  linked_at: string;
  github_login?: string;
}

const day = 24 * 60 * 60_000;

/** The date-time `days` days ago, an hour into that day's past. */
const daysAgo = (days: number) =>
  new Date(Date.now() - days * day - 60 * 60_000).toISOString();

/** A GitHub method linked 5 days ago, and a Google one, the account's first, 12 days ago. */
const github: Method = {
  method: "01968a2e-0000-7000-8000-0000000003a1",
  provider: "github",
  linked_at: daysAgo(5),
  github_login: "ana-gh",
};
const google: Method = {
  method: "01968a2e-0000-7000-8000-0000000003a2",
  provider: "google",
  linked_at: daysAgo(12),
};
/** The account's personal space. */
const personal = {
  space: "01968a2e-0000-7000-8000-000000000101",
  name: "ana",
  personal: true,
};

/** The account's methods the mocks list now. */
let methods: Method[] = [];
/** The providers the server has settings for. */
let providers: Provider[] = [];
/** The method this web session signed in with. */
let signedInWith: Provider = "google";
/** Whether the web session has ended. */
let ended = false;
/** The path of each `DELETE` request the web app sent. */
let removals: string[] = [];

/** Answers `body()` while the web session lasts, and a `401` problem once it has ended. */
const whileSignedIn = (body: () => object) => () =>
  ended
    ? HttpResponse.json(
        { status: 401, title: "Unauthorized", type: "about:blank" },
        {
          headers: { "content-type": "application/problem+json" },
          status: 401,
        },
      )
    : HttpResponse.json(body());

const server = setupServer(
  http.get(
    `*${getGetSessionUrl()}`,
    whileSignedIn(() => ({
      account: "01968a2e-0000-7000-8000-000000000001",
      username: "ana",
      sign_in_method: signedInWith,
      ...(methods.includes(github) ? { github_login: "ana-gh" } : {}),
    })),
  ),
  http.get(
    `*${getGetSpacesUrl()}`,
    whileSignedIn(() => ({ spaces: [personal] })),
  ),
  http.get(
    `*${getListSignInMethodsUrl()}`,
    whileSignedIn(() => ({
      methods: methods.map((method) => ({
        ...method,
        current: method.provider === signedInWith,
      })),
    })),
  ),
  http.get("*/auth/providers", () =>
    HttpResponse.json({ providers, sign_up: "open" }),
  ),
  http.delete(`*${getListSignInMethodsUrl()}/:method`, ({ params }) => {
    const removed = methods.find((method) => method.method === params.method);
    methods = methods.filter((method) => method !== removed);
    ended ||= removed?.provider === signedInWith;
    return new HttpResponse(null, { status: 204 });
  }),
  ...getSkaldAPIMock(),
);
server.events.on("request:start", ({ request }) => {
  if (request.method === "DELETE") removals.push(new URL(request.url).pathname);
});

beforeEach(() => {
  methods = [github, google];
  providers = ["github", "google"];
  signedInWith = "google";
  ended = false;
  removals = [];
});
beforeAll(() => server.listen({ onUnhandledFrame: "error" }));
afterAll(() => server.close());

/** The forms in which the page may say when `method`, linked 5 or 12 days and an hour ago, came. */
function linkedForms(method: Method): RegExp {
  const days = method === github ? 5 : 12;
  const date = new Date(method.linked_at);
  const short = (locale: string) =>
    date.toLocaleDateString(locale, { day: "numeric", month: "short" });
  const ago = `\\b(${days}|${days + 1}) days ago\\b`;
  const iso = date.toISOString().slice(0, 10);
  return new RegExp([ago, short("en-US"), short("en-GB"), iso].join("|"), "i");
}

/**
 * Renders the web app signed in, afresh, and opens the Account page from the account menu; waits
 * for its link buttons and for when each method was linked.
 */
async function openAccount(user: UserEvent) {
  cleanup();
  window.history.replaceState(null, "", "/");
  render(<App />);
  await user.click(await screen.findByRole("button", { name: "Account" }));
  await user.click(await screen.findByRole("menuitem", { name: "Account" }));
  await screen.findAllByRole("button", { name: /^link (github|google)$/i });
  for (const method of methods)
    await waitFor(() =>
      expect(document.body.textContent).toMatch(linkedForms(method)),
    );
}

/** The "Remove" buttons outside the dialogs. */
const removes = () =>
  screen
    .queryAllByRole("button", { name: /^remove\b/i })
    .filter((button) => !button.closest("[role=dialog], [role=alertdialog]"));

/** The text that names the method of a "Remove" button: its row's or list item's, and its name. */
function context(button: HTMLElement): string {
  const scope = button.closest("tr, [role=row], li, [role=listitem]");
  const name = button.getAttribute("aria-label") ?? button.textContent ?? "";
  return `${scope?.textContent ?? ""} ${name}`.toLowerCase();
}

/**
 * Opens the Account page and clicks Remove of the method of `provider`, which asks first in a
 * dialog and sends nothing; confirms there and waits for the request that removes the method.
 * Gives what the dialog said.
 */
async function removeAfterAsking(user: UserEvent, provider: Provider) {
  await openAccount(user);
  const other: Provider = provider === "github" ? "google" : "github";
  const found = removes().filter(
    (button) =>
      context(button).includes(provider) && !context(button).includes(other),
  );
  expect(found, `one Remove of ${provider}`).toHaveLength(1);
  await user.click(found[0]!);
  const dialog = await waitFor(() => {
    const open = [
      ...screen.queryAllByRole("dialog"),
      ...screen.queryAllByRole("alertdialog"),
    ];
    expect(open).toHaveLength(1);
    return open[0]!;
  });
  const asked = dialog.textContent ?? "";
  expect(removals).toEqual([]);

  await user.click(within(dialog).getByRole("button", { name: /^remove\b/i }));
  const id = provider === "github" ? github.method : google.method;
  await waitFor(() =>
    expect(removals).toEqual([`${getListSignInMethodsUrl()}/${id}`]),
  );
  return asked;
}

/** The link buttons of the provider whose name `name` matches. */
const links = (name: string) =>
  screen.queryAllByRole("button", {
    name: new RegExp(`^link ${name}$`, "i"),
  });

test("The Account page shows the username with the GitHub login, and each method with its provider, when it was linked and GitHub's login", async () => {
  await openAccount(userEvent.setup());

  const text = document.body.textContent ?? "";
  for (const part of ["ana", "ana-gh", "GitHub", "Google"])
    expect(text).toContain(part);
  expect(text).toMatch(linkedForms(github));
  expect(text).toMatch(linkedForms(google));
});

test("Remove shows on each method while the account has two or more, and on none with one", async () => {
  const user = userEvent.setup();
  await openAccount(user);
  expect(removes()).toHaveLength(2);

  methods = [google];
  await openAccount(user);
  expect(removes()).toHaveLength(0);
});

test("Removing another method asks first, without saying it signs this browser out, then removes it", async () => {
  const user = userEvent.setup();

  const asked = await removeAfterAsking(user, "github");

  expect(asked).not.toMatch(/this browser/i);
  await waitFor(() => expect(removes()).toHaveLength(0));
  expect(screen.getByRole("button", { name: "Account" })).toBeTruthy();
});

test("Removing this session's method says it signs this browser out, then shows the sign-in page", async () => {
  const user = userEvent.setup();

  const asked = await removeAfterAsking(user, "google");

  expect(asked).toMatch(/this browser/i);
  expect(asked).toMatch(/sign\w*\s+(this browser\s+|you\s+)?out/i);
  await screen.findByText(/creates? an account/i);
  expect(screen.queryByRole("button", { name: "Account" })).toBeNull();
});

test("The page has a link for each provider the server has settings for", async () => {
  const user = userEvent.setup();
  await openAccount(user);
  expect(links("github")).toHaveLength(1);
  expect(links("google")).toHaveLength(1);

  for (const [offered, absent] of [
    ["github", "google"],
    ["google", "github"],
  ] as [Provider, Provider][]) {
    providers = [offered];
    await openAccount(user);
    expect(links(offered)).toHaveLength(1);
    expect(links(absent)).toHaveLength(0);
  }
});
