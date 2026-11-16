import { MemoryRouter } from "react-router";
import {
  getGetSessionResponseMock,
  getGetSignInProvidersResponseMock,
  getListSessionsResponseMock,
  getListSignInMethodsResponseMock,
} from "../api/skald.msw.ts";
import { AccessScreen } from "../app/Access.tsx";
import { AccountScreen } from "../app/Account.tsx";
import { Cabinet } from "../app/Cabinet.tsx";
import { Home } from "../app/Home.tsx";
import { accountPath } from "../app/names.ts";
import { SignInPage } from "../app/SignInPage.tsx";

// The web app's screens with fixture answers of the generated mocks.

const providers = getGetSignInProvidersResponseMock({
  providers: ["github", "google"],
  sign_up: "open",
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

/** The time the fixture sessions count from, so the screenshots stay the same. */
const now = Date.parse("2026-11-10T15:00:00Z");
const before = (minutes: number) =>
  new Date(now - minutes * 60_000).toISOString();
const day = 24 * 60;

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

/** The sign-in page of 03-sign-in.png, on a server with both providers and open sign-up. */
export function KitSignIn() {
  return <SignInPage providers={providers} returnTo="/" />;
}

/**
 * The cabinet of an account signed up on the web, with no space yet; at phone width with the side
 * menu open, which says how to make one.
 */
export function KitAccount() {
  return (
    <MemoryRouter>
      <Cabinet session={session} spaces={[]} onSignedOut={() => {}} menuOpen>
        <Home session={session} />
      </Cabinet>
    </MemoryRouter>
  );
}

/**
 * The Access section of 10-access.png in the cabinet; `asking` opens the question of "Sign out
 * everywhere else".
 */
export function KitAccess({ asking = false }: { asking?: boolean }) {
  return (
    <MemoryRouter initialEntries={["/access"]}>
      <Cabinet session={session} spaces={[personal]} onSignedOut={() => {}}>
        <AccessScreen
          sessions={sessions}
          now={now}
          onRevoke={() => {}}
          onEndOthers={async () => {}}
          defaultAsking={asking}
        />
      </Cabinet>
    </MemoryRouter>
  );
}

/**
 * The Account page in the cabinet, with both providers' links; `asking` opens the question of
 * Remove of the method this session signed in with.
 */
export function KitAccountPage({ asking = false }: { asking?: boolean }) {
  return (
    <MemoryRouter initialEntries={[accountPath]}>
      <Cabinet session={session} spaces={[personal]} onSignedOut={() => {}}>
        <AccountScreen
          session={session}
          methods={methods}
          providers={["github", "google"]}
          now={now}
          onRemove={async () => {}}
          defaultAsking={asking ? methods[0] : undefined}
        />
      </Cabinet>
    </MemoryRouter>
  );
}
