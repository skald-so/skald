import { MemoryRouter } from "react-router";
import {
  getGetAccountDeletionResponseMock,
  getGetAccountUsageResponseMock,
  getGetSessionResponseMock,
  getGetSignInProvidersResponseMock,
  getGetSpaceUsageResponseMock,
  getListAccountDevicesResponseMock,
  getListServiceTokensResponseMock,
  getListSessionsResponseMock,
  getListSignInMethodsResponseMock,
  getListSpaceInvitesResponseMock,
  getListSpaceMembersResponseMock,
} from "../api/skald.msw.ts";
import { AccessScreen } from "../app/Access.tsx";
import { AccountScreen } from "../app/Account.tsx";
import { Cabinet } from "../app/Cabinet.tsx";
import { InvitePage } from "../app/InvitePage.tsx";
import { MachinesScreen } from "../app/Machines.tsx";
import { MembersScreen } from "../app/Members.tsx";
import { accountPath } from "../app/names.ts";
import { SignInPage } from "../app/SignInPage.tsx";
import { UsageScreen } from "../app/Usage.tsx";

// The web app's screens with fixture answers of the generated mocks.

const providers = getGetSignInProvidersResponseMock({
  providers: ["github", "google"],
  sign_up: "open",
  privacy_policy_url: "https://skald.example/privacy",
});

const session = getGetSessionResponseMock({
  account: "01968a2e-0000-7000-8000-000000000001",
  username: "ana",
  github_login: "ana",
  sign_in_method: "github",
});

const personal = {
  space: "01968a2e-0000-7000-8000-000000000101",
  name: "ana",
  personal: true,
};

/** The team space of 09-members.png. */
const team = {
  space: "01968a2e-0000-7000-8000-000000000102",
  name: "team-a",
  personal: false,
};

/** The time the fixture sessions count from, so the screenshots stay the same. */
const now = Date.parse("2026-11-10T15:00:00Z");
const before = (minutes: number) =>
  new Date(now - minutes * 60_000).toISOString();
const day = 24 * 60;

const { invites } = getListSpaceInvitesResponseMock({
  invites: [
    {
      invite: "01968a2e-0000-7000-8000-000000000811",
      role: "member",
      username: "ana",
      device_name: "ana-laptop",
      created_at: before(2 * day),
      expires_at: before(-6 * day),
      acceptance_waiting: false,
    },
    {
      invite: "01968a2e-0000-7000-8000-000000000812",
      role: "admin",
      username: "sam",
      device_name: "sam-workstation",
      created_at: before(day),
      expires_at: before(-4 * day),
      acceptance_waiting: true,
      acceptance_handle: "lee",
    },
  ],
});
const teamInvites = {
  rows: invites.map((invite) => ({ space: team, invite })),
  now,
  revoke: async () => {},
};

/** The sessions of 10-access.png: this browser, two machines and another browser. */
const { sessions } = getListSessionsResponseMock({
  sessions: [
    {
      session: "01968a2e-0000-7000-8000-0000000002a1",
      kind: "web",
      browser: "Chrome",
      os: "macOS",
      started_at: before(0),
      last_used_at: before(0),
      expires_at: before(-30 * day),
      current: true,
    },
    {
      session: "01968a2e-0000-7000-8000-0000000002a2",
      kind: "device",
      device_name: "ana-laptop",
      started_at: before(39 * day),
      last_used_at: before(1),
      current: false,
    },
    {
      session: "01968a2e-0000-7000-8000-0000000002a3",
      kind: "device",
      device_name: "ana-cloud",
      started_at: before(21 * day),
      last_used_at: before(180),
      current: false,
    },
    {
      session: "01968a2e-0000-7000-8000-0000000002a4",
      kind: "web",
      browser: "Firefox",
      os: "Windows",
      started_at: before(2 * day),
      last_used_at: before(day),
      expires_at: before(-28 * day),
      current: false,
    },
  ],
});

/**
 * The tokens of team-a's service agents: one used an hour ago, one never used, of two channels and
 * expiring soon.
 */
