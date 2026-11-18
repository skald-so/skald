// The VitePress config of the website and docs, for build.mjs, which stages docs/ and builds it.
// siteConfig(<options>):
//   docs    docs/, the directory the staged pages come from;
//   repo    the repository root: a link from a page to a file outside docs/ goes to that file on
//           GitHub, at <ref>;
//   ref     the git ref of those links;
//   pages   the staged pages, as {path, source, title}: <path> in the site's sources, <source> in
//           docs/;
//   out     where the site goes;
//   errors  a file that each link to a missing file outside docs/, or to a missing heading of one,
//           is appended to, one per line.
// Headings get GitHub's IDs, so the anchors that docs/ links to on GitHub hold on the site too.
import fs from 'node:fs';
import path from 'node:path';
import { slug } from 'github-slugger';

const github = 'https://github.com/skald-so/skald';
const scheme = /^[a-z][a-z0-9+.-]*:|^\/\//i;

export function siteConfig({ docs, repo, ref, pages, out, errors }) {
  return {
    title: 'Skald',
    description: 'A message broker and task queue for AI agents, with end-to-end encryption.',
    lang: 'en',
    cleanUrls: true,
    outDir: out,
    // build.mjs checks every link of the built pages instead.
    ignoreDeadLinks: true,
    markdown: {
      // Text like <tag> in docs/ is text, not HTML for Vue.
      html: false,
      anchor: { slugify: slug },
      // Code blocks in languages Shiki lacks, as plain text.
      languageAlias: { abnf: 'txt', caddy: 'txt', 'cbor-diag': 'txt', cddl: 'txt', hex: 'txt' },
      config: (md) =>
        md.core.ruler.after('inline', 'skald_repository_links', (state) => repositoryLinks(md, state, { docs, repo, ref, pages, errors })),
    },
    themeConfig: {
      // The main page is no VitePress page: its link loads it, outside VitePress's router, and the
      // 404 page leads to the docs start page instead.
      logoLink: { link: '/', target: '_self' },
      notFound: { link: '/start', linkText: 'Go to the docs', linkLabel: 'Go to the docs' },
      search: { provider: 'local' },
      sidebar: sidebar(pages),
      socialLinks: [{ icon: 'github', link: github }],
    },
  };
}

// sidebar <pages>: the pages of docs/ itself, then a group per directory, each in path order.
function sidebar(pages) {
  const groups = new Map([['', []]]);
  for (const page of pages) {
    const dir = path.posix.dirname(page.path);
    const top = dir === '.' ? '' : dir.split('/')[0];
    if (!groups.has(top)) groups.set(top, []);
    groups.get(top).push({ text: page.title, link: `/${page.path.replace(/(^|\/)index\.md$/, '$1').replace(/\.md$/, '')}` });
  }
  return [...groups].map(([dir, items]) => (dir === '' ? { items } : { text: dir, items }));
}

// repositoryLinks: a relative link of a page that leaves docs/ goes to its file on GitHub, the
// file and the heading it names checked; a link to a README.md of docs/ goes to the page it is
// staged as, index.md.
function repositoryLinks(md, state, { docs, repo, ref, pages, errors }) {
  const page = pages.find((p) => p.path === state.env.relativePath);
  if (!page) return;
  const source = path.join(docs, page.source);
  for (const token of state.tokens) {
    for (const child of token.children ?? []) {
      if (child.type !== 'link_open') continue;
      const href = child.attrGet('href');
      if (!href || href.startsWith('#') || scheme.test(href)) continue;
      const [target, fragment] = splitFragment(href);
      const file = path.resolve(path.dirname(source), decodeURI(target));
      if (isInside(docs, file)) {
        if (path.basename(file) === 'README.md' && !fs.existsSync(path.join(path.dirname(file), 'index.md'))) {
          child.attrSet('href', `${target.replace(/README\.md$/, 'index.md')}${fragment === undefined ? '' : `#${fragment}`}`);
        }
        continue;
      }
      const shown = `${page.source}: ${href}`;
      if (!isInside(repo, file)) {
        fs.appendFileSync(errors, `${shown}: outside the repository\n`);
        continue;
      }
      const relative = path.relative(repo, file).split(path.sep).join('/');
      if (!fs.existsSync(file)) fs.appendFileSync(errors, `${shown}: no such file in the repository\n`);
      else if (fragment !== undefined && file.endsWith('.md') && !headingIds(md, file).has(decodeURIComponent(fragment))) {
        fs.appendFileSync(errors, `${shown}: no such heading in ${relative}\n`);
      }
      const kind = fs.existsSync(file) && fs.statSync(file).isDirectory() ? 'tree' : 'blob';
      child.attrSet('href', `${github}/${kind}/${ref}/${relative}${fragment === undefined ? '' : `#${fragment}`}`);
    }
  }
}

const splitFragment = (href) => {
  const at = href.indexOf('#');
  return at < 0 ? [href, undefined] : [href.slice(0, at), href.slice(at + 1)];
};
const isInside = (dir, file) => {
  const relative = path.relative(dir, file);
  return relative === '' || (!relative.startsWith('..') && !path.isAbsolute(relative));
};

// headingIds <md> <file>: the IDs of the headings of a Markdown file, GitHub's as on the pages.
const ids = new Map();
function headingIds(md, file) {
  if (!ids.has(file)) {
    const tokens = md.parse(fs.readFileSync(file, 'utf8'), {});
    ids.set(file, new Set(tokens.filter((token) => token.type === 'heading_open').map((token) => token.attrGet('id'))));
  }
  return ids.get(file);
}
