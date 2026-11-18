// Builds the website and docs and fails on a broken internal link: the main page at / from
// site/main/ with Vite, and docs/ with VitePress (site/config.mjs). A page is each Markdown file of
// docs/ that git does not ignore, at its path, a README.md as its directory's index; without a
// docs/start.md, the docs start page, /start, lists the pages. A page at / fails the build, as the
// main page is there. Links between pages, to their headings and to files outside docs/ must resolve.
// The fonts' licenses ship in fonts/.
// Usage, after npm ci in site/: node site/build.mjs <out dir> [<docs dir>]
//   <docs dir>  docs/ by default; its parent is the repository root that links leave docs/ for.
//   SKALD_SITE_REF  the git ref that links to files outside docs/ name on GitHub; main by default.
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { build as viteBuild } from 'vite';
import { build } from 'vitepress';
import { servedFile } from './served.mjs';

const here = path.dirname(new URL(import.meta.url).pathname);
const [outArg, docsArg, ...extra] = process.argv.slice(2);
if (!outArg || extra.length > 0) {
  console.error('usage: node site/build.mjs <out dir> [<docs dir>]');
  process.exit(2);
}
const docs = fs.realpathSync(docsArg ?? path.join(here, '..', 'docs'));
const repo = path.dirname(docs);
const out = path.resolve(outArg);
const ref = process.env.SKALD_SITE_REF || 'main';
// The packages of the main page's fonts, in @fontsource-variable/.
const fonts = ['inter', 'space-grotesk', 'jetbrains-mono'];
const scheme = /^[a-z][a-z0-9+.-]*:|^\/\//i;
const decode = (text) =>
  text.replace(/&(amp|lt|gt|quot|#39|#x27);/g, (m, name) => ({ amp: '&', lt: '<', gt: '>', quot: '"', '#39': "'", '#x27': "'" })[name]);

const work = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'skald-site.')));
let status = 1;
try {
  status = await main();
} finally {
  fs.rmSync(work, { recursive: true, force: true });
}
process.exit(status);

async function main() {
  // The sources: docs/ without what git ignores, each README.md as index.md unless its directory
  // has one.
  const src = path.join(work, 'src');
  // The pages' imports resolve from the packages of site/.
  fs.symlinkSync(path.join(here, 'node_modules'), path.join(work, 'node_modules'));
  const files = execFileSync('git', ['ls-files', '--cached', '--others', '--exclude-standard', '-z', '--', '.'], { cwd: docs, encoding: 'utf8' })
    .split('\0')
    .filter((f) => f !== '' && fs.existsSync(path.join(docs, f)));
  const pages = [];
  for (const file of files.sort()) {
    const dir = path.posix.dirname(file);
    const index = dir === '.' ? 'index.md' : `${dir}/index.md`;
    const staged = path.posix.basename(file) === 'README.md' && !files.includes(index) ? index : file;
    fs.mkdirSync(path.join(src, path.posix.dirname(staged)), { recursive: true });
    fs.copyFileSync(path.join(docs, file), path.join(src, staged));
    if (file.endsWith('.md')) pages.push({ path: staged, source: file, title: title(path.join(docs, file)) });
  }
  const root = pages.find((p) => p.path === 'index.md');
  if (root) {
    console.error(`site: ${root.source} is a page at /, where the main page is`);
    return 1;
  }
  if (!pages.some((p) => p.path === 'start.md')) {
    fs.writeFileSync(path.join(src, 'start.md'), startPage(pages));
  }

  const errors = path.join(work, 'errors');
  fs.writeFileSync(errors, '');
  const options = { docs, repo, ref, pages, out, errors };
  fs.mkdirSync(path.join(src, '.vitepress'));
  fs.writeFileSync(
    path.join(src, '.vitepress', 'config.mjs'),
    `import { siteConfig } from ${JSON.stringify(path.join(here, 'config.mjs'))};\nexport default siteConfig(${JSON.stringify(options)});\n`,
  );
  fs.rmSync(out, { recursive: true, force: true });
  try {
    await build(src);
    await buildMainPage(out, ref);
  } catch (error) {
    console.error(`site: the build failed: ${error.message}`);
    return 1;
  }
  const broken = [...new Set(fs.readFileSync(errors, 'utf8').split('\n').filter(Boolean)), ...checkLinks(out, pages)];
  for (const line of broken) console.error(`site: broken link: ${line}`);
  if (broken.length > 0) {
    console.error(`site: ${broken.length} broken link(s)`);
    return 1;
  }
  console.log(`site: built ${pages.length} page(s) into ${out}`);
  return 0;
}

