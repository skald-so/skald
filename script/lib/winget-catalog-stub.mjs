// Loopback GitHub REST stand-in for a WinGet catalog and its submission fork.
import { createHash } from 'node:crypto';
import { once } from 'node:events';
import http from 'node:http';

export async function catalogFixture() {
  const upstream = 'fixture-catalog/winget-pkgs';
  const fork = 'fixture-publisher/winget-pkgs';
  const oid = (value) => createHash('sha1').update(JSON.stringify(value)).digest('hex');
  const initialTree = oid({});
  const initialCommit = oid(initialTree);
  const trees = new Map([[initialTree, {}]]);
  const commits = new Map([[initialCommit, initialTree]]);
  const blobs = new Map();
  const refs = new Map([[upstream, new Map([['master', initialCommit]])], [fork, new Map([['master', initialCommit]])]]);
  const pulls = [];
  const missing = [404, { message: 'Not Found' }];
  const invalid = [422, { message: 'Validation Failed' }];
  let url;

  const repository = (full_name) => ({
    full_name,
    name: 'winget-pkgs',
    owner: { login: full_name.split('/')[0] },
    default_branch: 'master',
    fork: full_name === fork,
  });
  const refObject = (ref, sha) => ({ ref: `refs/heads/${ref}`, object: { type: 'commit', sha } });
  const gitObjects = (route, method, body, repo) => {
    const heads = refs.get(repo);
    const object = route.match(/^\/git\/(?:ref|refs)\/heads\/(.+)$/);
    if (object) {
      const ref = object[1];
      if (method === 'GET' && heads.has(ref)) return [200, refObject(ref, heads.get(ref))];
      if (method === 'PATCH' && repo === fork && heads.has(ref) && commits.has(body.sha)) {
        heads.set(ref, body.sha);
        return [200, refObject(ref, body.sha)];
      }
      return missing;
    }
    if (route === '/git/refs' && method === 'POST' && repo === fork) {
      const ref = body.ref?.match(/^refs\/heads\/(.+)$/)?.[1];
      if (!ref || heads.has(ref) || !commits.has(body.sha)) return invalid;
      heads.set(ref, body.sha);
      return [201, refObject(ref, body.sha)];
    }
    const git = route.match(/^\/git\/(blobs|trees|commits)(?:\/([^/]+))?$/);
    if (!git) return missing;
    const [, type, sha] = git;
    if (method === 'GET') {
      if (type === 'commits' && commits.has(sha)) return [200, { sha, tree: { sha: commits.get(sha) } }];
      if (type === 'trees' && trees.has(sha)) {
        return [200, { sha, tree: Object.entries(trees.get(sha)).map(([path, ref]) => ({ path, sha: ref, type: 'blob', mode: '100644' })) }];
      }
      if (type === 'blobs' && blobs.has(sha)) return [200, { sha, encoding: 'base64', content: Buffer.from(blobs.get(sha)).toString('base64') }];
      return missing;
    }
    if (method !== 'POST' || sha || repo !== fork) return missing;
    if (type === 'blobs') {
      const content = Buffer.from(body.content, body.encoding ?? 'utf8').toString('utf8');
      const blobSha = oid(content);
      blobs.set(blobSha, content);
      return [201, { sha: blobSha }];
    }
    if (type === 'trees') {
      if (body.base_tree && !trees.has(body.base_tree)) return invalid;
      const tree = { ...trees.get(body.base_tree) };
      for (const file of body.tree) {
        if (file.type !== 'blob' || file.mode !== '100644') return invalid;
        const blobSha = file.sha ?? oid(file.content);
        if (file.content !== undefined) blobs.set(blobSha, file.content);
        if (!blobs.has(blobSha)) return invalid;
        tree[file.path] = blobSha;
      }
      const treeSha = oid(tree);
      trees.set(treeSha, tree);
      return [201, { sha: treeSha }];
    }
    if (!trees.has(body.tree) || !body.parents?.every((parent) => commits.has(parent))) return invalid;
    const commitSha = oid(body);
    commits.set(commitSha, body.tree);
    return [201, { sha: commitSha }];
  };

  const reply = (request, body) => {
    if (!/^(?:Bearer|token) fixture-token$/.test(request.headers.authorization ?? '')) return [401, { message: 'Bad credentials' }];
    const { pathname, searchParams } = new URL(request.url, url);
    const route = decodeURIComponent(pathname).match(/^\/repos\/([^/]+\/[^/]+)(.*)$/);
    const [, repo, endpoint] = route ?? [];
    if (!refs.has(repo)) return missing;
    const { method } = request;
    if (!endpoint && method === 'GET') return [200, repository(repo)];
    if (repo === upstream && endpoint === '/pulls') {
      if (method === 'GET') {
        const state = searchParams.get('state') ?? 'open';
        const head = searchParams.get('head');
        const base = searchParams.get('base');
        return [200, pulls.filter((pull) =>
          (state === 'all' || pull.state === state) && (!head || pull.head.label === head) && (!base || pull.base.ref === base))];
      }
      if (method === 'POST') {
        const [owner, branch] = (body.head ?? '').split(':');
        const sha = refs.get(fork).get(branch);
        if (owner !== repository(fork).owner.login || !sha || sha === initialCommit || body.base !== 'master') return invalid;
        if (pulls.some((pull) => pull.state === 'open' && pull.head.label === body.head)) return invalid;
        const number = pulls.length + 10;
        const pull = {
          number, title: body.title, body: body.body, state: 'open', merged: false, merged_at: null,
          html_url: `${url}/${upstream}/pull/${number}`,
          head: { ref: branch, label: body.head, sha, repo: repository(fork) },
          base: { ref: 'master', sha: initialCommit, repo: repository(upstream) },
        };
        pulls.push(pull);
        return [201, pull];
      }
    }
    const pullNumber = endpoint.match(/^\/pulls\/(\d+)$/)?.[1];
    if (repo === upstream && method === 'GET' && pullNumber) {
      const pull = pulls.find(({ number }) => number === Number(pullNumber));
      return pull ? [200, pull] : missing;
    }
    return gitObjects(endpoint, method, body, repo);
  };

  const server = http.createServer(async (request, response) => {
    let status, body;
    try {
      const chunks = [];
      for await (const chunk of request) chunks.push(chunk);
      [status, body] = reply(request, chunks.length ? JSON.parse(Buffer.concat(chunks)) : {});
    } catch (error) {
      [status, body] = [400, { message: error.message }];
    }
    response.writeHead(status, { 'content-type': 'application/json' }).end(JSON.stringify(body));
  });
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  url = `http://127.0.0.1:${server.address().port}`;
  return {
    url, upstream, fork, pulls,
    files: (pull) => {
      const sha = refs.get(fork).get(pull.head.ref);
      return Object.fromEntries(Object.entries(trees.get(commits.get(sha))).map(([name, blob]) => [name, blobs.get(blob)]));
    },
    merge: (pull) => {
      Object.assign(pull, { state: 'closed', merged: true, merged_at: '2026-01-01T00:00:00Z' });
      refs.get(upstream).set('master', pull.head.sha);
    },
    close: () => new Promise((resolve, reject) => server.close((error) => error ? reject(error) : resolve())),
  };
}
