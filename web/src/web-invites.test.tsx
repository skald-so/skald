// The app's Members, Access and invite-link pages against the web API.
import {
  cleanup,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
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
import { App } from "./App.tsx";
import { getSkaldAPIMock } from "./api/skald.msw.ts";

const acme = "01968a2e-0000-7000-8000-000000000801";
const globex = "01968a2e-0000-7000-8000-000000000802";
const other = "01968a2e-0000-7000-8000-000000000803";
const spaces = [
  { space: acme, name: "acme", personal: false },
  { space: globex, name: "globex", personal: false },
  { space: other, name: "other-team", personal: false },
];
const member = {
  handle: "ana",
  role: "admin",
  linked_devices: 1,
  joined_at: "2025-03-08T12:00:00Z",
};

const dayMs = 86_400_000;
const daysFromNow = (days: number) =>
  new Date(Date.now() + days * dayMs).toISOString();

interface Invite {
  invite: string;
  role: "member" | "admin";
  username: string;
  device_name: string;
  created_at: string;
  expires_at: string;
  acceptance_waiting: boolean;
  acceptance_handle?: string;
}

const open: Invite = {
  invite: "01968a2e-0000-7000-8000-000000000811",
  role: "member",
  username: "jodie-garcia",
  device_name: "Jodie's laptop",
  created_at: daysFromNow(-10),
  expires_at: daysFromNow(16),
  acceptance_waiting: false,
};
const waiting: Invite = {
  invite: "01968a2e-0000-7000-8000-000000000812",
  role: "admin",
  username: "kai-chen",
  device_name: "Kai's desktop",
  created_at: daysFromNow(-4),
  expires_at: daysFromNow(5),
  acceptance_waiting: true,
  acceptance_handle: "tess",
};
const remote: Invite = {
  ...waiting,
  invite: "01968a2e-0000-7000-8000-000000000813",
};

let signedIn: boolean;
let invites: Record<string, Invite[]>;
let sent: Request[];
const unauthenticated = () => new HttpResponse(null, { status: 401 });
const forbidden = () =>
  HttpResponse.json(
    {
      type: "https://skald.so/problems/not-allowed",
      title: "Not allowed",
      status: 403,
      detail: "The space's admin must make this request.",
    },
    { status: 403, headers: { "content-type": "application/problem+json" } },
  );
const server = setupServer(
  http.get("*/session", () =>
    signedIn
      ? HttpResponse.json({
          account: "01968a2e-0000-7000-8000-000000000001",
          username: member.handle,
          sign_in_method: "github",
        })
      : unauthenticated(),
  ),
  http.get("*/auth/providers", () =>
    HttpResponse.json({ providers: ["github"], sign_up: "open" }),
  ),
  http.get("*/spaces", () => HttpResponse.json({ spaces })),
  http.get("*/spaces/:space/members", ({ params }) =>
    HttpResponse.json({
      members: [
        { ...member, role: params.space === other ? "member" : "admin" },
      ],
    }),
  ),
  http.get("*/spaces/:space/invites", ({ params }) => {
    const list = invites[String(params.space)];
    return list === undefined
      ? forbidden()
      : HttpResponse.json({ invites: list });
  }),
  http.delete("*/spaces/:space/invites/:invite", ({ params }) => {
    const space = String(params.space);
    const list = invites[space];
    if (list === undefined) return forbidden();
    invites[space] = list.filter((item) => item.invite !== params.invite);
    return new HttpResponse(null, { status: 204 });
  }),
  http.get("*/account/sessions", () => HttpResponse.json({ sessions: [] })),
  http.get("*/spaces/:space/service-tokens", () =>
    HttpResponse.json({ tokens: [] }),
  ),
  ...getSkaldAPIMock(),
);
server.events.on("request:start", ({ request }) => sent.push(request));

beforeAll(() => server.listen({ onUnhandledFrame: "error" }));
beforeEach(() => {
  signedIn = true;
  invites = { [acme]: [open, waiting], [globex]: [remote] };
  sent = [];
  window.history.replaceState(null, "", "/");
});
afterEach(cleanup);
afterAll(() => server.close());

function textOf(element: HTMLElement) {
  const words: string[] = [];
  const walker = document.createTreeWalker(element, NodeFilter.SHOW_TEXT);
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    words.push(node.textContent ?? "");
  }
  return words.join(" ");
}

