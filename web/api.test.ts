// @vitest-environment node
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { expect, test } from "vitest";

const orval = join(
  dirname(createRequire(import.meta.url).resolve("orval/package.json")),
  "dist/bin/orval.mjs",
);

// Each check starts orval in a Node.js process: a loaded machine takes longer than vitest's 5 s.
const orvalCheck = { timeout: 30_000 };

/** Runs `orval --outdated`: compares src/api/ with the client generated from the spec. */
function checkClient(...args: string[]) {
  const run = spawnSync(process.execPath, [orval, "--outdated", ...args], {
    cwd: import.meta.dirname,
    encoding: "utf8",
  });
  return { status: run.status, output: run.stdout + run.stderr };
}

test(
  "the API client in src/api/ is generated from api/openapi.json",
  orvalCheck,
  () => {
    const { status, output } = checkClient();
    expect(
      status,
      `src/api/ is stale, run \`npm run generate\`:\n${output}`,
    ).toBe(0);
  },
);

test("a spec change makes the API client outdated", orvalCheck, () => {
  const dir = mkdtempSync(join(tmpdir(), "skald-web-"));
  try {
    const spec = JSON.parse(
      readFileSync(join(import.meta.dirname, "../api/openapi.json"), "utf8"),
    );
    spec.paths["/ping"] = {
      get: {
        operationId: "ping",
        responses: { "204": { description: "Pong." } },
      },
    };
    const input = join(dir, "openapi.json");
    writeFileSync(input, JSON.stringify(spec));

    const { status, output } = checkClient("--input", input);
    expect(status, output).toBe(1);
    expect(output).toContain("outdated: src/api/skald.ts");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
