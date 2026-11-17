// The Machines section of the web app, rendered whole against the API's mocks: the generated ones,
// and the answers these tests give for the session, its spaces and the account's devices.
//
// The API, as these tests use it, beside the spec:
// - `GET /account/devices`: `{"devices": [{"device", "name", "kind": "client" | "browser" |
//   "service", "registered_at", "state": "linked" | "waiting" | "revoked"}]}`.
//
// The web app, as these tests use it: the cabinet opens on Machines, which the side menu's link
// "Machines" also opens. Its table has a row per device that names it and says its kind, when it
// registered, as days ago or its date, and its state: "Linked", "Waiting for linking" or "Revoked".
// The column headers "Device", "Kind", "Registered" and "Status" sort the rows by a click; another
// click on the same one reverses them. Without devices, the page says to sign in a machine with
// `skald`.
import { App } from "./App.tsx";
import { getGetSessionUrl, getGetSpacesUrl } from "./api/skald.ts";
import { getSkaldAPIMock } from "./api/skald.msw.ts";
import { cleanup, render, screen, within } from "@testing-library/react";
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

type Kind = "client" | "browser" | "service";
type State = "linked" | "waiting" | "revoked";

interface Device {
  device: string;
  name: string;
  kind: Kind;
  registered_at: string;
  state: State;
}

/** The account's devices: `GET`. */
const devicesUrl = "/account/devices";

const hour = 60 * 60_000;
const day = 24 * hour;

/** A device named `name` of `kind`, registered `days` days and an hour ago, in `state`. */
function device(
  at: number,
  name: string,
  kind: Kind,
  days: number,
  state: State,
): Device {
  return {
    device: `01968a2e-0000-7000-8000-00000000040${at}`,
    name,
    kind,
    registered_at: new Date(Date.now() - days * day - hour).toISOString(),
    state,
  };
}

/** In the order the server gives them, which no column's order is. */
const fixture: Device[] = [
  device(1, "ana-old", "client", 90, "revoked"),
  device(2, "deploy-bot", "service", 10, "linked"),
  device(3, "ana-laptop", "client", 40, "linked"),
  device(4, "ana-mini", "client", 1, "waiting"),
  device(5, "ana-cloud", "client", 3, "linked"),
];

/** The devices the mocks give now. */
let devices: Device[] = [];

const server = setupServer(
  http.get(`*${getGetSessionUrl()}`, () =>
    HttpResponse.json({
      account: "01968a2e-0000-7000-8000-000000000001",
      username: "ana",
      sign_in_method: "github",
      github_login: "ana-gh",
    }),
  ),
  http.get(`*${getGetSpacesUrl()}`, () =>
    HttpResponse.json({
      spaces: [
        {
          space: "01968a2e-0000-7000-8000-000000000101",
          name: "ana",
          personal: true,
        },
      ],
    }),
  ),
  http.get("*/auth/providers", () =>
    HttpResponse.json({ sign_up: "open", providers: ["github"] }),
  ),
  http.get(`*${devicesUrl}`, () => HttpResponse.json({ devices })),
  ...getSkaldAPIMock(),
);

beforeAll(() => server.listen({ onUnhandledFrame: "error" }));
beforeEach(() => {
  window.history.replaceState(null, "", "/");
  devices = [...fixture];
});
afterEach(cleanup);
afterAll(() => server.close());

/** The device of `fixture` whose name the row `row` holds; fails without one. */
function deviceIn(row: HTMLElement): Device {
  const found = fixture.filter((device) =>
    (row.textContent ?? "").includes(device.name),
  );
  expect(found, `one device in ${row.textContent}`).toHaveLength(1);
  return found[0]!;
}

/** The rows of the page's tables that name a device, in order. */
function deviceRows(): HTMLElement[] {
  return screen
    .queryAllByRole("row")
    .filter((row) =>
      fixture.some((device) => (row.textContent ?? "").includes(device.name)),
    );
}

/** The devices the rows name, in order. */
const shown = () => deviceRows().map(deviceIn);

/** Renders the web app signed in at `/` and waits for the cabinet. */
async function cabinet() {
  render(<App />);
  await screen.findByRole("button", { name: "Account" });
}

