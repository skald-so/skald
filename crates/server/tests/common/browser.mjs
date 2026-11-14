// The person's browser for the integration tests: headless Chrome driven by playwright-core, loaded
// from the web UI's node_modules, the directory given as the only argument.
// Reads one JSON command per line on stdin, `{"id": <n>, "op": <op>, ...arguments}`, and answers
// each on stdout with `{"id": <n>, "ok": <result>}` or `{"id": <n>, "error": <message>}`; the
// first line is `{"ready": true}` once Chrome runs. Exits when stdin closes.
// Chrome resolves every *.test host to 127.0.0.1, so a test reaches a local server under a name of
// its choice: another name is another site, and a name other than localhost is not a secure context
// over plain HTTP. It is the local Google Chrome, or the binary in SKALD_CHROME, as for the kit tests.
import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import http from "node:http";
import https from "node:https";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createInterface } from "node:readline";

const send = (message) => process.stdout.write(`${JSON.stringify(message)}\n`);

let chromium;
try {
  ({ chromium } = createRequire(join(process.argv[2], "package.json"))(
    "playwright-core",
  ));
} catch (error) {
  console.error(
    `playwright-core is missing in ${process.argv[2]}: run npm ci there. ${error}`,
  );
  process.exit(1);
}

let browser;
try {
  const executablePath = process.env.SKALD_CHROME;
  browser = await chromium.launch({
    ...(executablePath ? { executablePath } : { channel: "chrome" }),
    // Below the 200 s browser.rs waits for the ready line: a slow start under load still succeeds,
    // or fails with playwright-core's own error.
    timeout: 180_000,
    args: [
      "--host-resolver-rules=MAP *.test 127.0.0.1",
      // Plain HTTP stays plain: no upgrade of a navigation to HTTPS.
      "--disable-features=HttpsUpgrades",
    ],
  });
} catch (error) {
  console.error(
    `Headless Chrome did not start: install Google Chrome or set SKALD_CHROME to a Chrome or Chromium binary. ${error}`,
  );
  process.exit(1);
}

/** Pages by ID, each in a browser context of its own: one person's browser. */
const pages = new Map();
/** The requests each echo server got, by its port. */
const echoes = new Map();
let nextPage = 1;
let certificate;

