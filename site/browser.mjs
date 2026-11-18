// The built website in headless Chrome: for its browser tests and screenshots. Takes the Chrome of the
// kit tests, web/kit-browser.ts, so it needs npm ci in web/ as well as in site/.
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import http from 'node:http';
import os from 'node:os';
import path from 'node:path';
import { connectPinnedChrome, launchChrome } from '../web/kit-browser.ts';

const here = path.dirname(new URL(import.meta.url).pathname);
const types = {
  '.html': 'text/html; charset=utf-8',
  '.css': 'text/css',
  '.js': 'text/javascript',
  '.json': 'application/json',
  '.woff2': 'font/woff2',
  '.svg': 'image/svg+xml',
};

// startSite({pinned}): builds the website into a temporary directory, serves it on this machine as
// GitHub Pages does, a page without .html, and starts Chrome: the pinned Chrome of the kit's
// screenshots with `pinned`, otherwise Chrome Headless Shell. Returns {origin, browser, open, close}:
// open({width, system}) opens the main page `width` pixels wide with the system set to the `system`
// theme, rendered and with fonts loaded, and gives {page, requests}, `requests` the URLs of every
// request the page made.
export async function startSite({ pinned = false } = {}) {
  const dir = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'skald-site-browser.')));
  const remove = () => fs.rmSync(dir, { recursive: true, force: true });
  const out = path.join(dir, 'site');
  const built = spawnSync('node', [path.join(here, 'build.mjs'), out], { encoding: 'utf8' });
  if (built.status !== 0) {
    remove();
    throw new Error(`the site does not build:\n${built.stdout}${built.stderr}`);
  }
  const server = http.createServer((request, response) => {
    const file = fileOf(out, new URL(request.url, 'http://site').pathname);
    response.writeHead(file ? 200 : 404, { 'content-type': types[path.extname(file ?? '.html')] ?? 'application/octet-stream' });
    response.end(fs.readFileSync(file ?? path.join(out, '404.html')));
  });
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  const close = async () => {
    await new Promise((resolve) => server.close(resolve));
    remove();
  };
  let browser;
  try {
    browser = await (pinned ? connectPinnedChrome() : launchChrome());
  } catch (error) {
    await close();
    throw error;
  }
  const origin = `http://127.0.0.1:${server.address().port}`;
  return {
    origin,
    browser,
    async open({ width, system }) {
      const page = await browser.newPage({
        viewport: { width, height: 900 },
        colorScheme: system,
        permissions: ['clipboard-read', 'clipboard-write'],
      });
      const requests = [];
      page.on('request', (request) => requests.push(request.url()));
      await page.goto(`${origin}/`);
      await page.locator('h1').waitFor();
      await page.evaluate(async () => {
        await document.fonts.ready;
      });
      return { page, requests };
    },
    async close() {
      await browser.close();
      await close();
    },
  };
}

// fileOf <dir> <pathname>: the file of <dir> a path is served from, or null.
function fileOf(dir, pathname) {
  const name = decodeURIComponent(pathname).replace(/^\/+/, '');
  for (const candidate of [name === '' || name.endsWith('/') ? `${name}index.html` : name, `${name}.html`]) {
    const file = path.join(dir, candidate);
    if (file.startsWith(`${dir}${path.sep}`) && fs.existsSync(file) && fs.statSync(file).isFile()) return file;
  }
  return null;
}
