// Saves full-page screenshots of the main page in both themes at the check widths, taken in the
// pinned Chrome of the kit's screenshots: <dir>/main-<theme>-<width>.png. Reviews compare them with
// the approved design. The same of the threat model page, for the docs pages' look:
// <dir>/threat-model-<theme>-<width>.png. Usage, after npm ci in site/ and web/:
//   node site/screenshots.mjs [<dir>]  <dir> is site/screenshots/ by default
import fs from 'node:fs';
import path from 'node:path';
import { screenshot, themes, widths } from '../web/kit-browser.ts';
import { startSite } from './browser.mjs';

const dir = path.resolve(process.argv[2] ?? path.join(path.dirname(new URL(import.meta.url).pathname), 'screenshots'));
fs.mkdirSync(dir, { recursive: true });

const site = await startSite({ pinned: true });
try {
  for (const [name, at] of [['main', '/'], ['threat-model', '/threat-model']]) {
    for (const theme of themes) {
      for (const width of widths) {
        const { page } = await site.open({ width, system: theme, at });
        const file = path.join(dir, `${name}-${theme}-${width}.png`);
        fs.writeFileSync(file, await screenshot(page));
        await page.close();
        console.log(file);
      }
    }
  }
} finally {
  await site.close();
}
