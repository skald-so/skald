import { readFileSync } from "node:fs";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";
import { configDefaults } from "vitest/config";

// The dev and preview servers forward the paths in the spec to the API, a local skald-server by default.
const api = process.env.SKALD_API_URL ?? "http://127.0.0.1:8080";
const spec = JSON.parse(
  readFileSync(new URL("../api/openapi.json", import.meta.url), "utf8"),
) as { paths: Record<string, unknown> };
const proxy = Object.fromEntries(
  Object.keys(spec.paths).map((path) => [`/${path.split("/")[1]}`, api]),
);

/** The tests in headless Chrome: the local one, and the pinned one in Docker. */
const browserTests = [
  "kit.test.ts",
  "cabinet.test.ts",
  "kit-references.test.ts",
];

export default defineConfig({
  plugins: [react()],
  server: { proxy },
  preview: { proxy },
  test: {
    environment: "jsdom",
    projects: [
      {
        extends: true,
        test: {
          name: "unit",
          exclude: [...configDefaults.exclude, ...browserTests],
        },
      },
      // One file at a time, after the unit tests. A container that Docker starts or removes changes
      // the network, and Chrome aborts its requests in flight with ERR_NETWORK_CHANGED: a page in
      // the local Chrome of one file would not render while another file starts the pinned Chrome.
      // The browsers do not compete for the processor either.
      {
        extends: true,
        test: {
          name: "browser",
          include: browserTests,
          fileParallelism: false,
        },
      },
    ],
  },
});
