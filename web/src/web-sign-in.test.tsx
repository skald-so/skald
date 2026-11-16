// Signing in to the web app, rendered whole against the API's mocks: the generated ones, and the
// answers these tests name for the sign-in providers, the session and the spaces.
//
// The API, as these tests use it, beside the spec:
// - `GET /auth/providers`: `{"providers": ["github" | "google", ...], "sign_up": "open" | "closed"}`;
// - `GET /session` of a web session names its method: `"sign_in_method": "github" | "google"`;
// - `GET /spaces` marks the account's personal space with `"personal": true`.
//
// The web app, as these tests use it: without a session, the sign-in page, a link or button for each
// provider, whose name names it, a note that a first sign-in "creates an account", and the terminal
// box, its one textbox and its button "Continue"; each opens its page as a link or a GET form does.
// With a session, the cabinet shell of the UI kit: the space switcher, a button named "Space …", and
// the account footer with the menu button "Account".
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent, { type UserEvent } from "@testing-library/user-event";
import { http, HttpResponse } from "msw";
import { setupServer } from "msw/node";
import {
  afterAll,
  afterEach,
  beforeAll,
  beforeEach,
  expect,
  test,
} from "vitest";
import { App } from "./App.tsx";
import {
  getGetSessionMockHandler401,
  getGetSpacesMockHandler401,
  getSkaldAPIMock,
} from "./api/skald.msw.ts";
import {
  getGetSessionUrl,
  getGetSpacesUrl,
  getTakePersonalSpaceUrl,
} from "./api/skald.ts";

/** The sign-in providers the server has settings for and whether its sign-up is open: `GET`. */
const providersUrl = "/auth/providers";

type Provider = "github" | "google";

const server = setupServer(...getSkaldAPIMock());
/** Each request the web app sent, as `GET /session`. */
let sent: string[] = [];
server.events.on("request:start", ({ request }) => {
  sent.push(`${request.method} ${new URL(request.url).pathname}`);
});

beforeAll(() => server.listen({ onUnhandledFrame: "error" }));
beforeEach(() => {
  sent = [];
  window.history.replaceState(null, "", "/");
});
afterEach(() => {
  cleanup();
  server.resetHandlers();
  // The web app never creates the personal space: a machine's `skald` does.
  expect(sent).not.toContain(`POST ${getTakePersonalSpaceUrl()}`);
});
afterAll(() => server.close());

/** The providers' answer: `providers`, and sign-up open unless `closed`. */
function providers(names: Provider[], closed = false) {
  return http.get(`*${providersUrl}`, () =>
    HttpResponse.json({
      providers: names,
      sign_up: closed ? "closed" : "open",
    }),
  );
}

/** A browser without a session on a server with `names` and sign-up open unless `closed`. */
function signedOut(names: Provider[] = ["github", "google"], closed = false) {
  server.use(
    getGetSessionMockHandler401(),
    getGetSpacesMockHandler401(),
    providers(names, closed),
  );
}

interface SessionAnswer {
  username: string;
  github_login?: string;
  sign_in_method: Provider;
}

interface SpaceAnswer {
  name: string;
  personal?: boolean;
}

/** A web session of the account `session` whose spaces are `spaces`. */
function signedIn(session: SessionAnswer, spaces: SpaceAnswer[]) {
  server.use(
    http.get(`*${getGetSessionUrl()}`, () =>
      HttpResponse.json({
        account: "01968a2e-0000-7000-8000-000000000001",
        ...session,
      }),
    ),
    http.get(`*${getGetSpacesUrl()}`, () =>
      HttpResponse.json({
        spaces: spaces.map((space, at) => ({
          space: `01968a2e-0000-7000-8000-00000000010${at}`,
          ...space,
        })),
      }),
    ),
    providers(["github", "google"]),
  );
}

/** The sign-in buttons of the provider whose name `name` matches: links and buttons. */
function signInButtons(name: RegExp): HTMLElement[] {
  return [
    ...screen.queryAllByRole("link", { name }),
    ...screen.queryAllByRole("button", { name }),
  ];
}

/** Waits for the sign-in page: its note on creating an account. */
function signInPage(): Promise<HTMLElement> {
  return screen.findByText(/creates? an account/i);
}

/** Waits for the cabinet of `username`: the account menu and the username shown. */
async function cabinet(username: string) {
  await screen.findByRole("button", { name: "Account" });
  await waitFor(() => expect(document.body.textContent).toContain(username));
}

/**
 * Where a click on `element` sends the browser, as a link or a form's GET submission does: the
 * address it opens; `null` if it opens none.
 */
async function opened(user: UserEvent, element: HTMLElement) {
  let to: URL | null = null;
  const onClick = (event: MouseEvent) => {
    const link = (event.target as Element).closest("a[href]");
    if (!link) return;
    to = new URL((link as HTMLAnchorElement).href, window.location.href);
    event.preventDefault();
  };
  const onSubmit = (event: SubmitEvent) => {
    const form = event.target as HTMLFormElement;
    if (form.method.toLowerCase() === "get") {
      const url = new URL(form.action, window.location.href);
      url.search = "";
      new FormData(form, event.submitter).forEach((value, name) =>
        url.searchParams.append(name, String(value)),
      );
      to = url;
    }
    event.preventDefault();
  };
  document.addEventListener("click", onClick, true);
  document.addEventListener("submit", onSubmit, true);
  try {
    await user.click(element);
  } finally {
    document.removeEventListener("click", onClick, true);
    document.removeEventListener("submit", onSubmit, true);
  }
  return to as URL | null;
}

