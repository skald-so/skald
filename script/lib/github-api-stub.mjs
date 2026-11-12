// A stand-in for the GitHub API that release-plz opens and updates a Release PR and tags a release
// through, for script/release-plz-test. It keeps the PRs in memory and serves the calls release-plz
// makes for a Release PR: its PRs, their commits and labels, branch refs, and commits made with the
// GraphQL mutation createCommitOnBranch; and for a release: the PRs merged as a commit, tag objects
// and tag refs. Every request goes to <log> as a JSON line, with the status and the body of its
// reply; after each request <pulls> holds the PRs as JSON. <merges> has a line `<commit> <branch>`
// for each PR merged as <commit> from <branch>; it is read on each request.
// Usage: node github-api-stub.mjs <log> <pulls> <port file> <merges>; it listens on 127.0.0.1 and
// writes its port to <port file> once it accepts connections.
import fs from 'node:fs';
import http from 'node:http';

const [log, pullsFile, portFile, mergesFile] = process.argv.slice(2);
const pulls = [];
let commits = 0;
const oid = () => (++commits).toString(16).padStart(40, '0');
const bot = { login: 'skald-release[bot]', id: 1, type: 'Bot' };

function pull(number, body) {
  return {
    id: 1000 + number,
    number,
    state: 'open',
    draft: Boolean(body.draft),
    title: body.title,
    body: body.body ?? '',
    html_url: `https://github.com/skald-so/skald/pull/${number}`,
    url: `https://api.github.com/repos/skald-so/skald/pulls/${number}`,
    head: { ref: body.head, label: body.head, sha: oid() },
    base: { ref: body.base, label: body.base, sha: oid() },
    user: bot,
    labels: [],
    created_at: '2026-01-01T00:00:00Z',
    updated_at: '2026-01-01T00:00:00Z',
  };
}

function reply(req, body) {
  const path = req.url.split('?')[0];
  const json = body ? JSON.parse(body) : {};
  const repo = path.match(/^\/api\/v3\/repos\/[^/]+\/[^/]+(\/.*)$/)?.[1];
  let match;
  if (path === '/api/graphql') {
    return [200, { data: { createCommitOnBranch: { commit: { oid: oid() } } } }];
  }
  if (repo === undefined) return [404, { message: 'Not Found' }];
  if (repo === '/pulls' && req.method === 'GET') {
    return [200, pulls.filter((p) => p.state === 'open')];
  }
  if (repo === '/pulls' && req.method === 'POST') {
    const created = pull(pulls.length + 1, json);
    pulls.push(created);
    return [201, created];
  }
  if ((match = repo.match(/^\/pulls\/(\d+)$/))) {
    const found = pulls.find((p) => p.number === Number(match[1]));
    if (!found) return [404, { message: 'Not Found' }];
    if (req.method === 'PATCH') Object.assign(found, json);
    return [200, found];
  }
  if ((match = repo.match(/^\/pulls\/(\d+)\/commits$/))) {
    const commit = { message: 'chore: release', author: { name: 'skald-release', email: 'app@example.com', date: '2026-01-01T00:00:00Z' } };
    return [200, [{ sha: oid(), commit, author: bot, committer: bot }]];
  }
  if ((match = repo.match(/^\/commits\/([0-9a-f]{40})\/pulls$/))) {
    const merges = fs.existsSync(mergesFile) ? fs.readFileSync(mergesFile, 'utf8') : '';
    const merged = merges.split('\n').map((line) => line.split(' '));
    return [200, merged.filter(([commit]) => commit === match[1]).map(([, head], i) => ({
      ...pull(9000 + i, { title: 'merged', head, base: 'dev' }),
      state: 'closed',
    }))];
  }
  if (repo === '/git/tags' && req.method === 'POST') {
    return [201, { sha: oid(), tag: json.tag, object: { sha: json.object, type: json.type } }];
  }
  if ((match = repo.match(/^\/issues\/(\d+)\/labels$/))) {
    return [200, (json.labels ?? []).map((name, i) => ({ id: i + 1, name }))];
  }
  if (repo.startsWith('/git/refs')) {
    if (req.method === 'DELETE') return [204, null];
    const ref = json.ref ?? `refs/${repo.slice('/git/refs/'.length)}`;
    return [req.method === 'POST' ? 201 : 200, { ref, object: { sha: json.sha ?? oid(), type: 'commit' } }];
  }
  return [404, { message: 'Not Found' }];
}

const server = http.createServer((req, res) => {
  let body = '';
  req.on('data', (chunk) => (body += chunk));
  req.on('end', () => {
    let status, json;
    try {
      [status, json] = reply(req, body);
    } catch (error) {
      [status, json] = [400, { message: String(error) }];
    }
    const line = { method: req.method, url: req.url, body, status, response: json };
    fs.appendFileSync(log, `${JSON.stringify(line)}\n`);
    fs.writeFileSync(pullsFile, JSON.stringify(pulls));
    res.writeHead(status, { 'content-type': 'application/json' });
    res.end(json === null ? '' : JSON.stringify(json));
  });
});
server.listen(0, '127.0.0.1', () => fs.writeFileSync(portFile, String(server.address().port)));
