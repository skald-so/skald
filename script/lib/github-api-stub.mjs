// A stand-in for the GitHub API that script/release-pr opens and updates a Release PR and
// script/release-tag tags a release through, for script/release-pr-test and script/release-tag-test.
// It keeps the PRs in memory and serves the calls of a Release PR: the open PRs, a PR's update,
// branch refs, and commits made with the GraphQL mutation createCommitOnBranch; and of a tag: the PRs
// associated with a commit, tag objects and refs. Every request goes to <log> as a JSON line of its
// method, url, contentType (its Content-Type header, or null) and body; after each request <pulls>
// holds the PRs it opened as JSON.
// <associated>, optional, is read on each request: a JSON object from a commit's SHA to the PRs
// `GET /repos/{owner}/{repo}/commits/{sha}/pulls` lists for it, or to an HTTP status it fails with,
// or to {"empty": <n>, "pulls": <PRs>}: an empty list for the first <n> calls, then the PRs;
// `GET /repos/{owner}/{repo}/pulls/{number}` serves those PRs too. After each request <refs>, also
// optional, holds the refs written through the API as a JSON array of {method, ref, commit}, a tag
// object resolved to its commit; creating a ref that exists fails with 422, as on GitHub. <listed>,
// optional, is read on each request: a JSON array of other PRs `GET /repos/{owner}/{repo}/pulls`
// lists while they are open.
// Usage: node github-api-stub.mjs <log> <pulls> <port file> [<associated> <refs> [<listed>]]; it
// listens on 127.0.0.1 and writes its port to <port file> once it accepts connections.
import fs from 'node:fs';
import http from 'node:http';

const [log, pullsFile, portFile, associatedFile, refsFile, listedFile] = process.argv.slice(2);
const pulls = [];
// The refs written, and the commit of each tag object created.
const refs = [];
// The SHA each ref points to.
const existing = new Map();
const tagObjects = new Map();
// The calls that listed the PRs of each commit.
const listCalls = new Map();
const read = (file, empty) =>
  file && fs.existsSync(file) ? JSON.parse(fs.readFileSync(file, 'utf8') || empty) : JSON.parse(empty);
const associated = () => read(associatedFile, '{}');
const listed = () => read(listedFile, '[]');
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
    head: { ref: body.head, label: body.head, sha: oid(), repo: { full_name: 'skald-so/skald' } },
    base: { ref: body.base, label: body.base, sha: oid(), repo: { full_name: 'skald-so/skald' } },
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
  // createCommitOnBranch: on a branch that exists and points to expectedHeadOid, as on GitHub.
  if (path === '/api/graphql') {
    const input = json.variables?.input ?? {};
    const ref = `refs/heads/${input.branch?.branchName}`;
    if (!existing.has(ref) || existing.get(ref) !== input.expectedHeadOid) {
      return [200, { errors: [{ message: `${ref} is not at ${input.expectedHeadOid}` }] }];
    }
    const commit = oid();
    existing.set(ref, commit);
    return [200, { data: { createCommitOnBranch: { commit: { oid: commit } } } }];
  }
  if (repo === undefined) return [404, { message: 'Not Found' }];
  if ((match = repo.match(/^\/commits\/([0-9a-f]+)\/pulls$/)) && req.method === 'GET') {
    const found = associated()[match[1]] ?? [];
    if (typeof found === 'number') return [found, { message: 'Server Error' }];
    if (Array.isArray(found)) return [200, found];
    const calls = (listCalls.get(match[1]) ?? 0) + 1;
    listCalls.set(match[1], calls);
    return [200, calls > found.empty ? found.pulls : []];
  }
  if (repo === '/git/tags' && req.method === 'POST') {
    const sha = oid();
    tagObjects.set(sha, json.object);
    return [201, { sha, tag: json.tag, message: json.message, object: { sha: json.object, type: 'commit' } }];
  }
  if (repo === '/pulls' && req.method === 'GET') {
    return [200, [...pulls, ...listed()].filter((p) => p.state === 'open')];
  }
  if (repo === '/pulls' && req.method === 'POST') {
    const created = pull(pulls.length + 1, json);
    pulls.push(created);
    return [201, created];
  }
  if ((match = repo.match(/^\/pulls\/(\d+)$/))) {
    const found =
      pulls.find((p) => p.number === Number(match[1])) ??
      Object.values(associated())
        .flatMap((listed) => (Array.isArray(listed) ? listed : (listed.pulls ?? [])))
        .find((p) => p.number === Number(match[1]));
    if (!found) return [404, { message: 'Not Found' }];
    if (req.method === 'PATCH') Object.assign(found, json);
    return [200, found];
  }
  if (repo.startsWith('/git/refs')) {
    const ref = json.ref ?? `refs/${repo.slice('/git/refs/'.length)}`;
    if (req.method === 'POST' && existing.has(ref)) {
      return [422, { message: 'Reference already exists' }];
    }
    if (req.method === 'DELETE') existing.delete(ref);
    else if (req.method !== 'GET') existing.set(ref, json.sha);
    if (req.method !== 'GET') {
      refs.push({ method: req.method, ref, commit: tagObjects.get(json.sha) ?? json.sha ?? null });
    }
    if (req.method === 'DELETE') return [204, null];
    return [req.method === 'POST' ? 201 : 200, { ref, object: { sha: json.sha ?? oid(), type: 'commit' } }];
  }
  return [404, { message: 'Not Found' }];
}

const server = http.createServer((req, res) => {
  let body = '';
  req.on('data', (chunk) => (body += chunk));
  req.on('end', () => {
    const contentType = req.headers['content-type'] ?? null;
    const entry = { method: req.method, url: req.url, contentType, body };
    fs.appendFileSync(log, `${JSON.stringify(entry)}\n`);
    let status, json;
    try {
      [status, json] = reply(req, body);
    } catch (error) {
      [status, json] = [400, { message: String(error) }];
    }
    fs.writeFileSync(pullsFile, JSON.stringify(pulls));
    if (refsFile) fs.writeFileSync(refsFile, JSON.stringify(refs));
    res.writeHead(status, { 'content-type': 'application/json' });
    res.end(json === null ? '' : JSON.stringify(json));
  });
});
server.listen(0, '127.0.0.1', () => fs.writeFileSync(portFile, String(server.address().port)));
