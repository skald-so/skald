// The Access section: a token's row counts its last use and expiry from when the tokens load, also
// when the sessions fail or answer after the tokens.
import { cleanup, render, screen, within } from "@testing-library/react";
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
import type { ListedSpace, ServiceToken } from "../api/skald.schemas.ts";
import { Access } from "./Access.tsx";

const hourMs = 3_600_000;
const dayMs = 24 * hourMs;

/** The date-time `offset` milliseconds from now. */
const at = (offset: number) => new Date(Date.now() + offset).toISOString();

const team: ListedSpace = {
  space: "01968a2e-0000-7000-8000-000000000801",
  name: "team",
  personal: false,
};

/** `ci/deploy-bot`'s token, used 3 h ago, 80 days left. */
const token = (): ServiceToken => ({
  id: "01968a2e-0000-7000-8000-000000000802",
  device: "01968a2e-0000-7000-8000-000000000803",
  owner: "ci",
  agent: "deploy-bot",
  permissions: ["send"],
  channels: ["01968a2e-0000-7000-8000-000000000804"],
  created_at: at(-10 * dayMs),
  last_used_at: at(-3 * hourMs - 60_000),
  expires_at: at(80 * dayMs + hourMs),
});

/** The answer to `GET /account/sessions`. */
let sessions: Promise<Response>;

const server = setupServer(
  http.get("*/account/sessions", () => sessions),
  http.get("*/spaces/:space/service-tokens", () =>
    HttpResponse.json({ tokens: [token()] }),
  ),
);

beforeAll(() => server.listen({ onUnhandledFrame: "error" }));
beforeEach(() => {
  sessions = new Promise(() => {});
});
afterEach(cleanup);
afterAll(() => server.close());

/** The token's row once it shows. */
async function tokenRow() {
  const table = await screen.findByRole("table", {
    name: "Service agent tokens",
  });
  return within(table).getByRole("row", { name: /deploy-bot/ });
}

test("a token's row shows its last use and expiry when the sessions fail", async () => {
  sessions = Promise.resolve(
    HttpResponse.json(
      { type: "https://skald.so/problems/internal", status: 500 },
      { status: 500 },
    ),
  );
  render(<Access spaces={[team]} onSignedOut={() => {}} />);

  await screen.findByText("The sessions did not load.");
  const shown = (await tokenRow()).textContent;
  expect(shown).toContain("3 h ago");
  expect(shown).toContain("in 80 days");
});

test("a token's row shows its last use and expiry before the sessions answer", async () => {
  render(<Access spaces={[team]} onSignedOut={() => {}} />);

  const shown = (await tokenRow()).textContent;
  expect(shown).toContain("3 h ago");
  expect(shown).toContain("in 80 days");
});