/** A self-signed certificate for skald.test from openssl; Chrome takes it with ignoreHTTPSErrors. */
function tlsOptions() {
  if (!certificate) {
    const dir = mkdtempSync(join(tmpdir(), "skald-browser-"));
    try {
      execFileSync(
        "openssl",
        [
          "req",
          "-x509",
          "-newkey",
          "rsa:2048",
          "-nodes",
          "-days",
          "2",
          "-subj",
          "/CN=skald.test",
          "-keyout",
          join(dir, "key.pem"),
          "-out",
          join(dir, "cert.pem"),
        ],
        { stdio: "ignore" },
      );
      certificate = {
        key: readFileSync(join(dir, "key.pem")),
        cert: readFileSync(join(dir, "cert.pem")),
      };
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  }
  return certificate;
}

/** Listens on a free local port; resolves to the port. */
function listen(server) {
  return new Promise((resolve, reject) => {
    server.on("error", reject);
    server.listen(0, "127.0.0.1", () => resolve(server.address().port));
  });
}

function page(id) {
  const found = pages.get(id);
  if (!found) throw new Error(`no page ${id}`);
  return found;
}

/**
 * Runs `navigate`, which navigates `tab` and resolves to the last response, if any; gives the final
 * URL, the status of that response, the URLs the main frame requested on the way, redirects
 * included, and the navigation error, if any.
 */
async function navigation(tab, navigate) {
  const navigations = [];
  const record = (request) => {
    if (request.isNavigationRequest() && request.frame() === tab.mainFrame())
      navigations.push(request.url());
  };
  const failures = [];
  const fail = (request) => {
    if (request.isNavigationRequest() && request.frame() === tab.mainFrame())
      failures.push(`${request.url()}: ${request.failure()?.errorText}`);
  };
  tab.on("request", record);
  tab.on("requestfailed", fail);
  let status = null;
  let error = null;
  try {
    const response = await navigate();
    status = response?.status() ?? null;
  } catch (caught) {
    error = [String(caught.message ?? caught), ...failures].join("\n");
  } finally {
    tab.off("request", record);
    tab.off("requestfailed", fail);
  }
  return { url: tab.url(), status, navigations, error };
}

/** How long `followRefreshes` waits in all, as long as playwright-core waits for a navigation. */
const REFRESH_TIMEOUT = 30_000;

/**
 * Waits until `tab` stops navigating by itself: until its document has loaded and holds no
 * `<meta http-equiv="refresh">`, such as the sign-in callback's page. A document that holds one is
 * followed to the document its refresh commits, an error page included.
 */
async function followRefreshes(tab) {
  const deadline = Date.now() + REFRESH_TIMEOUT;
  // At least 1 ms: a timeout of 0 is none to playwright-core.
  const left = () => Math.max(deadline - Date.now(), 1);
  const stuck = () =>
    new Error(
      `${tab.url()} did not stop refreshing within ${REFRESH_TIMEOUT} ms`,
    );
  let documents = 0;
  let committed = () => {};
  const count = (frame) => {
    if (frame !== tab.mainFrame()) return;
    documents += 1;
    committed();
  };
  tab.on("framenavigated", count);
  try {
    for (;;) {
      const seen = documents;
      // Its timeout is the rest of the deadline.
      await tab.waitForLoadState("load", { timeout: left() }).catch((error) => {
        throw error.name === "TimeoutError" ? stuck() : error;
      });
      // A navigation that replaces the document fails the evaluation: its document comes next.
      const refreshes = await tab
        .evaluate(
          () => document.querySelector('meta[http-equiv="refresh" i]') !== null,
        )
        .catch(() => true);
      if (documents !== seen) continue;
      if (!refreshes) return;
      await new Promise((resolve, reject) => {
        const timer = setTimeout(() => reject(stuck()), left());
        committed = () => {
          clearTimeout(timer);
          committed = () => {};
          resolve();
        };
        if (documents !== seen) committed();
      });
    }
  } finally {
    tab.off("framenavigated", count);
  }
}

/**
 * Clicks what `selector` finds in `tab` and waits for the navigation it starts to load; with
 * `settle`, also for the documents its refreshes open, as `followRefreshes` does.
 */
function click(tab, selector, settle = false) {
  return navigation(tab, async () => {
    const [response] = await Promise.all([
      tab.waitForNavigation({ waitUntil: "load" }),
      tab.locator(selector).click(),
    ]);
    if (settle) await followRefreshes(tab);
    return response;
  });
}

const ops = {
  /** Opens a page in a new browser context: a browser with no cookies. */
  async newPage() {
    const context = await browser.newContext({ ignoreHTTPSErrors: true });
    const id = nextPage++;
    pages.set(id, { context, page: await context.newPage() });
    return id;
  },

  /**
   * Navigates the page to `url`, as a person typing it, and waits for it to load; with `settle`,
   * also for the documents its refreshes open, as `followRefreshes` does. Gives the final URL, the
   * status of the last response of the navigation to `url`, before any refresh, the URLs the main
   * frame requested on the way, redirects and refreshes included, and the navigation error, if any.
   */
  async goto({ page: id, url, settle = false }) {
    // Each navigation gets a new tab of the browser: Chrome commits its error page for an empty
    // error response, such as a 404 without a body, after goto gave up on it, and that commit
    // would interrupt the next navigation in the same tab.
    const found = page(id);
    const tab = await found.context.newPage();
    found.page.close().catch(() => {});
    found.page = tab;
    return navigation(tab, async () => {
      const response = await tab.goto(url, { waitUntil: "load" });
      if (settle) await followRefreshes(tab);
      return response;
    });
  },

  /**
   * Clicks the element `selector` finds, a playwright-core selector such as
   * `role=button[name=/approve/i]`, as a person does, and waits for the navigation it starts to
   * load; with `settle`, as `goto` does. Gives what `goto` gives.
   */
  async click({ page: id, selector, settle = false }) {
    return click(page(id).page, selector, settle);
  },

  /** Clicks each `{page, selector}` of `clicks` at once, as `click` does; gives their visits. */
  async clickAll({ clicks }) {
    const tabs = clicks.map(({ page: id }) => page(id).page);
    // Each element first, so that the clicks themselves go out together.
    await Promise.all(
      clicks.map(({ selector }, at) =>
        tabs[at].locator(selector).waitFor({ state: "visible" }),
      ),
    );
    return Promise.all(
      clicks.map(({ selector }, at) => click(tabs[at], selector)),
    );
  },

  /**
   * Evaluates the script `expression`, which starts a navigation of the page, and waits until the
   * network has been idle for 0.5 s. Gives what `goto` gives.
   */
  async navigateBy({ page: id, expression }) {
    const tab = page(id).page;
    return navigation(tab, async () => {
      const [response] = await Promise.all([
        tab.waitForNavigation({ waitUntil: "load" }),
        tab.evaluate(expression),
      ]);
      await tab.waitForLoadState("networkidle");
      return response;
    });
  },

  /** Types `value` into the element `selector` finds, replacing what it holds. */
  async fill({ page: id, selector, value }) {
    await page(id).page.locator(selector).fill(value);
    return null;
  },

  /** How many elements `selector` finds. */
  async count({ page: id, selector }) {
    return page(id).page.locator(selector).count();
  },

  /** The value of each form field `selector` finds. */
  async values({ page: id, selector }) {
    const fields = page(id).page.locator(selector);
    return fields.evaluateAll((all) => all.map((field) => field.value));
  },

  /** The page's text as a person sees it. */
  async text({ page: id }) {
    return page(id).page.locator("body").innerText();
  },

  /**
   * What the button `selector` finds submits: its form's action and method, its own if it
   * overrides them, and the form's fields with the button's own, as `[name, value]` pairs.
   */
  async submission({ page: id, selector }) {
    return page(id)
      .page.locator(selector)
      .evaluate((button) => {
        const form = button.form;
        return {
          action: button.hasAttribute("formaction")
            ? button.formAction
            : form.action,
          method: button.hasAttribute("formmethod")
            ? button.formMethod
            : form.method,
          fields: [...new FormData(form, button).entries()].map(
            ([name, value]) => [name, String(value)],
          ),
        };
      });
  },

  /** Evaluates the script `expression` in the page; a promise is awaited. */
  async evaluate({ page: id, expression }) {
    return (await page(id).page.evaluate(expression)) ?? null;
  },

  async cookies({ page: id }) {
    return page(id).context.cookies();
  },

  async addCookies({ page: id, cookies }) {
    await page(id).context.addCookies(cookies);
    return null;
  },

  /**
   * Starts a server, over HTTPS with `tls`, that records each request's method, host, URL and
   * Cookie header and answers with an empty page.
   */
  async echo({ tls = false }) {
    const hits = [];
    const handle = (request, response) => {
      request.resume();
      hits.push({
        method: request.method,
        host: request.headers.host ?? "",
        url: request.url,
        cookie: request.headers.cookie ?? null,
      });
      response.writeHead(200, { "content-type": "text/html" });
      response.end("<!doctype html><title>echo</title>");
    };
    const server = tls
      ? https.createServer(tlsOptions(), handle)
      : http.createServer(handle);
    const port = await listen(server);
    echoes.set(port, hits);
    return port;
  },

  async hits({ port }) {
    return echoes.get(port) ?? [];
  },

  /** Starts an HTTPS server that forwards every request to the plain HTTP `target` as it is. */
  async tlsProxy({ target }) {
    const server = https.createServer(tlsOptions(), (request, response) => {
      const upstream = http.request(
        new URL(request.url, target),
        { method: request.method, headers: request.headers },
        (answer) => {
          response.writeHead(answer.statusCode, answer.rawHeaders);
          answer.pipe(response);
        },
      );
      upstream.on("error", (error) => {
        response.writeHead(502, { "content-type": "text/plain" });
        response.end(String(error));
      });
      request.pipe(upstream);
    });
    return listen(server);
  },
};

const lines = createInterface({ input: process.stdin });
lines.on("line", async (line) => {
  const { id, op, ...args } = JSON.parse(line);
  try {
    if (!ops[op]) throw new Error(`unknown op ${op}`);
    send({ id, ok: await ops[op](args) });
  } catch (error) {
    send({ id, error: String(error?.stack ?? error) });
  }
});
lines.on("close", async () => {
  await browser.close();
  process.exit(0);
});
send({ ready: true });
