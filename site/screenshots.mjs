// Saves full-page screenshots of the main page in both themes at the check widths, taken in the
// pinned Chrome of the kit's screenshots: <dir>/main-<theme>-<width>.png. Reviews compare them with
// the approved design. The same of the threat model page, for the docs pages' look:
// <dir>/threat-model-<theme>-<width>.png. Usage, after npm ci in site/ and web/:
//   node site/screenshots.mjs [<dir>]  <dir> is site/screenshots/ by default; site/references
//                                      updates the references
import fs from 'node:fs';
import path from 'node:path';
import { startSite } from './browser.mjs';
import { take, views } from './references.mjs';

const dir = path.resolve(process.argv[2] ?? path.join(path.dirname(new URL(import.meta.url).pathname), 'screenshots'));
fs.mkdirSync(dir, { recursive: true });

const site = await startSite({ pinned: true });
try {
  for (const view of views) {
    const file = path.join(dir, view.file);
    fs.writeFileSync(file, await take(site, view));
    console.log(file);
  }
} finally {
  await site.close();
}