// title <file>: the text of the page's first heading of level 1, or its file name.
function title(file) {
  const heading = fs.readFileSync(file, 'utf8').match(/^# +(.+?)\s*$/m);
  return heading ? heading[1].replace(/`/g, '') : path.basename(file, '.md');
}

// buildMainPage <out> <ref>: builds the main page of site/main/ into <out>, its link to a
// repository file at <ref>, and copies each font's license to fonts/<package>/LICENSE.
async function buildMainPage(out, ref) {
  await viteBuild({
    root: path.join(here, 'main'),
    configFile: false,
    logLevel: 'warn',
    plugins: [{ name: 'skald-site-ref', transformIndexHtml: { order: 'pre', handler: (html) => html.replaceAll('%SKALD_SITE_REF%', ref) } }],
    // Every asset a file of its own, never a data: URI, as the web app's.
    build: { outDir: out, emptyOutDir: false, assetsDir: 'assets/main', assetsInlineLimit: 0, modulePreload: { polyfill: false } },
  });
  for (const font of fonts) {
    fs.mkdirSync(path.join(out, 'fonts', font), { recursive: true });
    fs.copyFileSync(path.join(here, 'node_modules', '@fontsource-variable', font, 'LICENSE'), path.join(out, 'fonts', font, 'LICENSE'));
  }
}

// startPage <pages>: a page that links to every page.
function startPage(pages) {
  const lines = ['# Docs', ''];
  for (const page of pages) lines.push(`- [${page.title}](${page.path})`);
  return `${lines.join('\n')}\n`;
}

// checkLinks <dir> <pages>: the links of the built pages in <dir> that lead to no file, or to no
// element of their page with the fragment's ID, each as `<file>: <link>`, <file> the page's file in
// docs/ or the built file.
function checkLinks(dir, pages) {
  const sources = new Map(pages.map((p) => [p.path.replace(/\.md$/, '.html'), p.source]));
  const html = fs.readdirSync(dir, { recursive: true }).filter((f) => f.endsWith('.html'));
  const ids = new Map();
  const idsOf = (file) => {
    if (!ids.has(file)) {
      const text = fs.readFileSync(path.join(dir, file), 'utf8');
      ids.set(file, new Set([...text.matchAll(/\sid="([^"]*)"/g)].map((m) => decode(m[1]))));
    }
    return ids.get(file);
  };
  const broken = [];
  for (const file of html.sort()) {
    const url = `https://site/${file.replace(/(^|\/)index\.html$/, '$1').replace(/\.html$/, '')}`;
    const text = fs.readFileSync(path.join(dir, file), 'utf8');
    for (const [, raw] of text.matchAll(/\s(?:href|src)="([^"]*)"/g)) {
      const link = decode(raw);
      if (scheme.test(link)) continue;
      const target = new URL(link, url);
      const targetFile = servedFile(dir, target.pathname);
      const shown = `${sources.get(file) ?? file}: ${link}`;
      if (targetFile === null) broken.push(shown);
      else if (target.hash.length > 1 && targetFile.endsWith('.html') && !idsOf(targetFile).has(decodeURIComponent(target.hash.slice(1)))) {
        broken.push(shown);
      }
    }
  }
  return [...new Set(broken)];
}
