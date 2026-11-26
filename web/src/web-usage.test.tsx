// The Usage section of the web app, rendered whole against the API's mocks: the generated ones, and
// the answers these tests give for the session, its spaces, the account's usage and each space's.
//
// The API, as these tests use it, beside the spec:
// - `GET /account/usage`: `{"owned_spaces": {"count", "limit"?, "spaces": [{"space", "name"}]},
//   "devices": {"count", "limit"?}}`, with `owned_spaces.free_only` when plans are on;
// - `GET /spaces/{space}/usage`: `{"monthly_messages": {"count", "limit"?}, "message_size":
//   {"limit"?}, "mailbox_retention": {"limit"?}, "history_retention": {"limit"?},
//   "service_agents": {"count", "limit"?}}`, the size in bytes and the retentions in seconds.
// A `limit` is absent while no limit applies.
//
// The web app, as these tests use it: the side menu's link "Usage" opens the section, the page's
// main area, for the space the switcher chose, the button "Space …" and its options. Each count
// with a limit shows as "<count> of <limit>", with a bar, an element of role `progressbar` or
// `meter` whose accessible name starts with what it counts, "Spaces", "Devices" or "Messages this
// month", and whose value is the count and maximum the limit. The section lists the spaces the
// account owns by name, and says the largest message, as "64 KB", and how long an undelivered
// message is kept, as "7 days". A count without a limit has no bar, and the section says "no
// limit" or "unlimited". With plans off it names no plan. Without a space, it says to sign in a
// machine with `skald`, and still shows the owned spaces.
import { App } from "./App.tsx";
import { getGetSessionUrl, getGetSpacesUrl } from "./api/skald.ts";
import { getSkaldAPIMock } from "./api/skald.msw.ts";
import { cleanup, render, screen } from "@testing-library/react";
import userEvent, { type UserEvent } from "@testing-library/user-event";
import { http, HttpResponse, type JsonBodyType } from "msw";
import { setupServer } from "msw/node";
import {
  afterAll,
  afterEach,
  beforeAll,
  beforeEach,
  expect,
  test,
} from "vitest";

interface Counted {
  count: number;
  limit?: number;
}

interface AccountUsage {
  owned_spaces: Counted & {
    free_only?: boolean;
    spaces: { space: string; name: string }[];
  };
  devices: Counted;
}

interface SpaceUsage {
  monthly_messages: Counted;
  message_size: { limit?: number };
  mailbox_retention: { limit?: number };
  history_retention: { limit?: number };
  service_agents: Counted;
}

const ownId = "01968a2e-0000-7000-8000-000000000601";
const teamId = "01968a2e-0000-7000-8000-000000000602";
const otherId = "01968a2e-0000-7000-8000-000000000603";

/** The account's personal space, `kovac`, and the team space `blue-river`. */
const own = { space: ownId, name: "kovac" };
const team = { space: teamId, name: "blue-river" };

/** A limit of `value`, absent when it is `undefined`. */
const limited = (value: number | undefined) =>
  value === undefined ? {} : { limit: value };

/** `count` of `limit`, or without a limit. */
const counted = (count: number, limit?: number): Counted => ({
  count,
  ...limited(limit),
});

/** The cloud's message size and retentions: 64 KB, 7 days and 90 days. */
const kb64 = 64 * 1024;
const days7 = 7 * 86_400;
const days90 = 90 * 86_400;

/** A space's usage: `messages` of `limit`, with the cloud's size and retentions while it has one. */
function spaceUsage(messages: number, limit?: number): SpaceUsage {
  const cloud = limit !== undefined;
  return {
    monthly_messages: counted(messages, limit),
    message_size: limited(cloud ? kb64 : undefined),
    mailbox_retention: limited(cloud ? days7 : undefined),
    history_retention: limited(cloud ? days90 : undefined),
    service_agents: counted(2, cloud ? 10 : undefined),
  };
}

/** The account's spaces as the mocks give them now. */
let spaces: { space: string; name: string; personal?: boolean }[] = [];
/** The account's usage as the mocks give it now. */
let account: AccountUsage;
/** Each space's usage as the mocks give it now, by space ID. */
let usage: Record<string, SpaceUsage> = {};
/** The path of each `GET` request the web app sent, without its query. */
let gets: string[] = [];

/** The JSON answers to `GET` requests, by path: the session, its spaces and the account's usage. */
const answers: [string, () => JsonBodyType][] = [
  [
    getGetSessionUrl(),
    () => ({
      account: "01968a2e-0000-7000-8000-000000000001",
      username: "kovac",
      github_login: "kovac-gh",
      sign_in_method: "github",
    }),
  ],
  [getGetSpacesUrl(), () => ({ spaces })],
  ["/auth/providers", () => ({ providers: ["github"], sign_up: "open" })],
  ["/account/usage", () => account],
];

