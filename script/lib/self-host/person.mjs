// The person of script/self-host-test: Chrome Headless Shell in the browser's container on the clean
// machine, driven by the playwright-core of web/'s packages, which the test puts next to this file in
// node_modules. The stand-ins and the stack serve certificates the browser does not check.
// Usage:
//   node person.mjs approve <provider> <provider's sign-in page> <verification page> <code>
//     signs in to the provider's stand-in, GitHub or Google, at its sign-in page; opens the
//     verification page, signs in to the server there with that provider when the page asks, enters
//     the code and approves it
//   node person.mjs offers <verification page>
//     prints the providers, GitHub and Google, that the page without a session offers sign-in with,
//     one per line
//   node person.mjs refused <provider> <provider's sign-in page> <verification page>
//     signs in to the provider's stand-in at its sign-in page; opens the verification page, which
//     has the code filled in, and signs in to the server there with that provider; prints the
//     status of the callback's answer on the first line, then the HTML of the page it ended on
// Exits 1, saying why and with the page's text, when a step finds nothing to do.
import { chromium } from "playwright-core";

const CODE_FIELD = "role=textbox";
const APPROVE = "role=button[name=/approve/i]";
const FIRST_BUTTON = "role=button >> nth=0";
const providerLink = (provider) =>
  `a:has-text('${provider}'), button:has-text('${provider}')`;
const STEP_TIMEOUT = 60_000;

const [op, ...args] = process.argv.slice(2);
const browser = await chromium.launch({
  executablePath: "/usr/bin/chromium-headless-shell",
  args: ["--no-sandbox"],
});
const context = await browser.newContext({ ignoreHTTPSErrors: true });
context.setDefaultTimeout(STEP_TIMEOUT);
const page = await context.newPage();

/** Ends the run with `reason` and the page's text. */
async function refuse(reason) {
  const text = await page.innerText("body").catch(() => "");
  console.error(`person: ${reason}; at ${page.url()}:\n${text}`);
  await browser.close();
  process.exit(1);
}

/** Clicks what `selector` finds and waits for the page the form it sends gives. */
async function submit(selector) {
  await Promise.all([
    page.waitForNavigation({ waitUntil: "load" }),
    page.locator(selector).click(),
  ]);
}

/** Opens `url` and waits until its requests are done. */
async function open(url) {
  const response = await page.goto(url, { waitUntil: "networkidle" });
  return response?.status();
}

if (op === "approve" && args.length === 4) {
  const [provider, signIn, verification, code] = args;
  const status = await open(signIn);
  if (status !== 200)
    await refuse(`the ${provider} stand-in's sign-in answers ${status}`);
  await open(verification);
  if ((await page.locator(CODE_FIELD).count()) === 0) {
    const link = page.locator(providerLink(provider));
    if ((await link.count()) === 0) await refuse(`no sign-in with ${provider}`);
    const callback = `/auth/${provider.toLowerCase()}/callback`;
    await Promise.all([
      page.waitForResponse(
        (response) => new URL(response.url()).pathname === callback,
      ),
      link.first().click(),
    ]);
    await page.waitForLoadState("networkidle");
    await open(verification);
  }
  if ((await page.locator(CODE_FIELD).count()) !== 1)
    await refuse("no field for the code");
  await page.locator(CODE_FIELD).fill(code);
  if ((await page.locator(APPROVE).count()) === 0) {
    await submit(FIRST_BUTTON);
  }
  if ((await page.locator(APPROVE).count()) === 0)
    await refuse("no approve button");
  await submit(APPROVE);
} else if (op === "refused" && args.length === 3) {
  const [provider, signIn, verification] = args;
  const status = await open(signIn);
  if (status !== 200)
    await refuse(`the ${provider} stand-in's sign-in answers ${status}`);
  await open(verification);
  const link = page.locator(providerLink(provider));
  if ((await link.count()) === 0) await refuse(`no sign-in with ${provider}`);
  const callback = `/auth/${provider.toLowerCase()}/callback`;
  const [answer] = await Promise.all([
    page.waitForResponse(
      (response) => new URL(response.url()).pathname === callback,
    ),
    link.first().click(),
  ]);
  await page.waitForLoadState("networkidle");
  console.log(answer.status());
  console.log(await page.content());
} else if (op === "offers" && args.length === 1) {
  await open(args[0]);
  for (const provider of ["GitHub", "Google"]) {
    if ((await page.locator(providerLink(provider)).count()) > 0)
      console.log(provider);
  }
} else {
  console.error(
    "usage: node person.mjs approve <provider> <sign-in page> <verification page> <code>",
  );
  console.error("       node person.mjs offers <verification page>");
  console.error(
    "       node person.mjs refused <provider> <sign-in page> <verification page>",
  );
  process.exit(2);
}
await browser.close();
