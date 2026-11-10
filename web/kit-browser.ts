// Opens the kit page from a Vite dev server in headless Chrome: for the kit tests and screenshots.
import { chromium, type Browser, type Page } from "playwright-core";
import { createServer, type ViteDevServer } from "vite";

export type Theme = "light" | "dark";

export const themes: Theme[] = ["light", "dark"];
/** Check widths: the design screenshots and a phone. */
export const widths = [1440, 360];

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
   * Opens the kit page `width` pixels wide with the system set to `system`, rendered and with fonts
   * loaded. `theme` adds the URL parameter that forces a theme.
   */
  open(view: { width: number; system: Theme; theme?: Theme }): Promise<KitView>;
  close(): Promise<void>;
}

/** Starts the dev server and headless Chrome; `SKALD_CHROME` overrides the Chrome binary. */
export async function startKit(): Promise<Kit> {
  const server: ViteDevServer = await createServer({
    root: import.meta.dirname,
    logLevel: "warn",
    server: { host: "127.0.0.1", port: 0, strictPort: true },
  });
  let browser: Browser;
  try {
    await server.listen();
    browser = await launchChrome();
  } catch (error) {
    await server.close();
    throw error;
  }
  const origin = new URL(server.resolvedUrls!.local[0]!).origin;

  return {
    origin,
    browser,
    async open({ width, system, theme }) {
      const page = await browser.newPage({
        viewport: { width, height: 900 },
        colorScheme: system,
      });
      const requests: string[] = [];
      page.on("request", (request) => requests.push(request.url()));
      await page.goto(`${origin}/kit.html${theme ? `?theme=${theme}` : ""}`);
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
