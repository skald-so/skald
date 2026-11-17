// The Members section of the web app, rendered whole against the API's mocks: the generated ones,
// and the answers these tests give for the session, its spaces and each space's members.
//
// The API, as these tests use it, beside the spec:
// - `GET /spaces/{space}/members`: `{"members": [{"handle", "role": "admin" | "member",
//   "github_login"?, "linked_devices", "joined_at"}]}`, `github_login` only for an account with a
//   GitHub method.
//
// The web app, as these tests use it: the side menu's link "Members" opens the section for the
// space the switcher chose, the button "Space …" and its options. Its table has a row per member
// that holds its handle, its role, "Admin" or "Member", its GitHub login if it has one, the number
// of its linked devices in a cell of its own and when it joined, as days ago or its date. The filter
// buttons "All", "Admins" and "Members", each with its count, show every member or those of one
// role; the section's search field keeps the members whose handle or login holds its text, without
// asking the server. The column headers "Person", "Role", "Devices" and "Joined" sort the rows by a
// click; another click on the same one reverses them. Nothing there changes a member or a role.
// Without a space, the section says to sign in a machine with `skald`.
import {
  afterAll,
  afterEach,
  beforeAll,
  beforeEach,
  expect,
  test,
} from "vitest";
import { App } from "./App.tsx";
import { getGetSessionUrl, getGetSpacesUrl } from "./api/skald.ts";
import { getSkaldAPIMock } from "./api/skald.msw.ts";
import { HttpResponse, http } from "msw";
import { setupServer } from "msw/node";
import { cleanup, render, screen, within } from "@testing-library/react";
import userEvent, { type UserEvent } from "@testing-library/user-event";

type Role = "admin" | "member";

interface Member {
  handle: string;
  role: Role;
  github_login?: string;
  linked_devices: number;
  joined_at: string;
}

const dayMs = 86_400_000;

/** A member who joined `days` days and two hours ago. */
function joined(
  handle: string,
  role: Role,
  linked_devices: number,
  days: number,
  github_login?: string,
): Member {
  const at = new Date(Date.now() - days * dayMs - 2 * 3_600_000);
  const member: Member = {
    handle,
    role,
    linked_devices,
    joined_at: at.toISOString(),
  };
  return github_login === undefined ? member : { ...member, github_login };
}

/** The members of `team-a`, in the order the server gives them, which no column's order is. */
const team: Member[] = [
  joined("lee-park", "member", 2, 12),
  joined("ana", "admin", 4, 40, "ana-gh"),
  joined("sam-ortiz", "member", 1, 7, "sortiz"),
  joined("kim-lab", "admin", 0, 3, "kimura-k"),
  joined("bo-yang", "member", 3, 20, "byang"),
];
/** The members of the account's personal space, `ana`. */
const own: Member[] = [joined("ana", "admin", 4, 60, "ana-gh")];

const teamId = "01968a2e-0000-7000-8000-000000000501";
const ownId = "01968a2e-0000-7000-8000-000000000502";

/** The account's spaces as the mocks give them now. */
let spaces: { space: string; name: string; personal?: boolean }[] = [];
/** Each request the web app sent, its method and its whole URL. */
let sent: string[] = [];

const answer = (body: object) => () => HttpResponse.json(body);
const server = setupServer(
  http.get(
    `*${getGetSessionUrl()}`,
    answer({
      account: "01968a2e-0000-7000-8000-000000000001",
      username: "ana",
      github_login: "ana-gh",
      sign_in_method: "github",
    }),
  ),
  http.get(`*${getGetSpacesUrl()}`, () => HttpResponse.json({ spaces })),
  http.get(
    "*/auth/providers",
    answer({ providers: ["github"], sign_up: "open" }),
  ),
  http.get("*/spaces/:space/members", ({ params }) => {
    const members = { [teamId]: team, [ownId]: own }[String(params.space)];
    return members === undefined
      ? new HttpResponse(null, { status: 404 })
      : HttpResponse.json({ members });
  }),
  ...getSkaldAPIMock(),
);
server.events.on("request:start", ({ request }) => {
  sent.push(`${request.method} ${request.url}`);
});

