// Opens the kit page from a Vite dev server in headless Chrome: for the kit tests and screenshots.
// The website's browser tests and screenshots take its Chrome, pages and checks too.
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

/**
 * The screens of the kit page: the kit itself, the cabinet, the cabinet with the phone side menu
 * open, and the web app's sign-in and invite link pages, cabinet of an account with no space,
 * Machines section, Members section, Access section and its question before "Sign out everywhere
 * else", Account page and its removal confirmation, the page with a space blocking deletion,
 * the deletion confirmation, and Usage section.
 */
export type Screen =
  | "kit"
  | "cabinet"
  | "cabinet-menu"
  | "sign-in"
  | "invite"
  | "account"
  | "machines"
  | "members"
  | "access"
  | "access-confirm"
  | "account-page"
  | "account-page-confirm"
  | "account-page-blocked"
  | "account-page-delete"
  | "usage";

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
 * otherwise Chrome Headless Shell of playwright-core, or the binary in `SKALD_CHROME`.
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
    open({ width, system, theme, screen = "kit" }) {
      const query = new URLSearchParams({
        ...(screen !== "kit" && { screen }),
        ...(theme && { theme }),
      }).toString();
      return openPage(
        browser,
        `${origin}/kit.html${query ? `?${query}` : ""}`,
        { width, system },
      );
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
 * Opens `url` in a new page `width` pixels wide with the system set to `system`, rendered, its h1
 * shown, and with fonts loaded. `permissions` grants the page browser permissions, e.g. the
 * clipboard's.
 */
export async function openPage(
  browser: Browser,
  url: string,
  {
    width,
    system,
    permissions = [],
  }: { width: number; system: Theme; permissions?: string[] },
): Promise<KitView> {
  const page = await browser.newPage({
    viewport: { width, height: 900 },
    colorScheme: system,
    permissions,
  });
  const requests: string[] = [];
  page.on("request", (request) => requests.push(request.url()));
  await page.goto(url);
  await page.locator("h1").waitFor();
  await page.evaluate(async () => {
    await document.fonts.ready;
  });
  return { page, requests };
}

/** The kit's accessibility check: axe-core's violations on the page, each with its elements. */
export async function accessibilityViolations(
  page: Page,
): Promise<{ rule: string; nodes: string[] }[]> {
  await page.addScriptTag({
    path: createRequire(import.meta.url).resolve("axe-core"),
  });
  return page.evaluate(async () => {
    const { axe } = window as unknown as {
      axe: typeof import("axe-core");
    };
    const results = await axe.run(document);
    return results.violations.map((violation) => ({
      rule: violation.id,
      nodes: violation.nodes.map((node) => node.target.join(" ")),
    }));
  });
}

/**
 * Tabs through the page until the focus leaves it or comes back, and gives each element focused
 * from the keyboard with its outline, `<style> <width> <color>`.
 */
export async function keyboardFocus(
  page: Page,
): Promise<{ element: string; outline: string }[]> {
  const focused: { element: string; outline: string }[] = [];
  for (let i = 0; i < 500; i++) {
    await page.keyboard.press("Tab");
    const focus = await page.evaluate(() => {
      const element = document.activeElement;
      if (!element || element === document.body) return null;
      if (element.hasAttribute("data-kit-focused")) return null;
      element.setAttribute("data-kit-focused", "");
      const style = getComputedStyle(element);
      return {
        element: element.outerHTML.slice(0, 100),
        outline: `${style.outlineStyle} ${style.outlineWidth} ${style.outlineColor}`,
      };
    });
    if (!focus) break;
    focused.push(focus);
  }
  return focused;
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

/** Starts Chrome Headless Shell of playwright-core, or the binary in `SKALD_CHROME`. */
export async function launchChrome(): Promise<Browser> {
  const executablePath = process.env.SKALD_CHROME;
  try {
    return await chromium.launch(executablePath ? { executablePath } : {});
  } catch (error) {
    throw new Error(
      "Chrome Headless Shell did not start: install it with `npx playwright-core install chromium-headless-shell` in web/, or set SKALD_CHROME to a Chrome or Chromium binary.",
      { cause: error },
    );
  }
}

/**
 * Runs the Playwright browser server of `pinnedChrome` in Docker and connects to it. The server runs
 * the local playwright-core, mounted read-only; the browser reaches the dev server on this machine
 * through the connection. The container is removed when the browser closes.
 */
export async function connectPinnedChrome(): Promise<Browser> {
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