const { tokens } = getListServiceTokensResponseMock({
  tokens: [
    {
      id: "01968a2e-0000-7000-8000-0000000004a1",
      device: "a06e13fb-72c9-8e01-a4b5-6c7d8e9f0a03",
      owner: "ci",
      agent: "deploy-bot",
      permissions: ["send", "receive"],
      channels: ["01968a2e-0000-7000-8000-0000000005a1"],
      created_at: before(9 * day),
      last_used_at: before(60),
      expires_at: before(-81 * day),
    },
    {
      id: "01968a2e-0000-7000-8000-0000000004a2",
      device: "0b7e5a31-6c2d-8f40-9e1a-2b3c4d5e6f06",
      owner: "ci",
      agent: "release-bot",
      permissions: ["send", "queues"],
      channels: [
        "01968a2e-0000-7000-8000-0000000005a1",
        "01968a2e-0000-7000-8000-0000000005a2",
      ],
      created_at: before(2 * day),
      last_used_at: null,
      expires_at: before(-5 * day),
    },
  ],
});

/** The account's sign-in methods: GitHub, which this session signed in with, and Google. */
const { methods } = getListSignInMethodsResponseMock({
  methods: [
    {
      method: "01968a2e-0000-7000-8000-0000000003a1",
      provider: "github",
      linked_at: before(39 * day),
      github_login: "ana",
      current: true,
    },
    {
      method: "01968a2e-0000-7000-8000-0000000003a2",
      provider: "google",
      linked_at: before(2 * day),
      current: false,
    },
  ],
});

/** The devices of 08-machines.png, with a machine waiting for linking and a revoked one. */
const { devices } = getListAccountDevicesResponseMock({
  devices: [
    {
      device: "7f3a91c2-0b5e-84d8-9e1f-0a2b3c4d5e01",
      name: "ana-laptop",
      kind: "client",
      registered_at: before(39 * day),
      state: "linked",
    },
    {
      device: "e2c70a9d-3b16-84a2-8c3d-1e2f3a4b5c02",
      name: "ana-cloud",
      kind: "client",
      registered_at: before(21 * day),
      state: "linked",
    },
    {
      device: "a06e13fb-72c9-8e01-a4b5-6c7d8e9f0a03",
      name: "deploy-bot",
      kind: "service",
      registered_at: before(9 * day),
      state: "linked",
    },
    {
      device: "19be4f02-d7a3-8680-b1c2-d3e4f5a6b704",
      name: "ana-mini",
      kind: "client",
      registered_at: before(60),
      state: "waiting",
    },
    {
      device: "5d2a7c10-e8b4-839f-9a0b-1c2d3e4f5a05",
      name: "ana-desktop",
      kind: "client",
      registered_at: before(400 * day),
      state: "revoked",
    },
  ],
});

/**
 * The members of 09-members.png: the account, the only admin, and two members, one of them without
 * a GitHub login.
 */
const { members } = getListSpaceMembersResponseMock({
  members: [
    {
      handle: "ana",
      role: "admin",
      github_login: "ana",
      linked_devices: 4,
      joined_at: before(39 * day),
      you: true,
    },
    {
      handle: "lee-park",
      role: "member",
      linked_devices: 2,
      joined_at: before(6 * day),
    },
    {
      handle: "sam-ortiz",
      role: "member",
      github_login: "sortiz",
      linked_devices: 1,
      joined_at: before(day),
    },
  ],
});

/** The usage of 11-usage.png on the cloud's limits: the account owns its personal space and team-a. */
const accountUsage = getGetAccountUsageResponseMock({
  owned_spaces: {
    count: 2,
    limit: 2,
    spaces: [
      { space: personal.space, name: personal.name },
      { space: team.space, name: team.name },
    ],
  },
  devices: { count: 4, limit: 50 },
});
const spaceUsage = getGetSpaceUsageResponseMock({
  monthly_messages: { count: 12_480, limit: 10_000_000 },
  message_size: { limit: 64 * 1024 },
  mailbox_retention: { limit: 7 * 86_400 },
  history_retention: { limit: 90 * 86_400 },
  service_agents: { count: 2, limit: 10 },
});

