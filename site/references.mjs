// The main page's reference screenshots, site/references/: its views and how to take them. The
// comparison is the kit's, web/kit-references.ts.
import path from 'node:path';
import { screenshot, themes, widths } from '../web/kit-browser.ts';

const here = path.dirname(new URL(import.meta.url).pathname);

// Every view of the main page: both themes at the check widths, {theme, width, file}, the file e.g.
// main-light-1440.png.
export const views = themes.flatMap((theme) => widths.map((width) => ({ theme, width, file: `main-${theme}-${width}.png` })));

// The committed references, made by `node site/screenshots.mjs site/references`.
export const referenceDir = path.join(here, 'references');
// Where a failed comparison saves the screenshot and its diff image; ignored by git.
export const diffDir = path.join(here, 'references-diff');

// take <site> <view> [<change>]: a full-page screenshot of the main page in `view`, as a PNG, from a
// site of startSite (site/browser.mjs), after `change(page)` alters the page.
export async function take(site, { theme, width }, change) {
  const { page } = await site.open({ width, system: theme });
  try {
    await change?.(page);
    return await screenshot(page);
  } finally {
    await page.close();
  }
}
