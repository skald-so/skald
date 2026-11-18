// servedFile <dir> <pathname>: the file of the built site in <dir> that a path is served from, as
// GitHub Pages serves it: a page without .html, a directory's index.html. Relative to <dir>, or null.
import fs from 'node:fs';
import path from 'node:path';

export function servedFile(dir, pathname) {
  const name = decodeURIComponent(pathname).replace(/^\/+/, '');
  for (const candidate of [name === '' || name.endsWith('/') ? `${name}index.html` : name, `${name}.html`]) {
    const full = path.join(dir, candidate);
    if (full.startsWith(`${dir}${path.sep}`) && fs.existsSync(full) && fs.statSync(full).isFile()) return candidate;
  }
  return null;
}