test.each([
  [["github", "google"] as Provider[], 1, 1],
  [["github"] as Provider[], 1, 0],
  [["google"] as Provider[], 0, 1],
])(
  "without a session, the sign-in page offers the providers %j only",
  async (names, github, google) => {
    signedOut(names);
    render(<App />);
    await signInPage();
    expect(signInButtons(/github/i)).toHaveLength(github);
    expect(signInButtons(/google/i)).toHaveLength(google);
  },
);

test("with sign-up open, the note says that a first sign-in creates an account", async () => {
  signedOut(["github", "google"]);
  render(<App />);
  const note = await signInPage();
  expect(note.textContent).not.toMatch(/\bonly\b/i);
});

test("with sign-up closed, the note says that it creates one only for the people the server lists", async () => {
  signedOut(["github", "google"], true);
  render(<App />);
  const note = await signInPage();
  expect(note.textContent).toMatch(/\bonly\b/i);
});

test.each(["github", "google"] as Provider[])(
  "the %s button starts its sign-in, returning to the page",
  async (provider) => {
    const user = userEvent.setup();
    window.history.replaceState(null, "", "/?from=a-link");
    signedOut();
    render(<App />);
    await signInPage();
    const [button] = signInButtons(new RegExp(provider, "i"));
    const to = await opened(user, button!);
    expect(to?.origin).toBe(window.location.origin);
    expect(to?.pathname).toBe(`/auth/${provider}`);
    const back = to?.searchParams.get("return_to") ?? "";
    expect(new URL(back, window.location.origin).href).toBe(
      window.location.href,
    );
  },
);

test("the terminal box opens the verification page with the code", async () => {
  const user = userEvent.setup();
  signedOut();
  render(<App />);
  await signInPage();
  await user.type(screen.getByRole("textbox"), "BCDF-GHJK");
  const to = await opened(
    user,
    screen.getByRole("button", { name: "Continue" }),
  );
  expect(to?.origin).toBe(window.location.origin);
  expect(to?.pathname).toBe("/device");
  expect(to?.searchParams.get("user_code")).toBe("BCDF-GHJK");
});

test("the space switcher lists the account's spaces, the personal one marked", async () => {
  const user = userEvent.setup();
  signedIn({ username: "ana", sign_in_method: "github" }, [
    { name: "team-a" },
    { name: "ana", personal: true },
    { name: "team-b", personal: false },
  ]);
  render(<App />);
  await cabinet("ana");
  await user.click(await screen.findByRole("button", { name: /^Space\b/ }));
  const options = screen.getAllByRole("option");
  expect(options).toHaveLength(3);
  const marked = screen.getAllByRole("option", { name: /personal/i });
  expect(marked).toHaveLength(1);
  expect(marked[0]!.textContent).toContain("ana");
  for (const team of ["team-a", "team-b"])
    expect(screen.getByRole("option", { name: new RegExp(team) })).not.toBe(
      marked[0],
    );
});

test("an account with no space gets the empty state that says to sign in a machine with skald", async () => {
  signedIn({ username: "ana", sign_in_method: "google" }, []);
  render(<App />);
  await cabinet("ana");
  const said = await screen.findAllByText(/machine/i);
  expect(
    said.some((element) =>
      /skald/.test(element.closest("p, div, section")?.textContent ?? ""),
    ),
  ).toBe(true);
  expect(screen.queryAllByRole("option")).toHaveLength(0);
});

test.each([
  [
    "a GitHub session: the username, the login and GitHub",
    { username: "ana", github_login: "ana-gh", sign_in_method: "github" },
    ["ana", "ana-gh", "GitHub"],
    ["Google"],
  ],
  [
    "a Google session of an account with a GitHub method: the username, the login and Google",
    { username: "cy", github_login: "cy-gh", sign_in_method: "google" },
    ["cy", "cy-gh", "Google"],
    [],
  ],
  [
    "a Google session of an account without one: the username and Google, no GitHub login",
    { username: "bo", sign_in_method: "google" },
    ["bo", "Google"],
    ["GitHub"],
  ],
] as [string, SessionAnswer, string[], string[]][])(
  "the account footer of %s; the page area names the account",
  async (_, session, shown, hidden) => {
    signedIn(session, [{ name: "team-a" }]);
    render(<App />);
    await cabinet(session.username);
    const text = document.body.textContent ?? "";
    for (const part of shown) expect(text).toContain(part);
    for (const part of hidden) expect(text).not.toContain(part);
    expect(screen.getByRole("main").textContent).toContain(session.username);
  },
);

test("when the API refuses the session, the web app shows the sign-in page", async () => {
  signedIn({ username: "ana", sign_in_method: "github" }, []);
  server.use(getGetSpacesMockHandler401());
  render(<App />);
  await signInPage();
  expect(signInButtons(/github/i)).toHaveLength(1);
  expect(screen.queryByRole("button", { name: "Account" })).toBeNull();
});