/**
 * The sign-in page of 03-sign-in.png, on a server with both providers, open sign-up and a privacy
 * policy.
 */
export function KitSignIn() {
  return <SignInPage providers={providers} returnTo="/" />;
}

export function KitInvite() {
  return <InvitePage />;
}

/**
 * The cabinet of an account signed up on the web, with no space and no device yet; at phone width
 * with the side menu open, which says how to make one.
 */
export function KitAccount() {
  return (
    <MemoryRouter>
      <Cabinet session={session} spaces={[]} onSignedOut={() => {}} menuOpen>
        <MachinesScreen username={session.username} devices={[]} now={now} />
      </Cabinet>
    </MemoryRouter>
  );
}

/** The Machines section of 08-machines.png in the cabinet, the page it opens on. */
export function KitMachines() {
  return (
    <MemoryRouter>
      <Cabinet session={session} spaces={[personal]} onSignedOut={() => {}}>
        <MachinesScreen
          username={session.username}
          devices={devices}
          now={now}
        />
      </Cabinet>
    </MemoryRouter>
  );
}

/** The Members section of 09-members.png in the cabinet, for the team space the switcher chose. */
export function KitMembers() {
  return (
    <MemoryRouter initialEntries={["/members"]}>
      <Cabinet
        session={session}
        spaces={[personal, team]}
        defaultSpace={team.space}
        onSignedOut={() => {}}
      >
        <MembersScreen
          space={team.name}
          members={members}
          invites={teamInvites}
          now={now}
        />
      </Cabinet>
    </MemoryRouter>
  );
}

/** The Usage section of 11-usage.png in the cabinet, for the team space the switcher chose. */
export function KitUsage() {
  return (
    <MemoryRouter initialEntries={["/usage"]}>
      <Cabinet
        session={session}
        spaces={[personal, team]}
        defaultSpace={team.space}
        onSignedOut={() => {}}
      >
        <UsageScreen
          space={team.name}
          account={accountUsage}
          usage={spaceUsage}
        />
      </Cabinet>
    </MemoryRouter>
  );
}

/**
 * The Access section of 10-access.png in the cabinet, with the tokens of team-a's service agents;
 * `asking` opens the question of "Sign out everywhere else".
 */
export function KitAccess({ asking = false }: { asking?: boolean }) {
  return (
    <MemoryRouter initialEntries={["/access"]}>
      <Cabinet
        session={session}
        spaces={[personal, team]}
        onSignedOut={() => {}}
      >
        <AccessScreen
          sessions={sessions}
          tokens={tokens.map((token) => ({ space: team, token }))}
          invites={teamInvites}
          now={now}
          onRevoke={() => {}}
          onEndOthers={async () => {}}
          onRevokeToken={async () => {}}
          defaultAsking={asking}
        />
      </Cabinet>
    </MemoryRouter>
  );
}

/**
 * The Account page in the cabinet, with both providers' links and the account's deletion: `view`
 * `confirm` opens the question of Remove of the method this session signed in with, `blocked` has
 * team-a block the deletion, and `delete` opens the question of Delete.
 */
export function KitAccountPage({
  view,
}: {
  view?: "confirm" | "blocked" | "delete";
}) {
  const blocked = view === "blocked";
  return (
    <MemoryRouter initialEntries={[accountPath]}>
      <Cabinet
        session={session}
        spaces={blocked ? [personal, team] : [personal]}
        onSignedOut={() => {}}
      >
        <AccountScreen
          session={session}
          methods={methods}
          providers={["github", "google"]}
          now={now}
          onRemove={async () => {}}
          defaultAsking={view === "confirm" ? methods[0] : undefined}
          deletion={getGetAccountDeletionResponseMock({
            deletes: [{ space: personal.space, name: personal.name }],
            blocks: blocked ? [{ space: team.space, name: team.name }] : [],
          })}
          onDelete={async () => {}}
          defaultDeleting={view === "delete"}
        />
      </Cabinet>
    </MemoryRouter>
  );
}