const server = setupServer(
  ...answers.map(([path, body]) =>
    http.get(`*${path}`, () => HttpResponse.json(body())),
  ),
  http.get("*/spaces/:space/usage", ({ params }) => {
    const found = usage[String(params.space)];
    const missing = { type: "https://skald.so/problems/no-space", status: 404 };
    return found
      ? HttpResponse.json(found)
      : HttpResponse.json(missing, { status: 404 });
  }),
  ...getSkaldAPIMock(),
);
server.events.on("request:start", ({ request }) => {
  if (request.method === "GET") gets.push(new URL(request.url).pathname);
});

beforeAll(() => server.listen({ onUnhandledFrame: "error" }));
beforeEach(() => {
  spaces = [{ ...own, personal: true }, team];
  account = {
    owned_spaces: { ...counted(1, 2), spaces: [own] },
    devices: counted(4, 10),
  };
  usage = {
    [ownId]: spaceUsage(5, 1_000_000),
    [teamId]: spaceUsage(12_480, 1_000_000),
  };
  gets = [];
  window.history.replaceState(null, "", "/");
});
afterEach(cleanup);
afterAll(() => server.close());

/** Whether the web app sent `GET <path>`. */
const asked = (path: string) => gets.includes(path);

/** The text of the page's main area, a space between the texts of its elements. */
function mainText(): string {
  const texts = document.createTreeWalker(
    screen.getByRole("main"),
    NodeFilter.SHOW_TEXT,
  );
  const parts: string[] = [];
  while (texts.nextNode()) parts.push(texts.currentNode.textContent ?? "");
  return parts.join(" ");
}

/** A pattern of `value` as a page may write it, its thousands separated or not. */
const number = (value: number) =>
  String(value).replace(/\B(?=(\d{3})+(?!\d))/g, "[,\\s\\u00a0\\u202f]?");

/** Whether the main area says `count` of `limit`. */
const saysOf = (count: number, limit: number) =>
  new RegExp(
    `(^|\\D)${number(count)}\\s*of\\s*${number(limit)}(\\D|$)`,
    "i",
  ).test(mainText());

/** Whether the main area holds the number `value` on its own. */
const holds = (value: number) =>
  new RegExp(`(^|\\D)${number(value)}(\\D|$)`).test(mainText());

/** The bars of the page whose name matches `name`, of either role. */
const barsNamed = (name: RegExp) => [
  ...screen.queryAllByRole("progressbar", { name }),
  ...screen.queryAllByRole("meter", { name }),
];

/** Every bar of the page. */
const allBars = () => barsNamed(/.*/);

/** The value and maximum of the one bar whose name matches `name`; `undefined` unless one does. */
function bar(name: RegExp): [number, number] | undefined {
  const found = barsNamed(name);
  if (found.length !== 1) return undefined;
  const element = found[0]!;
  const attribute = (aria: string, plain: string) =>
    Number(element.getAttribute(aria) ?? element.getAttribute(plain));
  return [
    attribute("aria-valuenow", "value"),
    attribute("aria-valuemax", "max"),
  ];
}

const spacesBar = /^spaces\b/i;
const devicesBar = /^devices\b/i;
const messagesBar = /^messages this month\b/i;

/** Chooses the space `name` in the switcher. */
async function choose(user: UserEvent, name: string) {
  const switcher = await screen.findByRole("button", { name: /^Space\b/ });
  await user.click(switcher);
  const named = new RegExp(`\\b${name}\\b`);
  const option = await screen.findByRole("option", { name: named });
  await user.click(option);
}

/** Renders the cabinet, opens Usage and chooses the space `name`. */
async function usageOf(user: UserEvent, name: string) {
  render(<App />);
  await user.click(await screen.findByRole("link", { name: "Usage" }));
  await choose(user, name);
}

