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
//   node person.mjs page <web app's page>
//     opens the page of the web app without a session; prints its text once it shows the sign-in
//     page
//   node person.mjs cabinet <provider> <provider's sign-in page> <web app's page> [<section>...]
//     signs in to the provider's stand-in at its sign-in page; opens the web app's page and signs in
//     there with that provider, one click, landing in the cabinet; then opens each section by its
//     link in the side menu and prints a JSON line for it: {"section", "text", "bars"}, the text of
//     the page's main area and how many progress bars and meters it holds
//   node person.mjs revoke <provider> <provider's sign-in page> <web app's page> <device>
//     as cabinet; then, in Access, ends the CLI session of <device> with Revoke on its row
// Exits 1, saying why and with the page's text, when a step finds nothing to do, and, in the web
// app, when the page breaks a rule of its Content-Security-Policy.
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
// What the browser refused for the page's Content-Security-Policy.
const violations = [];
page.on("console", (message) => {
  if (
    message.type() === "error" &&
    /Content Security Policy/i.test(message.text())
  )
    violations.push(message.text().slice(0, 300));
});

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

/** Signs in to `provider`'s stand-in at its sign-in page `signIn`, then opens `verification`. */
async function openSignedIn(provider, signIn, verification) {
  const status = await open(signIn);
  if (status !== 200)
    await refuse(`the ${provider} stand-in's sign-in answers ${status}`);
  await open(verification);
}

/** Signs in to the server with `provider` on the page; gives the answer of its callback. */
async function signInWith(provider) {
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
  return answer;
}

const SIGN_IN_PAGE =
  "a:has-text('Continue with'), button:has-text('Continue with')";
const STEP_POLL = 250;

/** Ends the run when the web app broke a rule of its policy. */
async function checkPolicy() {
  if (violations.length > 0)
    await refuse(
      `the web app breaks its Content-Security-Policy:\n${violations.join("\n")}`,
    );
}

/** The page's main area: its `main` element, or its body without one. */
function mainArea() {
  return page.locator("main").first();
}

/** The main area's text once it stays the same for a second, or after the step's timeout. */
async function settledText() {
  const area =
    (await page.locator("main").count()) > 0
      ? mainArea()
      : page.locator("body");
  let last = null;
  let same = 0;
  for (let waited = 0; waited < STEP_TIMEOUT; waited += STEP_POLL) {
    const text = await area.innerText().catch(() => "");
    same = text === last && text.trim() !== "" ? same + 1 : 0;
    if (same >= 1000 / STEP_POLL) return text;
    last = text;
    await page.waitForTimeout(STEP_POLL);
  }
  return last ?? "";
}

/** Signs in to the web app at `app` with `provider`, its stand-in's sign-in page `signIn`. */
async function signInToApp(provider, signIn, app) {
  await openSignedIn(provider, signIn, app);
  await checkPolicy();
  if ((await page.locator(SIGN_IN_PAGE).count()) === 0)
    await refuse("the web app shows no sign-in page");
  const answer = await signInWith(provider);
  if (answer.status() >= 400)
    await refuse(`the sign-in's callback answers ${answer.status()}`);
  await page.waitForLoadState("networkidle");
  if (new URL(page.url()).origin !== new URL(app).origin)
    await refuse("the sign-in does not return to the web app");
  if ((await page.locator(SIGN_IN_PAGE).count()) > 0)
    await refuse("after signing in, the web app still shows the sign-in page");
  await checkPolicy();
}

/** Opens the cabinet's section by its link in the side menu. */
async function openSection(section) {
  const link = page.getByRole("link", { name: section, exact: true });
  if ((await link.count()) === 0)
    await refuse(`no link to ${section} in the side menu`);
  await link.first().click();
  await page.waitForLoadState("networkidle");
}

if (op === "page" && args.length === 1) {
  const status = await open(args[0]);
  if (status !== 200) await refuse(`the page answers ${status}`);
  await page
    .locator(SIGN_IN_PAGE)
    .first()
    .waitFor()
    .catch(() => {});
  if ((await page.locator(SIGN_IN_PAGE).count()) === 0)
    await refuse("the web app shows no sign-in page");
  await checkPolicy();
  console.log(await page.innerText("body"));
} else if (op === "cabinet" && args.length >= 3) {
  const [provider, signIn, app, ...sections] = args;
  await signInToApp(provider, signIn, app);
  for (const section of sections) {
    await openSection(section);
    const text = await settledText();
    const bars = await page
      .locator(
        "main [role=progressbar], main [role=meter], main progress, main meter",
      )
      .count();
    await checkPolicy();
    console.log(JSON.stringify({ section, text, bars }));
  }
} else if (op === "revoke" && args.length === 4) {
  const [provider, signIn, app, device] = args;
  await signInToApp(provider, signIn, app);
  await openSection("Access");
  await settledText();
  const row = page
    .getByRole("row")
    .filter({ hasText: "CLI" })
    .filter({ hasText: device });
  if ((await row.count()) !== 1)
    await refuse(`not one row of the CLI session of ${device} in Access`);
  const origin = new URL(app).origin;
  const ended = page.waitForResponse(
    (response) =>
      response.request().method() !== "GET" &&
      new URL(response.url()).origin === origin,
  );
  await row.getByRole("button", { name: /revoke/i }).click();
  const confirm = page
    .getByRole("dialog")
    .or(page.getByRole("alertdialog"))
    .getByRole("button", { name: /revoke|end|confirm/i });
  if ((await confirm.count()) > 0) await confirm.first().click();
  const answer = await ended;
  if (!answer.ok()) await refuse(`Revoke answers ${answer.status()}`);
  await checkPolicy();
} else if (op === "approve" && args.length === 4) {
  const [provider, signIn, verification, code] = args;
  await openSignedIn(provider, signIn, verification);
  if ((await page.locator(CODE_FIELD).count()) === 0) {
    await signInWith(provider);
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
  await openSignedIn(provider, signIn, verification);
  const answer = await signInWith(provider);
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
  console.error("       node person.mjs page <web app's page>");
  console.error(
    "       node person.mjs cabinet <provider> <sign-in page> <web app's page> [<section>...]",
  );
  console.error(
    "       node person.mjs revoke <provider> <sign-in page> <web app's page> <device>",
  );
  process.exit(2);
}
await browser.close();
