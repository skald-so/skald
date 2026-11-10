// Opens the kit page from a Vite dev server in headless Chrome: for the kit tests and screenshots.
import { execFileSync, spawn } from "node:child_process";
import { randomUUID } from "node:crypto";
import { createRequire } from "node:module";
import { dirname } from "node:path";
import { chromium, type Browser, type Page } from "playwright-core";
import { createServer, type ViteDevServer } from "vite";

export type Theme = "light" | "dark";

export const themes: Theme[] = ["light", "dark"];
/** Check widths: the design screenshots and a phone. */
export const widths = [1440, 360];

/** The screens of the kit page: the kit itself, the cabinet, the cabinet with the phone side menu open. */
export type Screen = "kit" | "cabinet" | "cabinet-menu";

export interface KitView {
  page: Page;
  /** URLs of every request the page made. */
  requests: string[];
}

export interface Kit {
  /** The dev server origin, e.g. `http://127.0.0.1:41234`. */
  origin: string;
  browser: Browser;
  /**
   * Opens a screen of the kit page, default the kit itself, `width` pixels wide with the system set
   * to `system`, rendered and with fonts loaded. `theme` adds the URL parameter that forces a theme.
   */
  open(view: {
    width: number;
    system: Theme;
    theme?: Theme;
    screen?: Screen;
  }): Promise<KitView>;
  close(): Promise<void>;
}

/**
 * The Chrome of the screenshots: Chromium of the Playwright image, the same on every machine, so the
 * references hold on macOS and in CI. Its version is the playwright-core version in package.json.
 */
export const pinnedChrome =
  "mcr.microsoft.com/playwright:v1.65.0-noble@sha256:bc4fbb329a25287ed23e9216c97063646abdd1bf2da669a9895fa3e2078b2302";

/**
 * Flags of the pinned Chrome. Without partial raster a repaint redraws whole tiles, so the pixels
 * do not depend on whether a tile was drawn before the page settled: with it, a repainted text's
 * LCD fringe and a rounded corner could come out one step off.
 */
const pinnedChromeArgs = ["--disable-partial-raster"];

/**
 * The browser server in the container: `playwright run-server` for one client, which launches the
 * pinned Chrome with `pinnedChromeArgs` in place of any flags the client asks for. No client picks
 * the program or its flags. Until the container is removed, any container that reaches the
 * published port can still connect and drive a Chrome: `GET /json` gives the endpoint path, and on
 * Docker Desktop every container reaches the port through `host.docker.internal`.
 *
 * `chromium.launchServer` takes fixed flags without this override, but its Chrome cannot reach the
 * dev server on this machine: `exposeNetwork` works only for a browser that `run-server` launches.
 * The override replaces a private method of this pinned playwright-core version; the flag check in
 * kit-references.test.ts fails if it stops working.
 */
const browserServer = `const { PlaywrightServer } = require("/playwright-core/lib/coreBundle").remote;
const server = new PlaywrightServer({ mode: "default", path: "/", maxConnections: 1 });
const launch = server._initLaunchBrowserMode.bind(server);
server._initLaunchBrowserMode = (browser, proxy, options, id) =>
  launch(browser, proxy, { ...options, args: ${JSON.stringify(pinnedChromeArgs)} }, id);
server.listen(3000, "0.0.0.0").then((endpoint) => console.log("Listening on " + endpoint));`;

/**
 * Starts the dev server and headless Chrome. `pinned` takes the Chrome of `pinnedChrome` in Docker;
 * otherwise the local Google Chrome, or the binary in `SKALD_CHROME`.
 */
