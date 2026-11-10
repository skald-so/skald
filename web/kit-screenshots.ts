// Saves full-page screenshots of the kit page in both themes at both check widths, taken in the
// pinned Chrome: <dir>/kit-<theme>-<width>.png. Usage: node kit-screenshots.ts [<dir>], default
// kit-screenshots/; `npm run kit:references` saves the references to kit-references/.
import { mkdirSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { views } from "./kit-references.ts";
import { screenshot, startKit } from "./kit-browser.ts";

const dir = resolve(process.argv[2] ?? "kit-screenshots");
mkdirSync(dir, { recursive: true });

const kit = await startKit({ pinned: true });
try {
  for (const { theme, width, file } of views) {
    const { page } = await kit.open({ width, system: theme, theme });
    const path = join(dir, file);
    writeFileSync(path, await screenshot(page));
    await page.close();
    console.log(path);
  }
} finally {
  await kit.close();
}