test("Usage shows each count under its limit with a bar, the other limits as text and the owned spaces", async () => {
  const user = userEvent.setup();
  await usageOf(user, team.name);

  await expect.poll(() => bar(messagesBar)).toEqual([12_480, 1_000_000]);
  expect(bar(spacesBar)).toEqual([1, 2]);
  expect(bar(devicesBar)).toEqual([4, 10]);
  for (const [count, limit] of [
    [12_480, 1_000_000],
    [1, 2],
    [4, 10],
  ] as const)
    expect(saysOf(count, limit), `${count} of ${limit}: ${mainText()}`).toBe(
      true,
    );
  expect(mainText()).toMatch(/64\s*(KB|KiB)/);
  expect(mainText()).toMatch(/7\s*days/i);
  expect(mainText()).toMatch(/\bkovac\b/);
  expect(mainText()).not.toMatch(/no limit|unlimited/i);
  expect(mainText()).not.toMatch(
    /\b(plans?|free|pro|business|enterprise|billing|pricing|upgrade|subscription)\b/i,
  );
  expect(asked("/account/usage")).toBe(true);
  expect(asked(`/spaces/${teamId}/usage`)).toBe(true);
});

test("a count at its limit fills its bar", async () => {
  const user = userEvent.setup();
  account = {
    owned_spaces: { ...counted(2, 2), spaces: [own, team] },
    devices: counted(10, 10),
  };
  usage[teamId] = spaceUsage(1_000_000, 1_000_000);
  await usageOf(user, team.name);

  await expect.poll(() => bar(messagesBar)).toEqual([1_000_000, 1_000_000]);
  expect(bar(spacesBar)).toEqual([2, 2]);
  expect(bar(devicesBar)).toEqual([10, 10]);
  for (const [count, limit] of [
    [1_000_000, 1_000_000],
    [2, 2],
    [10, 10],
  ] as const)
    expect(saysOf(count, limit), `${count} of ${limit}: ${mainText()}`).toBe(
      true,
    );
  expect(mainText()).toMatch(/\bkovac\b/);
  expect(mainText()).toMatch(/\bblue-river\b/);
});

test("the Free quota is labeled separately from the list of all owned spaces", async () => {
  const user = userEvent.setup();
  account.owned_spaces = {
    ...counted(1, 2),
    free_only: true,
    spaces: [own, team],
  };
  await usageOf(user, team.name);

  await expect.poll(() => bar(/^free spaces\b/i)).toEqual([1, 2]);
  const owned = screen.getByRole("region", { name: "Spaces you own" });
  const names = Array.from(
    owned.querySelectorAll("li"),
    (item) => item.textContent,
  );
  expect(names).toEqual([own.name, team.name]);
});

test("without the settings, each count shows that no limit applies, without a bar", async () => {
  const user = userEvent.setup();
  const other = { space: otherId, name: "green-hill" };
  spaces = [...spaces, other];
  account = {
    owned_spaces: { ...counted(3), spaces: [own, team, other] },
    devices: counted(37),
  };
  usage[teamId] = spaceUsage(12_480);
  await usageOf(user, team.name);

  await expect.poll(() => holds(12_480)).toBe(true);
  expect(mainText()).toMatch(/no limit|unlimited/i);
  expect(holds(37), mainText()).toBe(true);
  for (const name of ["kovac", "blue-river", "green-hill"])
    expect(mainText()).toMatch(new RegExp(`\\b${name}\\b`));
  expect(allBars()).toEqual([]);
  expect(mainText()).not.toMatch(/\bof\s*\d/i);
  expect(mainText()).not.toMatch(/\d\s*(KB|KiB)\b/);
});

test("choosing another space shows its messages this month", async () => {
  const user = userEvent.setup();
  await usageOf(user, team.name);
  await expect.poll(() => bar(messagesBar)).toEqual([12_480, 1_000_000]);

  await choose(user, own.name);
  await expect.poll(() => bar(messagesBar)).toEqual([5, 1_000_000]);
  expect(saysOf(5, 1_000_000), mainText()).toBe(true);
  expect(asked(`/spaces/${ownId}/usage`)).toBe(true);

  await choose(user, team.name);
  await expect.poll(() => bar(messagesBar)).toEqual([12_480, 1_000_000]);
});

test("without a space, Usage says to sign in a machine with skald and still shows the owned spaces", async () => {
  const user = userEvent.setup();
  spaces = [];
  account = {
    owned_spaces: { ...counted(1, 2), spaces: [] },
    devices: counted(0, 10),
  };
  render(<App />);
  await user.click(await screen.findByRole("link", { name: "Usage" }));

  await expect.poll(() => bar(spacesBar)).toEqual([1, 2]);
  const text = mainText().toLowerCase();
  expect(
    ["sign", "machine", "skald"].filter((word) => text.includes(word)),
  ).toEqual(["sign", "machine", "skald"]);
  expect(bar(messagesBar)).toBeUndefined();
  expect(gets.filter((path) => /^\/spaces\/[^/]+\/usage$/.test(path))).toEqual(
    [],
  );
});
