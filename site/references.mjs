// The website's reference screenshots, site/references/: their views and how to take them. The
// comparison is the kit's, web/kit-references.ts.
import path from 'node:path';
import { screenshot, themes, widths } from '../web/kit-browser.ts';

const here = path.dirname(new URL(import.meta.url).pathname);

// Every view: the main page, then the threat model page for the docs pages' look, each in both
// themes at the check widths, {at, theme, width, file}, <at> the page's path, the file e.g.
// main-light-1440.png or threat-model-dark-360.png.
const pages = [
  ['main', '/'],
  ['threat-model', '/threat-model'],
];
export const views = pages.flatMap(([name, at]) =>
  themes.flatMap((theme) => widths.map((width) => ({ at, theme, width, file: `${name}-${theme}-${width}.png` }))),
);

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