export async function startKit({ pinned = false } = {}): Promise<Kit> {
  const server: ViteDevServer = await createServer({
    root: import.meta.dirname,
    logLevel: "warn",
    server: { host: "127.0.0.1", port: 0, strictPort: true },
  });
  let browser: Browser;
  try {
    await server.listen();
    browser = await (pinned ? connectPinnedChrome() : launchChrome());
  } catch (error) {
    await server.close();
    throw error;
  }
  const origin = new URL(server.resolvedUrls!.local[0]!).origin;

  const kit: Kit = {
    origin,
    browser,
    async open({ width, system, theme, screen = "kit" }) {
      const page = await browser.newPage({
        viewport: { width, height: 900 },
        colorScheme: system,
      });
      const requests: string[] = [];
      page.on("request", (request) => requests.push(request.url()));
      const query = new URLSearchParams({
        ...(screen !== "kit" && { screen }),
        ...(theme && { theme }),
      }).toString();
      await page.goto(`${origin}/kit.html${query ? `?${query}` : ""}`);
      await page.locator("h1").waitFor();
      await page.evaluate(async () => {
        await document.fonts.ready;
      });
      return { page, requests };
    },
    async close() {
      await browser.close();
      await server.close();
    },
  };
  try {
    // The first load pre-bundles the dependencies, transforms every module and starts a renderer:
    // seconds on a busy machine. It happens here, in the setup, not in the first test.
    await (await kit.open({ width: widths[0]!, system: "light" })).page.close();
  } catch (error) {
    await kit.close();
    throw error;
  }
  return kit;
}

/**
 * A screenshot with animations stopped and the text caret hidden, as a PNG: of the full page, or of
 * the window with `window`.
 */
export function screenshot(
  page: Page,
  { window = false } = {},
): Promise<Buffer> {
  return page.screenshot({
    fullPage: !window,
    animations: "disabled",
    caret: "hide",
  });
}

async function launchChrome(): Promise<Browser> {
  const executablePath = process.env.SKALD_CHROME;
  try {
    return await chromium.launch(
      executablePath ? { executablePath } : { channel: "chrome" },
    );
  } catch (error) {
    throw new Error(
      "Headless Chrome did not start: install Google Chrome or set SKALD_CHROME to a Chrome or Chromium binary.",
      { cause: error },
    );
  }
}

/**
 * Runs the Playwright browser server of `pinnedChrome` in Docker and connects to it. The server runs
 * the local playwright-core, mounted read-only; the browser reaches the dev server on this machine
 * through the connection. The container is removed when the browser closes.
 */
async function connectPinnedChrome(): Promise<Browser> {
  const require = createRequire(import.meta.url);
  const playwright = dirname(require.resolve("playwright-core/package.json"));
  const version = (
    require("playwright-core/package.json") as { version: string }
  ).version;
  if (!pinnedChrome.includes(`:v${version}-`)) {
    throw new Error(
      `The pinned Chrome image ${pinnedChrome} is not for playwright-core ${version}: update pinnedChrome in kit-browser.ts and the references.`,
    );
  }
  const name = `skald-kit-chrome-${randomUUID()}`;
  const docker = spawn(
    "docker",
    [
      "run",
      "--rm",
      "--init",
      "--ipc=host",
      "--platform=linux/amd64",
      `--name=${name}`,
      "--user=pwuser",
      "--publish=127.0.0.1::3000",
      `--volume=${playwright}:/playwright-core:ro`,
      pinnedChrome,
      "node",
      "--eval",
      browserServer,
    ],
    { stdio: ["ignore", "pipe", "pipe"] },
  );
  const remove = () => {
    process.off("exit", remove);
    try {
      execFileSync("docker", ["rm", "--force", name], { stdio: "ignore" });
    } catch {
      // Already removed.
    }
  };
  process.on("exit", remove);
  try {
    await new Promise<void>((resolve, reject) => {
      let output = "";
      const fail = (reason: string) =>
        reject(
          new Error(
            `The pinned Chrome did not start in Docker (${reason}); Docker must be running.\n${output}`,
          ),
        );
      docker.on("error", (error) => fail(error.message));
      docker.on("exit", (code) => fail(`docker exited with ${code}`));
      docker.stderr.on("data", (data: Buffer) => (output += data));
      docker.stdout.on("data", (data: Buffer) => {
        output += data;
        if (output.includes("Listening on")) resolve();
      });
    });
    const address = execFileSync("docker", ["port", name, "3000/tcp"], {
      encoding: "utf8",
    })
      .split("\n")[0]!
      .trim();
    const browser = await chromium.connect(`ws://${address}/`, {
      exposeNetwork: "<loopback>",
      timeout: 60_000,
    });
    browser.on("disconnected", remove);
    return browser;
  } catch (error) {
    remove();
    throw error;
  }
}
