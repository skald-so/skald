/// <reference types="vitest/config" />
import { readFileSync } from "node:fs";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

// The dev and preview servers forward the paths in the spec to the API, a local skald-server by default.
const api = process.env.SKALD_API_URL ?? "http://127.0.0.1:8080";
const spec = JSON.parse(
  readFileSync(new URL("../api/openapi.json", import.meta.url), "utf8"),
) as { paths: Record<string, unknown> };
const proxy = Object.fromEntries(
  Object.keys(spec.paths).map((path) => [`/${path.split("/")[1]}`, api]),
);

export default defineConfig({
  plugins: [react()],
  server: { proxy },
  preview: { proxy },
  test: { environment: "jsdom" },
});
