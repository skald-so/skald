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

const ops = {
  /** Opens a page in a new browser context: a browser with no cookies. */
  async newPage() {
    const context = await browser.newContext({ ignoreHTTPSErrors: true });
    const id = nextPage++;
    pages.set(id, { context, page: await context.newPage() });
    return id;
  },

  /**
   * Navigates the page to `url`, as a person typing it, and waits for `waitUntil`, default `load`.
   * Gives the final URL, the status of the last response, the URLs the main frame requested on the
   * way, redirects included, and the navigation error, if any.
   */
  async goto({ page: id, url, waitUntil = "load" }) {
    // Each navigation gets a new tab of the browser: Chrome commits its error page for an empty
    // error response, such as a 404 without a body, after goto gave up on it, and that commit
    // would interrupt the next navigation in the same tab.
    const found = page(id);
    const tab = await found.context.newPage();
    found.page.close().catch(() => {});
    found.page = tab;
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
      const response = await tab.goto(url, { waitUntil });
      status = response?.status() ?? null;
    } catch (caught) {
      error = [String(caught.message ?? caught), ...failures].join("\n");
    } finally {
      tab.off("request", record);
      tab.off("requestfailed", fail);
    }
    return { url: tab.url(), status, navigations, error };
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