const adminDevice = /\badmin(?:['’]s)?\s+device\b/i;

function inviteRows(role: Invite["role"], identifying: string) {
  const main = within(screen.getByRole("main"));
  const entries = [
    ...main.queryAllByRole("row"),
    ...main.queryAllByRole("listitem"),
    ...main.queryAllByRole("article"),
  ];
  return entries.filter((row) => {
    const text = textOf(row);
    return (
      text.includes(identifying) &&
      within(row).queryByRole("button", { name: /^revoke\b/i }) !== null &&
      new RegExp(`\\b${role}\\b`, "i").test(text.replace(adminDevice, ""))
    );
  });
}

function inviteRow(role: Invite["role"], identifying: string) {
  const matching = inviteRows(role, identifying);
  expect(matching).toHaveLength(1);
  return matching[0]!;
}

function expectDate(row: HTMLElement, at: string, days: number) {
  const date = new Date(at);
  const forms = [
    at.slice(0, 10),
    ...["en-US", "en-GB"].map((locale) =>
      date.toLocaleDateString(locale, { month: "short", day: "numeric" }),
    ),
  ];
  const text = textOf(row);
  const relative = new RegExp(
    `\\b(${days - 1}|${days}|${days + 1})\\s*d(ays?)?\\b`,
    "i",
  );
  expect(
    forms.some((form) => text.includes(form)) || relative.test(text),
    `${at}: ${text}`,
  ).toBe(true);
}

function expectState(row: HTMLElement, invite: Invite) {
  const text = textOf(row);
  expect(adminDevice.test(text)).toBe(invite.acceptance_waiting);
  if (!invite.acceptance_waiting) {
    expect(text).toMatch(/newcomer|waiting for acceptance/i);
  }
  expectDate(row, invite.created_at, invite === open ? 10 : 4);
  expectDate(row, invite.expires_at, invite === open ? 16 : 5);
}

async function showInvites(section: "Members" | "Access") {
  const user = userEvent.setup();
  render(<App />);
  await user.click(await screen.findByRole("link", { name: section }));
  if (section === "Members") {
    await user.click(await screen.findByRole("button", { name: /^Space\b/ }));
    await user.click(await screen.findByRole("option", { name: /\bacme\b/ }));
    await screen.findByRole("heading", { name: /^invite links$/i });
  }
  return user;
}

test("Members lists the creator, device, dates, role and both waiting states for a space's invite links", async () => {
  const user = await showInvites("Members");
  await waitFor(() => {
    for (const invite of [open, waiting]) {
      const row = inviteRow(invite.role, invite.device_name);
      expect(textOf(row)).toContain(invite.username);
      expectState(row, invite);
    }
  });
  for (const role of ["button", "link"]) {
    expect(screen.queryByRole(role, { name: /create.*invite/i })).toBeNull();
  }
  await user.click(await screen.findByRole("button", { name: /^Space\b/ }));
  await user.click(await screen.findByRole("option", { name: /other-team/i }));
  await waitFor(() => {
    expect(
      screen.queryByRole("heading", { name: /^invite links$/i }),
    ).toBeNull();
    expect(screen.queryByRole("button", { name: /^revoke\b/i })).toBeNull();
  });
});

test("Access lists invite links across the spaces the account administers", async () => {
  await showInvites("Access");
  await waitFor(() => {
    expectState(inviteRow("member", "acme"), open);
    expectState(inviteRow("admin", "acme"), waiting);
    expectState(inviteRow("admin", "globex"), remote);
  });
  expect(
    within(screen.getByRole("main")).queryByText(/other-team/i),
  ).toBeNull();
  expect(screen.queryByRole("alert")).toBeNull();
});

const revocations = () =>
  sent
    .filter((request) => request.method === "DELETE")
    .map((request) => new URL(request.url).pathname);

test.each([
  ["Members", open.device_name, waiting.device_name],
  ["Access", "acme", "acme"],
] as const)(
  "%s asks before revoking a link and removes only the confirmed row",
  async (section, openMarker, waitingMarker) => {
    const user = await showInvites(section);
    const revoke = async () => {
      const row = await waitFor(() => inviteRow(open.role, openMarker));
      await user.click(within(row).getByRole("button", { name: /^revoke\b/i }));
      let dialog: HTMLElement | null;
      await waitFor(() => {
        dialog =
          screen.queryByRole("dialog") ?? screen.queryByRole("alertdialog");
        expect(dialog).not.toBeNull();
      });
      return dialog!;
    };

    let dialog = await revoke();
    expect(revocations()).toEqual([]);
    await user.click(within(dialog).getByRole("button", { name: /^cancel$/i }));
    expect(revocations()).toEqual([]);
    expect(inviteRow(open.role, openMarker)).toBeTruthy();

    dialog = await revoke();
    await user.click(within(dialog).getByRole("button", { name: /revoke/i }));
    await waitFor(() => {
      expect(revocations()).toEqual([`/spaces/${acme}/invites/${open.invite}`]);
      expect(inviteRows(open.role, openMarker)).toEqual([]);
    });
    expect(inviteRow(waiting.role, waitingMarker)).toBeTruthy();
  },
);

test.each([false, true])(
  "an invite link offers the terminal instructions with signed-in status %s",
  async (session) => {
    signedIn = session;
    const fragment = "secret-fragment".repeat(6);
    window.history.replaceState(null, "", `/invite/${open.invite}#${fragment}`);
    render(<App />);

    await waitFor(() => {
      const text = screen.getByRole("main").textContent ?? "";
      expect(text).toMatch(/skald accept-invite/);
      expect(text).toMatch(/paste/i);
      expect(text).toMatch(/terminal/i);
      for (const value of [
        open.invite,
        fragment,
        open.username,
        open.device_name,
      ]) {
        expect(text).not.toContain(value);
      }
    });
    expect(sent.map((request) => request.url).join("\n")).not.toContain(
      fragment,
    );
  },
);
