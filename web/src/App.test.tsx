// The web app when it cannot load the account: the server health, a new attempt; and a page the
// cabinet does not have.
import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { delay, http, HttpResponse } from "msw";
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
  getGetHealthMockHandler200,
  getGetHealthMockHandler503,
  getGetSessionMockHandler200,
  getGetSessionMockHandler500,
  getGetSpacesMockHandler200,
  getGetSpacesMockHandler500,
  getListAccountDevicesMockHandler200,
} from "./api/skald.msw.ts";
import { getGetHealthUrl, getGetSessionUrl } from "./api/skald.ts";

const server = setupServer();

beforeAll(() => server.listen({ onUnhandledFrame: "error" }));
beforeEach(() => window.history.replaceState(null, "", "/"));
afterEach(() => {
  cleanup();
  server.resetHandlers();
});
afterAll(() => server.close());

/** The server fails the account's session and spaces: the web app does not load. */
const failing = [getGetSessionMockHandler500(), getGetSpacesMockHandler500()];

/** A web session of `ana` with no space and no device. */
const signedIn = [
  getGetSessionMockHandler200({
    account: "01968a2e-0000-7000-8000-000000000001",
    username: "ana",
    sign_in_method: "github",
  }),
  getGetSpacesMockHandler200({ spaces: [] }),
  getListAccountDevicesMockHandler200({ devices: [] }),
];

test("when the web app does not load, shows that the server is up", async () => {
  server.use(
    ...failing,
    http.get(`*${getGetHealthUrl()}`, async () => {
      await delay(100);
      return HttpResponse.json({ status: "ok" });
    }),
  );
  render(<App />);
  expect(await screen.findByText("Server: checking…")).toBeTruthy();
  expect(await screen.findByText("Server: ok")).toBeTruthy();
});

test("when the web app does not load, shows that the server is unavailable", async () => {
  server.use(...failing, getGetHealthMockHandler503());
  render(<App />);
  expect(await screen.findByText("Server: unavailable")).toBeTruthy();
});

test("when the requests fail, shows that the server is unreachable", async () => {
  server.use(
    http.get(`*${getGetSessionUrl()}`, () => HttpResponse.error()),
    getGetSpacesMockHandler500(),
    http.get(`*${getGetHealthUrl()}`, () => HttpResponse.error()),
  );
  render(<App />);
  expect(await screen.findByText("Server: unreachable")).toBeTruthy();
});

test("shows that the server is unreachable on a health status the spec does not list", async () => {
  server.use(
    ...failing,
    http.get(`*${getGetHealthUrl()}`, () =>
      HttpResponse.json({ status: "ok" }, { status: 500 }),
    ),
  );
  render(<App />);
  expect(await screen.findByText("Server: unreachable")).toBeTruthy();
});

test("Try again loads the web app again", async () => {
  const user = userEvent.setup();
  server.use(...failing, getGetHealthMockHandler200());
  render(<App />);
  const retry = await screen.findByRole("button", { name: "Try again" });
  server.resetHandlers(...signedIn);
  await user.click(retry);
  expect(await screen.findByRole("button", { name: "Account" })).toBeTruthy();
  expect(screen.getByRole("heading", { level: 1 }).textContent).toBe(
    "Machines",
  );
});

test("a page the cabinet does not have says so and links to the cabinet", async () => {
  window.history.replaceState(null, "", "/no-such-page");
  server.use(...signedIn);
  render(<App />);
  expect((await screen.findByRole("heading", { level: 1 })).textContent).toBe(
    "Not found",
  );
  expect(
    screen.getByRole("link", { name: "Open the cabinet" }).getAttribute("href"),
  ).toBe("/");
});