beforeAll(() => server.listen({ onUnhandledFrame: "error" }));
beforeEach(() => {
  window.history.replaceState(null, "", "/");
  sent = [];
  spaces = [
    { space: ownId, name: "ana", personal: true },
    { space: teamId, name: "team-a" },
  ];
});
afterEach(cleanup);
afterAll(() => server.close());

/** Whether the web app sent `GET <path>`, whatever its query. */
const asked = (path: string) =>
  sent.some((request) => {
    const [method, url] = request.split(" ");
    return method === "GET" && new URL(url!).pathname === path;
  });

/** The table rows that hold a handle of `members`, in order. */
function rowsOf(members: Member[]): HTMLElement[] {
  return screen
    .queryAllByRole("row")
    .filter((row) =>
      members.some((member) => row.textContent?.includes(member.handle)),
    );
}

/** The member of `members` whose handle `row` holds. */
function memberOf(row: HTMLElement, members: Member[]): Member {
  const held = members.filter((member) =>
    row.textContent?.includes(member.handle),
  );
  expect(held.map((member) => member.handle)).toHaveLength(1);
  return held[0]!;
}

/** The handles the rows of `members` hold, in order. */
const handles = (members: Member[]) =>
  rowsOf(members).map((row) => memberOf(row, members).handle);

/** Chooses the space `name` in the switcher. */
async function choose(user: UserEvent, name: string) {
  await user.click(await screen.findByRole("button", { name: /^Space\b/ }));
  await user.click(
    await screen.findByRole("option", { name: new RegExp(`\\b${name}\\b`) }),
  );
}

/** Renders the cabinet, opens Members and chooses `team-a`; waits for its rows. */
async function teamMembers(user: UserEvent) {
  render(<App />);
  await user.click(await screen.findByRole("link", { name: "Members" }));
  await choose(user, "team-a");
  await expect.poll(() => rowsOf(team).length).toBe(team.length);
}

/** The text of each cell of `row`, its row header included. */
const cellsOf = (row: HTMLElement) =>
  ["cell", "gridcell", "rowheader"]
    .flatMap((role) => within(row).queryAllByRole(role))
    .map((cell) => cell.textContent?.trim() ?? "");

/** Whether `text`, a row's, says `member` joined `days` days ago: as days ago or as its date. */
function saysJoined(text: string, member: Member, days: number): boolean {
  const at = new Date(member.joined_at);
  const dates = [
    at.toISOString().slice(0, 10),
    at.toLocaleDateString("en-US", { month: "short", day: "numeric" }),
    at.toLocaleDateString("en-GB", { day: "numeric", month: "short" }),
  ];
  const ago = new RegExp(`(^|\\D)(${days}|${days + 1}) ?d(ays?)?\\b`, "i");
  return dates.some((date) => text.includes(date)) || ago.test(text);
}

test("Members lists each member of the chosen space with its role, login, linked devices and when it joined", async () => {
  const user = userEvent.setup();
  await teamMembers(user);

  const days = new Map([
    ["lee-park", 12],
    ["ana", 40],
    ["sam-ortiz", 7],
    ["kim-lab", 3],
    ["bo-yang", 20],
  ]);
  for (const row of rowsOf(team)) {
    const member = memberOf(row, team);
    const text = cellsOf(row).join(" ");
    const roles = ["admin", "member"].filter((role) =>
      text.toLowerCase().includes(role),
    );
    expect(roles, member.handle).toEqual([member.role]);
    if (member.github_login !== undefined)
      expect(text, member.handle).toContain(member.github_login);
    const count = new RegExp(`^${member.linked_devices}( devices?)?$`, "i");
    expect(
      cellsOf(row).filter((cell) => count.test(cell)),
      member.handle,
    ).toHaveLength(1);
    expect(
      saysJoined(text, member, days.get(member.handle)!),
      `${member.handle}: ${text}`,
    ).toBe(true);
  }
  for (const login of ["ana-gh", "sortiz", "kimura-k", "byang"]) {
    const holding = rowsOf(team).filter((row) =>
      row.textContent?.includes(login),
    );
    expect(holding, login).toHaveLength(1);
  }

  const main = screen.getByRole("main");
  const changes = /remove|make\b.*\badmin|invite|change role/i;
  expect(within(main).queryAllByRole("button", { name: changes })).toEqual([]);
  expect(within(main).queryAllByRole("link", { name: changes })).toEqual([]);
  expect(asked(`/spaces/${teamId}/members`)).toBe(true);
});

