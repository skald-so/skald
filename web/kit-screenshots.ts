// Saves full-page screenshots of the kit page in both themes at both check widths:
// <dir>/kit-<theme>-<width>.png. Usage: node kit-screenshots.ts [<dir>], default kit-screenshots/.
import { mkdirSync } from "node:fs";
import { join, resolve } from "node:path";
import { startKit, themes, widths } from "./kit-browser.ts";

const dir = resolve(process.argv[2] ?? "kit-screenshots");
mkdirSync(dir, { recursive: true });

const kit = await startKit();
try {
  for (const theme of themes) {
    for (const width of widths) {
      const { page } = await kit.open({ width, system: theme, theme });
      const path = join(dir, `kit-${theme}-${width}.png`);
      await page.screenshot({ path, fullPage: true });
      await page.close();
      console.log(path);
    }
  }
} finally {
  await kit.close();
}
