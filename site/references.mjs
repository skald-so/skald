// The main page's reference screenshots, site/references/: its views and how to take them, and the
// views saved for review only. The comparison is the kit's, web/kit-references.ts.
import path from 'node:path';
import { screenshot, themes, widths } from '../web/kit-browser.ts';

const here = path.dirname(new URL(import.meta.url).pathname);

// viewsOf <name> <at>: the page at the path <at> in both themes at the check widths, {at, theme,
// width, file}, the file e.g. <name>-light-1440.png.
const viewsOf = (name, at) => themes.flatMap((theme) => widths.map((width) => ({ at, theme, width, file: `${name}-${theme}-${width}.png` })));

// Every view of the main page.
export const views = viewsOf('main', '/');
// The threat model page, for reviews of the docs pages' look; no reference.
export const reviewViews = viewsOf('threat-model', '/threat-model');

// The committed references, made by `node site/screenshots.mjs site/references`.
export const referenceDir = path.join(here, 'references');
// Where a failed comparison saves the screenshot and its diff image; ignored by git.
export const diffDir = path.join(here, 'references-diff');

// take <site> <view> [<change>]: a full-page screenshot of the page of `view`, as a PNG, from a site
// of startSite (site/browser.mjs), after `change(page)` alters the page.
export async function take(site, { at, theme, width }, change) {
  const { page } = await site.open({ width, system: theme, at });
  try {
    await change?.(page);
    return await screenshot(page);
  } finally {
    await page.close();
  }
}
