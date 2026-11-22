// Service agent tokens in the Access section and service agents in the Usage section of the web
// app, rendered whole against the API's mocks: the generated ones, and the answers these tests give
// for the session, its spaces, its sessions, each space's members, service tokens and usage.
//
// The API, as these tests use it, beside the spec:
// - `GET /spaces/{space}/service-tokens`: `{"tokens": [{"id", "device", "owner", "agent",
//   "permissions": ["send" | "receive" | "queues", ...], "channels": [<ID>, ...], "created_at",
//   "last_used_at": <date-time> | null, "expires_at"}]}`, the space's live tokens; for an account
//   that is no admin of the space, `403` with the problem `https://skald.so/problems/not-allowed`;
// - `DELETE /spaces/{space}/service-tokens/{token}` revokes one: `204`;
// - `GET /spaces/{space}/usage`, beside what the tests of Usage read: `"service_agents": {"count",
//   "limit"?}`, `limit` absent while no limit applies.
//
// The web app, as these tests use it: the side menu's link "Access" opens the section, which has,
// beside the sessions, a row per live token of each space where the account is an admin. A token's
// row names its agent, as `ci` and `deploy-bot`; says its permissions, as "send", "receive" and
// "queues"; its channels, by ID, by the first 8 characters of an ID, or by their number, as "2
// channels"; when it was created, as days ago or its date; its last use, as "3 h ago", or "never"
// before its first; and when it expires, as the days left or its date. Each token's row has a
// button named "Revoke …", which opens a dialog, role `dialog` or `alertdialog`, with "Cancel" and
// a button whose name says "revoke"; nothing is revoked before that one is clicked, and the row of a
// revoked token goes. A space where the account is no admin shows no token and no failure, no
// element of role `alert`. The side menu's link "Usage" opens the section for the space the
// switcher chose: the service agents with their limit as "<count> of <limit>", with a bar, an
// element of role `progressbar` or `meter` whose accessible name starts with "Service agents" and
// whose value is the count and maximum the limit; without a limit, no such bar, and the section
// says "no limit" or "unlimited".
import { App } from "./App.tsx";
import { getGetSessionUrl, getGetSpacesUrl } from "./api/skald.ts";
import { http, HttpResponse } from "msw";
import { afterAll, beforeAll, beforeEach, expect, test } from "vitest";
import {
  cleanup,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent, { type UserEvent } from "@testing-library/user-event";
import { setupServer } from "msw/node";
import { getSkaldAPIMock } from "./api/skald.msw.ts";

type Permission = "send" | "receive" | "queues";

interface Token {
  id: string;
  device: string;
  owner: string;
  agent: string;
  permissions: Permission[];
  channels: string[];
  created_at: string;
  last_used_at: string | null;
  expires_at: string;
}

interface Counted {
  count: number;
  limit?: number;
}

const hourMs = 3_600_000;
const dayMs = 24 * hourMs;

/** The date-time `offset` milliseconds from now. */
const at = (offset: number) => new Date(Date.now() + offset).toISOString();

/** The account's personal space `ana`, the team space `blue-river` and `green-hill`. */
const ownId = "01968a2e-0000-7000-8000-000000000901";
const teamId = "01968a2e-0000-7000-8000-000000000902";
const otherId = "01968a2e-0000-7000-8000-000000000903";

/** Channels of `blue-river`. */
const deployChannel = "01968a2e-1111-7000-8000-000000000a01";
const releaseChannel = "01968a2e-2222-7000-8000-000000000a02";

/** `ci/deploy-bot`'s token: send and receive in one channel, used 3 h ago, 80 days left. */
const deploy: Token = {
  id: "01968a2e-0000-7000-8000-000000000b01",
  device: "01968a2e-0000-7000-8000-000000000c01",
  owner: "ci",
  agent: "deploy-bot",
  permissions: ["send", "receive"],
  channels: [deployChannel],
  created_at: at(-10 * dayMs - hourMs),
  last_used_at: at(-3 * hourMs - 60_000),
  expires_at: at(80 * dayMs + hourMs),
};
/** `ci/release-bot`'s token: send and queues in two channels, never used, 5 days left. */
const release: Token = {
  id: "01968a2e-0000-7000-8000-000000000b02",
  device: "01968a2e-0000-7000-8000-000000000c02",
  owner: "ci",
  agent: "release-bot",
  permissions: ["send", "queues"],
  channels: [deployChannel, releaseChannel],
  created_at: at(-2 * dayMs - hourMs),
  last_used_at: null,
  expires_at: at(5 * dayMs + hourMs),
};

/** The live tokens of `blue-river` as the mocks give them now. */
let tokens: Token[] = [];
/** `blue-river`'s service agents as its usage gives them now. */
let agents: Counted = { count: 0 };
/** Each request the web app sent, as `DELETE /spaces/<ID>/service-tokens/<ID>`. */
let sent: string[] = [];

/** A member entry of the account, `ana`, of `role`. */
const ana = (role: "admin" | "member") => ({
  handle: "ana",
  role,
  github_login: "ana-gh",
  linked_devices: 2,
  joined_at: at(-30 * dayMs),
});

/** The problem of a request that needs an admin. */
const notAllowed = () =>
  HttpResponse.json(
    {
      type: "https://skald.so/problems/not-allowed",
      title: "Not allowed",
      status: 403,
    },
    { status: 403, headers: { "content-type": "application/problem+json" } },
  );

const server = setupServer(
  http.get(`*${getGetSessionUrl()}`, () =>
    HttpResponse.json({
      account: "01968a2e-0000-7000-8000-000000000001",
      username: "ana",
      github_login: "ana-gh",
      sign_in_method: "github",
    }),
  ),
  http.get(`*${getGetSpacesUrl()}`, () =>
    HttpResponse.json({
      spaces: [
        { space: ownId, name: "ana", personal: true },
        { space: teamId, name: "blue-river" },
        { space: otherId, name: "green-hill" },
      ],
    }),
  ),
  http.get("*/auth/providers", () =>
    HttpResponse.json({ providers: ["github"], sign_up: "open" }),
  ),
  http.get("*/account/sessions", () =>
    HttpResponse.json({
      sessions: [
        {
          session: "01968a2e-0000-7000-8000-000000000d01",
          kind: "web",
          browser: "Chrome",
          os: "macOS",
          started_at: at(-dayMs),
          last_used_at: at(-60_000),
          expires_at: at(29 * dayMs),
          current: true,
        },
      ],
    }),
  ),
  http.get("*/spaces/:space/members", ({ params }) =>
    HttpResponse.json({
      members: [ana(params.space === otherId ? "member" : "admin")],
    }),
  ),
  http.get("*/spaces/:space/service-tokens", ({ params }) => {
    if (params.space === otherId) return notAllowed();
    return HttpResponse.json({
      tokens: params.space === teamId ? tokens : [],
    });
  }),
  http.delete("*/spaces/:space/service-tokens/:token", ({ params }) => {
    if (params.space === teamId)
      tokens = tokens.filter((token) => token.id !== params.token);
    return new HttpResponse(null, { status: 204 });
  }),
  http.get("*/account/usage", () =>
    HttpResponse.json({
      owned_spaces: {
        count: 1,
        limit: 2,
        spaces: [{ space: ownId, name: "ana" }],
      },
      devices: { count: 2, limit: 10 },
    }),
  ),
  http.get("*/spaces/:space/usage", () =>
    HttpResponse.json({
      monthly_messages: { count: 120, limit: 1_000_000 },
      message_size: { limit: 64 * 1024 },
      mailbox_retention: { limit: 7 * 86_400 },
      history_retention: { limit: 90 * 86_400 },
      service_agents: agents,
    }),
  ),
  ...getSkaldAPIMock(),
);
beforeAll(() => {
  // Each request the web app sends, as `<METHOD> <path>`.
  server.events.on("request:start", ({ request }) => {
    sent.push(`${request.method} ${new URL(request.url).pathname}`);
  });
  server.listen({ onUnhandledFrame: "error" });
});
beforeEach(() => {
  [tokens, agents, sent] = [[deploy, release], { count: 3, limit: 10 }, []];
  cleanup();
  window.history.replaceState(null, "", "/");
});
afterAll(() => server.close());

/** The requests the web app sent that revoke a token. */
const revocations = () =>
  sent.filter((request) =>
    /^DELETE \/spaces\/[^/]+\/service-tokens\//.test(request),
  );

/** The text of `element`, a space between the texts of its elements, as a person reads it. */
function textOf(element: HTMLElement): string {
  const texts = document.createTreeWalker(element, NodeFilter.SHOW_TEXT);
  const parts: string[] = [];
  while (texts.nextNode()) parts.push(texts.currentNode.textContent ?? "");
  return parts.join(" ");
}

/** The rows of the page whose text holds `agent`. */
const rowsOf = (agent: string) =>
  screen.queryAllByRole("row").filter((row) => textOf(row).includes(agent));

/** The one row of the token of `agent`, once it shows. */
async function tokenRow(agent: string): Promise<HTMLElement> {
  await waitFor(() => expect(rowsOf(agent)).toHaveLength(1));
  return rowsOf(agent)[0]!;
}

/** Renders the web app signed in and opens Access; waits for the row of `ci/deploy-bot`. */
async function openAccess(user: UserEvent): Promise<HTMLElement> {
  render(<App />);
  await user.click(await screen.findByRole("link", { name: "Access" }));
  return tokenRow(deploy.agent);
}

/**
 * The forms in which a row may say the day `when`, `days` from now either way: the days, rounded
 * either way, or its date.
 */
function dayForms(when: string, days: number): string[] {
  const date = new Date(when);
  return [
    `${days} days`,
    `${days + 1} days`,
    date.toISOString().slice(0, 10),
    ...["en-US", "en-GB"].map((locale) =>
      date.toLocaleDateString(locale, { month: "short", day: "numeric" }),
    ),
  ];
}

/** The forms in which a row may name `channels`: by ID or the start of one, or their number. */
const channelForms = (channels: string[]) => [
  ...channels.flatMap((channel) => [channel, channel.slice(0, 8)]),
  channels.length === 1 ? "1 channel" : `${channels.length} channels`,
];

/** Fails unless the text of `row` holds one of `forms`, in any case; `what` names them. */
function expectOneOf(row: HTMLElement, forms: string[], what: string) {
  const text = textOf(row).toLowerCase();
  expect(
    forms.some((form) => text.includes(form.toLowerCase())),
    `${what}, one of ${forms.join(" | ")}: ${textOf(row)}`,
  ).toBe(true);
}

/** The "Revoke" button of `row`. */
const revokeIn = (row: HTMLElement) =>
  within(row).getByRole("button", { name: /^revoke\b/i });

/** The open dialog, of either role. */
async function question(): Promise<HTMLElement> {
  await waitFor(() =>
    expect([
      ...screen.queryAllByRole("dialog"),
      ...screen.queryAllByRole("alertdialog"),
    ]).toHaveLength(1),
  );
  return [
    ...screen.queryAllByRole("dialog"),
    ...screen.queryAllByRole("alertdialog"),
  ][0]!;
}

test("Access has a row per live token of a space where the account is an admin: its agent, permissions, channels, creation, last use and expiry", async () => {
  const row = await openAccess(userEvent.setup());
  const text = textOf(row);
  expect(text).toMatch(/\bci\b/);
  expect(text).toMatch(/\bsend\b/i);
  expect(text).toMatch(/\breceive\b/i);
  expect(text).not.toMatch(/queue/i);
  expectOneOf(row, channelForms(deploy.channels), "the channel");
  expectOneOf(row, dayForms(deploy.created_at, 10), "the creation");
  expect(text).toMatch(/\b3\s*(h|hours?)\b/i);
  expect(text).not.toMatch(/\bnever\b/i);
  expectOneOf(row, dayForms(deploy.expires_at, 80), "the expiry");
  expect(revokeIn(row)).toBeTruthy();

  const unused = await tokenRow(release.agent);
  const other = textOf(unused);
  expect(other).toMatch(/\bci\b/);
  expect(other).toMatch(/\bsend\b/i);
  expect(other).toMatch(/queue/i);
  expect(other).not.toMatch(/\breceive\b/i);
  expectOneOf(unused, channelForms(release.channels), "the channels");
  expectOneOf(unused, dayForms(release.created_at, 2), "the creation");
  expect(other).toMatch(/\bnever\b/i);
  expectOneOf(unused, dayForms(release.expires_at, 5), "the expiry");
  expect(revokeIn(unused)).toBeTruthy();
});

test("a space where the account is no admin shows none of its tokens and no failure", async () => {
  await openAccess(userEvent.setup());
  await tokenRow(release.agent);

  await waitFor(() =>
    expect(sent).toContain(`GET /spaces/${teamId}/service-tokens`),
  );
  expect(screen.queryAllByRole("alert")).toEqual([]);
  expect(
    screen.queryAllByRole("row").filter((row) => /\bci\b/.test(textOf(row))),
  ).toHaveLength(2);
});

test("Revoke on a token's row asks first, and Cancel revokes nothing", async () => {
  const user = userEvent.setup();
  const row = await openAccess(user);

  await user.click(revokeIn(row));
  const dialog = await question();
  expect(revocations()).toEqual([]);

  await user.click(within(dialog).getByRole("button", { name: /^cancel$/i }));
  await waitFor(() =>
    expect([
      ...screen.queryAllByRole("dialog"),
      ...screen.queryAllByRole("alertdialog"),
    ]).toEqual([]),
  );
  expect(revocations()).toEqual([]);
  expect(rowsOf(deploy.agent)).toHaveLength(1);
});

test("confirming revokes that token in its space, and only its row goes", async () => {
  const user = userEvent.setup();
  const row = await openAccess(user);

  await user.click(revokeIn(row));
  const dialog = await question();
  await user.click(within(dialog).getByRole("button", { name: /revoke/i }));

  await waitFor(() =>
    expect(revocations()).toEqual([
      `DELETE /spaces/${teamId}/service-tokens/${deploy.id}`,
    ]),
  );
  await waitFor(() => expect(rowsOf(deploy.agent)).toHaveLength(0));
  expect(rowsOf(release.agent)).toHaveLength(1);
  expect(
    sent.filter((request) => /^(DELETE|POST) \/account\//.test(request)),
  ).toEqual([]);
});

/** The bars of the page named for the service agents, of either role. */
const agentBars = () => [
  ...screen.queryAllByRole("progressbar", { name: /^service agents\b/i }),
  ...screen.queryAllByRole("meter", { name: /^service agents\b/i }),
];

/** Renders the web app signed in, opens Usage and chooses `blue-river`. */
async function openUsage(user: UserEvent) {
  render(<App />);
  await user.click(await screen.findByRole("link", { name: "Usage" }));
  await user.click(await screen.findByRole("button", { name: /^Space\b/ }));
  await user.click(
    await screen.findByRole("option", { name: /\bblue-river\b/ }),
  );
}

test("Usage shows the space's service agents of their limit, with a bar", async () => {
  await openUsage(userEvent.setup());

  await waitFor(() => expect(agentBars()).toHaveLength(1));
  const bar = agentBars()[0]!;
  const value = (aria: string, plain: string) =>
    Number(bar.getAttribute(aria) ?? bar.getAttribute(plain));
  expect([
    value("aria-valuenow", "value"),
    value("aria-valuemax", "max"),
  ]).toEqual([3, 10]);
  expect(textOf(screen.getByRole("main"))).toMatch(/\b3\s*of\s*10\b/);
  expect(sent).toContain(`GET /spaces/${teamId}/usage`);
});

test("without a limit, Usage shows the space's service agents with no bar and says no limit applies", async () => {
  agents = { count: 4 };
  await openUsage(userEvent.setup());

  await waitFor(() =>
    expect(textOf(screen.getByRole("main"))).toMatch(/service agents/i),
  );
  const main = textOf(screen.getByRole("main"));
  expect(main).toMatch(/no limit|unlimited/i);
  expect(agentBars()).toEqual([]);
  expect(main).not.toMatch(/\b4\s*of\s*\d/);
});