test("the role filters show their counts and the members of one role", async () => {
  const user = userEvent.setup();
  await teamMembers(user);
  const filter = (name: RegExp) => screen.getByRole("button", { name });
  const all = filter(/^all\W*5$/i);
  const admins = filter(/^admins\W*2$/i);
  const members = filter(/^members\W*3$/i);

  await user.click(admins);
  await expect.poll(() => handles(team).sort()).toEqual(["ana", "kim-lab"]);
  await user.click(members);
  await expect
    .poll(() => handles(team).sort())
    .toEqual(["bo-yang", "lee-park", "sam-ortiz"]);
  await user.click(all);
  await expect.poll(() => handles(team).length).toBe(team.length);
});

test.each([
  ["a handle", "park", ["lee-park"]],
  ["a login", "sortiz", ["sam-ortiz"]],
  ["a login's part", "kimura", ["kim-lab"]],
])(
  "the search keeps the members whose handle or login holds %s, without asking the server",
  async (_, query, kept) => {
    const user = userEvent.setup();
    await teamMembers(user);
    const asked = sent.length;

    await user.type(
      within(screen.getByRole("main")).getByRole("searchbox"),
      query,
    );
    await expect.poll(() => handles(team).sort()).toEqual(kept);
    const since = sent.slice(asked);
    expect(since.filter((request) => request.includes(query))).toEqual([]);
    expect(since.filter((request) => request.includes("/members"))).toEqual([]);
  },
);

/** Each sortable column: its header's name and a member's value in it. */
const columns: [string, RegExp, (member: Member) => string | number][] = [
  ["Person", /^(person|handle|name)\b/i, (member) => member.handle],
  ["Role", /^role\b/i, (member) => member.role],
  ["Devices", /^devices\b/i, (member) => member.linked_devices],
  ["Joined", /^(joined|added)\b/i, (member) => Date.parse(member.joined_at)],
];

/** `values` sorted up. */
function sortedUp(values: (string | number)[]): (string | number)[] {
  return [...values].sort((a, b) => (a < b ? -1 : a > b ? 1 : 0));
}

/** Whether `values` are sorted up or down. */
function sorted(values: (string | number)[]): boolean {
  const up = sortedUp(values);
  const same = (other: (string | number)[]) =>
    other.every((value, at) => value === values[at]);
  return same(up) || same([...up].reverse());
}

test.each(columns)(
  "a click on %s sorts the rows by it; another reverses them",
  async (_, name, value) => {
    const user = userEvent.setup();
    await teamMembers(user);
    const values = () => rowsOf(team).map((row) => value(memberOf(row, team)));
    await user.click(screen.getByRole("button", { name }));
    const first = values();
    expect(sorted(first), `${first}`).toBe(true);

    await user.click(screen.getByRole("button", { name }));
    expect(values()).toEqual([...first].reverse());
  },
);

test("choosing another space shows its members", async () => {
  const user = userEvent.setup();
  await teamMembers(user);

  await choose(user, "ana");
  await expect.poll(() => rowsOf(team).length).toBe(1);
  expect(handles(own)).toEqual(["ana"]);
  const row = rowsOf(own)[0]!;
  expect(saysJoined(cellsOf(row).join(" "), own[0]!, 60)).toBe(true);
  expect(screen.getByRole("button", { name: /^all\W*1$/i })).toBeTruthy();
  expect(asked(`/spaces/${ownId}/members`)).toBe(true);

  await choose(user, "team-a");
  await expect.poll(() => rowsOf(team).length).toBe(team.length);
});

test("without a space, Members says to sign in a machine with skald", async () => {
  const user = userEvent.setup();
  spaces = [];
  render(<App />);
  await user.click(await screen.findByRole("link", { name: "Members" }));

  const main = screen.getByRole("main");
  await expect
    .poll(() => {
      const text = (main.textContent ?? "").toLowerCase();
      return ["sign", "machine", "skald"].filter((word) => text.includes(word));
    })
    .toEqual(["sign", "machine", "skald"]);
  expect(rowsOf(team)).toEqual([]);
  expect(sent.filter((request) => request.includes("/members"))).toEqual([]);
});
