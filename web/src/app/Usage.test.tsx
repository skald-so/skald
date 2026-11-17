// The Usage section when the switcher chooses another space before the answer for the space before
// comes: only the chosen space's usage shows.
import { cleanup, render, screen } from "@testing-library/react";
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
import type { ListedSpace, SpaceUsage } from "../api/skald.schemas.ts";
import { ChosenSpace } from "./space.ts";
import { Usage } from "./Usage.tsx";

const first: ListedSpace = {
  space: "01968a2e-0000-7000-8000-000000000701",
  name: "first",
  personal: true,
};
const second: ListedSpace = {
  space: "01968a2e-0000-7000-8000-000000000702",
  name: "second",
  personal: false,
};

/** The usage of a space with `messages` of 1,000 this month. */
const usage = (messages: number): SpaceUsage => ({
  monthly_messages: { count: messages, limit: 1_000 },
  message_size: { limit: 65_536 },
  mailbox_retention: { limit: 604_800 },
});

/** How each space's usage is answered, by space ID: held until released, or failed. */
let answers: Record<string, Promise<Response>>;

/** An answer of `body` held until the returned function releases it. */
function held(body: SpaceUsage): [Promise<Response>, () => void] {
  let release!: () => void;
  const answer = new Promise<Response>((resolve) => {
    release = () => resolve(HttpResponse.json(body));
  });
  return [answer, release];
}

const failed = () =>
  Promise.resolve(
    HttpResponse.json(
      { type: "https://skald.so/problems/internal", status: 500 },
      { status: 500 },
    ),
  );

const server = setupServer(
  http.get("*/account/usage", () =>
    HttpResponse.json({
      owned_spaces: { count: 1, limit: 2, spaces: [first] },
      devices: { count: 1, limit: 50 },
    }),
  ),
  http.get(
    "*/spaces/:space/usage",
    ({ params }) => answers[String(params.space)],
  ),
);

beforeAll(() => server.listen({ onUnhandledFrame: "error" }));
beforeEach(() => {
  answers = {};
});
afterEach(cleanup);
afterAll(() => server.close());

/** The section for `space`, as the cabinet renders it. */
const section = (space: ListedSpace) => (
  <ChosenSpace value={space}>
    <Usage onSignedOut={() => {}} />
  </ChosenSpace>
);

/** The messages this month the section shows; `undefined` without its bar. */
const messages = () =>
  screen
    .queryByRole("meter", { name: /^Messages this month/ })
    ?.getAttribute("aria-valuenow") ?? undefined;

const loading = () => screen.queryByText("Loading usage") !== null;

test("a late answer for the space chosen before does not replace the chosen space's usage", async () => {
  const [late, release] = held(usage(111));
  answers[first.space] = late;
  answers[second.space] = Promise.resolve(HttpResponse.json(usage(222)));
  const { rerender } = render(section(first));
  rerender(section(second));
  await expect.poll(messages).toBe("222");

  release();
  await late;
  // The late answer has had its turn to show.
  await new Promise((resolve) => setTimeout(resolve, 50));
  expect(messages()).toBe("222");
  expect(loading()).toBe(false);
});

test("while the chosen space's usage loads, the space before's does not show", async () => {
  answers[first.space] = Promise.resolve(HttpResponse.json(usage(111)));
  const [pending, release] = held(usage(222));
  answers[second.space] = pending;
  const { rerender } = render(section(first));
  await expect.poll(messages).toBe("111");

  rerender(section(second));
  expect(messages()).toBeUndefined();
  expect(loading()).toBe(true);

  release();
  await expect.poll(messages).toBe("222");
});

test("a failure for the space before does not show for the chosen space", async () => {
  answers[first.space] = failed();
  const [pending, release] = held(usage(222));
  answers[second.space] = pending;
  const { rerender } = render(section(first));
  await screen.findByText("The usage did not load.");

  rerender(section(second));
  expect(screen.queryByText("The usage did not load.")).toBeNull();
  expect(loading()).toBe(true);

  release();
  await expect.poll(messages).toBe("222");
  expect(screen.queryByText("The usage did not load.")).toBeNull();
});
