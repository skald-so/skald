import { cleanup, render, screen } from "@testing-library/react";
import { http, HttpResponse } from "msw";
import { setupServer } from "msw/node";
import { afterAll, afterEach, beforeAll, expect, test } from "vitest";
import { App } from "./App.tsx";
import {
  getGetHealthMockHandler200,
  getGetHealthMockHandler503,
} from "./api/skald.msw.ts";
import { getGetHealthUrl } from "./api/skald.ts";

const server = setupServer();

beforeAll(() => server.listen({ onUnhandledFrame: "error" }));
afterEach(() => {
  cleanup();
  server.resetHandlers();
});
afterAll(() => server.close());

test("shows that the server is up", async () => {
  server.use(getGetHealthMockHandler200());
  render(<App />);
  expect(screen.getByRole("status").textContent).toBe("Server: checking…");
  expect(await screen.findByText("Server: ok")).toBeTruthy();
});

test("shows that the server is unavailable", async () => {
  server.use(getGetHealthMockHandler503());
  render(<App />);
  expect(await screen.findByText("Server: unavailable")).toBeTruthy();
});

test("shows that the server is unreachable when the request fails", async () => {
  server.use(http.get(`*${getGetHealthUrl()}`, () => HttpResponse.error()));
  render(<App />);
  expect(await screen.findByText("Server: unreachable")).toBeTruthy();
});

test("shows that the server is unreachable on a status the spec does not list", async () => {
  server.use(
    http.get(`*${getGetHealthUrl()}`, () =>
      HttpResponse.json({ status: "ok" }, { status: 500 }),
    ),
  );
  render(<App />);
  expect(await screen.findByText("Server: unreachable")).toBeTruthy();
});