/** Renders the cabinet and waits until Machines shows every device of the mocks. */
async function machines() {
  await cabinet();
  await screen.findAllByText(fixture[0]!.name);
  await expect.poll(() => deviceRows().length).toBe(devices.length);
}

/** The words of the states the row `row` says. */
function statesIn(row: HTMLElement): string[] {
  const text = (row.textContent ?? "").toLowerCase();
  return ["linked", "waiting", "revoked"].filter((word) => text.includes(word));
}

/** The forms in which a row may say that `device` registered `days` days and an hour ago. */
function registeredForms(device: Device, days: number): RegExp {
  const date = new Date(device.registered_at);
  const forms = [
    date.toISOString().slice(0, 10),
    `(?<!\\d)(${days}|${days + 1})\\s*d(ays?)?\\b`,
  ];
  for (const locale of ["en-US", "en-GB"])
    forms.push(
      date.toLocaleDateString(locale, { month: "short", day: "numeric" }),
    );
  return new RegExp(forms.join("|"), "i");
}

test("the cabinet opens on Machines: a row per device with its name, kind, registration and state", async () => {
  await machines();

  expect(screen.getByRole("link", { name: "Machines" })).toBeTruthy();
  expect(
    shown()
      .map((device) => device.name)
      .sort(),
  ).toEqual(fixture.map((device) => device.name).sort());
  for (const row of deviceRows()) {
    const device = deviceIn(row);
    expect(statesIn(row), device.name).toEqual([device.state]);
    const kind =
      device.kind === "service" ? /service|agent/i : /machine|client/i;
    expect(row.textContent, device.name).toMatch(kind);
  }
  const laptop = deviceRows().find(
    (row) => deviceIn(row).name === "ana-laptop",
  )!;
  expect(laptop.textContent).toMatch(registeredForms(fixture[2]!, 40));
});

/** The orders of `devices` by each sortable column: the header's name and the device's value. */
const columns: [string, RegExp, (device: Device) => string | number][] = [
  ["Device", /^device\b/i, (device) => device.name],
  ["Kind", /^kind\b/i, (device) => device.kind],
  [
    "Registered",
    /^registered\b/i,
    (device) => Date.parse(device.registered_at),
  ],
  ["Status", /^(status|state)\b/i, (device) => device.state],
];

/** Whether `values` go up, or down, never the other way. */
function ordered(values: (string | number)[]): boolean {
  const up = values.every((value, at) => at === 0 || values[at - 1]! <= value);
  const down = values.every(
    (value, at) => at === 0 || values[at - 1]! >= value,
  );
  return up || down;
}

/** Whether each value of `values` comes in one run. */
function grouped(values: (string | number)[]): boolean {
  return values.every(
    (value, at) =>
      at === 0 ||
      values[at - 1] === value ||
      !values.slice(0, at).includes(value),
  );
}

/** Clicks the column header whose name matches `name`. */
async function sortBy(user: UserEvent, name: RegExp) {
  await user.click(screen.getByRole("button", { name }));
}

test.each(columns)(
  "a click on %s sorts the rows by it; another reverses them",
  async (_, name, value) => {
    const user = userEvent.setup();
    await machines();
    const values = () => shown().map(value);
    // The server's order is not this column's.
    expect(grouped(values()) && ordered(values())).toBe(false);

    await sortBy(user, name);
    const first = values();
    expect(grouped(first), `${first}`).toBe(true);
    if (new Set(first).size === first.length)
      expect(ordered(first), `${first}`).toBe(true);

    await sortBy(user, name);
    expect(values()).toEqual([...first].reverse());
  },
);

test("without devices, Machines says to sign in a machine with skald", async () => {
  devices = [];
  await cabinet();

  const main = screen.getByRole("main");
  const says = (_: string, element: Element | null) => {
    const said = (node: Element) => {
      const text = (node.textContent ?? "").toLowerCase();
      return ["sign", "machine", "skald"].every((word) => text.includes(word));
    };
    return (
      element !== null && said(element) && ![...element.children].some(said)
    );
  };
  expect(await within(main).findAllByText(says)).not.toHaveLength(0);
  expect(deviceRows()).toHaveLength(0);
});
