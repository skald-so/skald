// Stand-ins for what the release check, script/release-check, reaches, for script/release-check-test:
//   - serve(<world>): a stand-in GitHub, its releases' files, its REST API for releases, and a
//     stand-in npm registry, on 127.0.0.1 over HTTP, in the test's own process;
//   - as a command, `node release-check-stub.mjs <state> <tool> <args...>`: stand-ins of gh (its
//     release and api commands through the stand-in GitHub, and attestation verify), cosign
//     (verify), docker (buildx imagetools inspect) and crane (digest) against a stand-in GHCR, and
//     curl, which reaches registry.npmjs.org at the stand-in npm registry and nothing but 127.0.0.1
//     otherwise, and wget, which downloads nothing: the test has no network.
// A world, the test's object that serve() reads on each request:
//   releases    the GitHub Releases, oldest first: {tag, prerelease, immutable, dir}; <dir> holds
//               every file of the release
//   latest      the tag GitHub marks the latest release, or null for none
//   npm         the package skald-so: versions, each version to {tgz, manifest}; distTags;
//               provenance, each version to its provenance {repo, workflow, ref}, or none
//   requests    every request served: {method, path, agent}
//   apiLimit    optional: {wait, stale, forbidden}. Requests without a token, those of skald update
//               with the user agent skald/<version>, are answered 403 until <wait> seconds after
//               the first such request or read of /rate_limit past the first <stale> reads, which
//               report some left; /rate_limit reports none left until then. With <forbidden> they
//               are always answered 403, while /rate_limit reports some left.
// <state>, a JSON file the commands read: server, the stand-in's URL; curl, the real curl; calls, a
// file each command appends a JSON line to: {tool, args, status, subjects}; image, {name, tags},
// each tag of the image to its digest; signatures, cosign's: {digest, identity, issuer};
// attestations, GitHub Artifact Attestations of the release files and the image: {digest, repo,
// workflow, ref, predicateType}, <digest> a file's SHA-256 in hex or the image's sha256:<hex>.
import { spawnSync } from 'node:child_process';
import crypto from 'node:crypto';
import fs from 'node:fs';
import http from 'node:http';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

export const repository = 'skald-so/skald';
export const releaseWorkflow = 'skald-so/skald/.github/workflows/release.yml';
export const oidcIssuer = 'https://token.actions.githubusercontent.com';
export const imageName = 'ghcr.io/skald-so/skald-server';
export const provenanceType = 'https://slsa.dev/provenance/v1';
const publishType = 'https://github.com/npm/attestation/tree/main/specs/publish/v0.1';
// certificateIdentity <workflow> <ref>: the identity Fulcio gives a workflow of GitHub Actions.
export const certificateIdentity = (workflow, ref) => `https://github.com/${workflow}@${ref}`;

const digestOf = (algorithm, bytes) => crypto.createHash(algorithm).update(bytes).digest('hex');

// ---- The stand-in GitHub and npm registry ----

const assetsOf = (base, release, index) =>
  fs
    .readdirSync(release.dir)
    .sort()
    .map((name, n) => {
      const bytes = fs.readFileSync(path.join(release.dir, name));
      return {
        id: (index + 1) * 1000 + n,
        name,
        size: bytes.length,
        digest: `sha256:${digestOf('sha256', bytes)}`,
        state: 'uploaded',
        content_type: 'application/octet-stream',
        browser_download_url: `${base}/download/${release.tag}/${encodeURIComponent(name)}`,
        url: `${base}/api/repos/${repository}/releases/assets/${(index + 1) * 1000 + n}`,
      };
    });

const releaseJson = (base, world, release) => {
  const index = world.releases.indexOf(release);
  const day = String(index + 1).padStart(2, '0');
  return {
    id: index + 1,
    tag_name: release.tag,
    name: release.tag,
    draft: false,
    prerelease: release.prerelease,
    immutable: release.immutable,
    created_at: `2026-11-${day}T10:00:00Z`,
    published_at: `2026-11-${day}T10:05:00Z`,
    html_url: `https://github.com/${repository}/releases/tag/${release.tag}`,
    body: `Release ${release.tag}`,
    assets: assetsOf(base, release, index),
  };
};

const packument = (base, world) => {
  const versions = {};
  const time = {};
  for (const [version, { tgz, manifest }] of Object.entries(world.npm.versions)) {
    const bytes = fs.readFileSync(tgz);
    const dist = {
      tarball: `${base}/npm/skald-so/-/skald-so-${version}.tgz`,
      shasum: digestOf('sha1', bytes),
      integrity: `sha512-${crypto.createHash('sha512').update(bytes).digest('base64')}`,
    };
    if (world.npm.provenance[version]) {
      dist.attestations = {
        url: `${base}/npm/-/npm/v1/attestations/skald-so@${version}`,
        provenance: { predicateType: provenanceType },
      };
    }
    versions[version] = { ...manifest, _id: `skald-so@${version}`, dist };
    time[version] = '2026-11-01T10:10:00.000Z';
  }
  return { _id: 'skald-so', name: 'skald-so', 'dist-tags': world.npm.distTags, versions, time };
};

// The npm attestations of a version, as the registry serves them: its provenance and its publish
// attestation, each a stand-in Sigstore bundle.
const npmAttestations = (world, version) => {
  const found = world.npm.provenance[version];
  const tgz = world.npm.versions[version]?.tgz;
  if (!found || !tgz) return null;
  const subject = { name: `pkg:npm/skald-so@${version}`, sha512: digestOf('sha512', fs.readFileSync(tgz)) };
  const bundle = (standin) => ({ mediaType: 'application/vnd.dev.sigstore.bundle.v0.3+json', standin });
  return {
    attestations: [
      { predicateType: provenanceType, bundle: bundle({ subject, ...found, predicateType: provenanceType }) },
      { predicateType: publishType, bundle: bundle({ subject, predicateType: publishType, npm: true }) },
    ],
  };
};

// serve <world>: resolves to {url, close} once the stand-in listens.
export function serve(world) {
  const server = http.createServer((request, response) => {
    const url = new URL(request.url, 'http://127.0.0.1');
    const route = decodeURIComponent(url.pathname);
    world.requests.push({ method: request.method, path: route, agent: String(request.headers['user-agent'] ?? '') });
    const base = `http://127.0.0.1:${server.address().port}`;
    const json = (body) => response.writeHead(200, { 'content-type': 'application/json' }).end(JSON.stringify(body));
    const file = (name) => {
      response.writeHead(200, { 'content-type': 'application/octet-stream' });
      fs.createReadStream(name).pipe(response);
    };
    const missing = () => response.writeHead(404, { 'content-type': 'application/json' }).end('{"message":"Not Found"}');
    if (request.method !== 'GET' && request.method !== 'HEAD') return missing();

    let m = route.match(/^\/download\/([^/]+)\/([^/]+)$/);
    if (m) {
      const release = world.releases.find((r) => r.tag === m[1]);
      const name = release && path.join(release.dir, m[2]);
      return name && fs.statSync(name, { throwIfNoEntry: false })?.isFile() ? file(name) : missing();
    }
    const now = Math.floor(Date.now() / 1000);
    const limit = world.apiLimit;
    const tokenless = String(request.headers['user-agent'] ?? '').startsWith('skald/');
    const stale = Boolean(limit) && route === '/api/rate_limit' && limit.reset === undefined && (limit.stale ?? 0) > 0;
    if (stale) limit.stale--;
    else if (limit && (route === '/api/rate_limit' || tokenless)) limit.reset ??= now + limit.wait;
    const usedUp = Boolean(limit && !limit.forbidden && now < limit.reset);
    if (route === '/api/rate_limit') {
      const left = !usedUp || stale;
      const core = { limit: 60, used: left ? 0 : 60, remaining: left ? 60 : 0, reset: left ? now + 3600 : limit.reset };
      return json({ resources: { core }, rate: core });
    }
    if (route.startsWith('/api/') && tokenless && (usedUp || limit?.forbidden)) {
      return response.writeHead(403, { 'content-type': 'application/json' }).end(JSON.stringify({ message: usedUp ? 'API rate limit exceeded' : 'Forbidden' }));
    }
    m = route.match(/^\/api\/repos\/([^/]+)\/([^/]+)\/releases(\/.*)?$/i);
    if (m) {
      if (`${m[1]}/${m[2]}`.toLowerCase() !== repository) return missing();
      const rest = m[3] ?? '';
      const page = Number(url.searchParams.get('page') ?? 1);
      if (rest === '' || rest === '/') {
        return json(page > 1 ? [] : [...world.releases].reverse().map((r) => releaseJson(base, world, r)));
      }
      let release = null;
      if (rest === '/latest') release = world.releases.find((r) => r.tag === world.latest);
      else if ((m = rest.match(/^\/tags\/(.+)$/))) release = world.releases.find((r) => r.tag === m[1]);
      else if ((m = rest.match(/^\/assets\/(\d+)$/))) {
        for (const r of world.releases) {
          const asset = assetsOf(base, r, world.releases.indexOf(r)).find((a) => a.id === Number(m[1]));
          if (!asset) continue;
          if (/octet-stream/.test(String(request.headers.accept ?? ''))) return file(path.join(r.dir, asset.name));
          return json(asset);
        }
        return missing();
      } else if ((m = rest.match(/^\/(\d+)(\/assets)?$/))) {
        release = world.releases[Number(m[1]) - 1] ?? null;
        if (release && m[2]) return json(page > 1 ? [] : releaseJson(base, world, release).assets);
      }
      return release ? json(releaseJson(base, world, release)) : missing();
    }
    if (route === '/npm/skald-so') return Object.keys(world.npm.versions).length ? json(packument(base, world)) : missing();
    m = route.match(/^\/npm\/skald-so\/-\/skald-so-(.+)\.tgz$/);
    if (m) return world.npm.versions[m[1]] ? file(world.npm.versions[m[1]].tgz) : missing();
    m = route.match(/^\/npm\/-\/npm\/v1\/attestations\/skald-so@(.+)$/);
    if (m) {
      const found = npmAttestations(world, m[1]);
      return found ? json(found) : missing();
    }
    return missing();
  });
  return new Promise((resolve) => {
    server.listen(0, '127.0.0.1', () =>
      resolve({ url: `http://127.0.0.1:${server.address().port}`, close: () => new Promise((done) => server.close(done)) }),
    );
  });
}

// ---- The stand-in commands ----

// split <args> <flags with a value>: {named: {flag: [values]}, plain: [arguments]}. A flag outside
// <flags with a value> takes none unless written --flag=value.
function split(args, valued) {
  const named = {};
  const plain = [];
  let i = 0;
  while (i < args.length) {
    const arg = args[i++];
    if (arg === '--') {
      plain.push(...args.slice(i));
      break;
    }
    if (!/^-./.test(arg)) {
      plain.push(arg);
      continue;
    }
    const [flag, inline] = arg.includes('=') ? [arg.slice(0, arg.indexOf('=')), arg.slice(arg.indexOf('=') + 1)] : [arg, undefined];
    const value = inline !== undefined ? inline : valued.includes(flag) ? args[i++] : true;
    named[flag] = [...(named[flag] ?? []), value];
  }
  return { named, plain };
}
const one = (parsed, ...flags) => flags.map((f) => parsed.named[f]?.at(-1)).find((v) => v !== undefined);

async function command(stateFile, tool, args) {
  const state = JSON.parse(fs.readFileSync(stateFile, 'utf8'));
  const subjects = [];
  const say = (text) => process.stdout.write(`${text}\n`);
  const complain = (text) => process.stderr.write(`${text}\n`);
  const unsupported = () => {
    complain(`release-check-stub: the test's stand-in ${tool} cannot run: ${tool} ${args.join(' ')}`);
    return 64;
  };
  // api <route>: the stand-in GitHub's answer, {status, body}.
  const api = async (route) => {
    const address = `${state.server}/api/${route.replace(/^\/+/, '').replace('{owner}/{repo}', repository)}`;
    const response = await fetch(address);
    return { status: response.status, body: await response.text() };
  };
  const jq = (filter, body) => {
    const result = spawnSync('jq', ['-r', filter], { input: body, encoding: 'utf8' });
    process.stdout.write(result.stdout ?? '');
    process.stderr.write(result.stderr ?? '');
    return result.status ?? 1;
  };
  // resolve <reference>: the stand-in GHCR's digest of an image reference, or null.
  const resolve = (reference) => {
    const text = String(reference).replace(/^(oci|docker):\/\//, '');
    const at = text.indexOf('@');
    const name = at >= 0 ? text.slice(0, at) : text.replace(/:[^:/]+$/, '');
    if (name !== state.image.name) return null;
    if (at >= 0) return Object.values(state.image.tags).includes(text.slice(at + 1)) ? text.slice(at + 1) : null;
    const tag = text.match(/:([^:/]+)$/)?.[1] ?? 'latest';
    return state.image.tags[tag] ?? null;
  };

  const tools = {
    async gh() {
      const [verb, ...rest] = args;
      if (verb === '--version' || verb === 'version') return say('gh version 2.97.0 (2026-10-01)'), 0;
      if (verb === 'auth') return rest[0] === 'token' ? (say('stub-token'), 0) : 0;
      if (verb === 'attestation' && rest[0] === 'verify') return verifyAttestation(rest.slice(1));
      const parsed = split(rest, ['-R', '--repo', '--json', '-q', '--jq', '-t', '--template', '-p', '--pattern', '-D', '--dir',
        '-O', '--output', '-A', '--archive', '-L', '--limit', '-X', '--method', '-H', '--header', '-f', '-F', '--field',
        '--raw-field', '--order', '--hostname', '--cache']);
      const repo = one(parsed, '-R', '--repo') ?? process.env.GH_REPO ?? repository;
      if (String(repo).replace(/^https:\/\/github\.com\//, '').toLowerCase() !== repository) {
        return complain(`gh: the stand-in GitHub has no repository ${repo}`), 1;
      }
      const filter = one(parsed, '-q', '--jq');
      if (verb === 'api') {
        if (String(one(parsed, '-X', '--method') ?? 'GET').toUpperCase() !== 'GET') return unsupported();
        const { status, body } = await api(String(parsed.plain[0] ?? ''));
        if (status >= 300) return complain(`gh: Not Found (HTTP ${status})`), 1;
        return filter ? jq(filter, body) : (process.stdout.write(body), 0);
      }
      if (verb !== 'release') return unsupported();
      const [sub, tag] = parsed.plain;
      const latestFetched = await api(`repos/${repository}/releases/latest`);
      const latest = latestFetched.status < 300 ? JSON.parse(latestFetched.body) : null;
      if (sub === 'list') {
        const listed = JSON.parse((await api(`repos/${repository}/releases`)).body)
          .filter((r) => !(parsed.named['--exclude-pre-releases'] && r.prerelease))
          .slice(0, Number(one(parsed, '-L', '--limit') ?? 30));
        if (one(parsed, '--order') === 'asc') listed.reverse();
        return output(listed.map((r) => ghRelease(r, latest)), parsed, filter, (r) => `${r.tagName}\t${r.isPrerelease ? 'Pre-release' : ''}`);
      }
      const fetched = tag ? await api(`repos/${repository}/releases/tags/${tag}`) : latestFetched;
      if (fetched.status >= 300) return complain('release not found'), 1;
      const release = JSON.parse(fetched.body);
      if (sub === 'view') return output([ghRelease(release, latest)], parsed, filter, (r) => `title:\t${r.name}\ntag:\t${r.tagName}`, true);
      if (sub === 'download') {
        const dir = String(one(parsed, '-D', '--dir') ?? '.');
        const patterns = (parsed.named['-p'] ?? parsed.named['--pattern'] ?? ['*']).map((p) => new RegExp(`^${String(p).replace(/[.+^$()|{}\\]/g, '\\$&').replace(/\*/g, '.*').replace(/\?/g, '.')}$`));
        const chosen = release.assets.filter((a) => patterns.some((p) => p.test(a.name)));
        if (chosen.length === 0) return complain('no assets to download'), 1;
        fs.mkdirSync(dir, { recursive: true });
        for (const asset of chosen) {
          const response = await fetch(asset.browser_download_url);
          if (!response.ok) return complain(`HTTP ${response.status}: ${asset.name}`), 1;
          const into = one(parsed, '-O', '--output') ?? path.join(dir, asset.name);
          fs.writeFileSync(String(into), Buffer.from(await response.arrayBuffer()));
        }
        return 0;
      }
      return unsupported();
    },
    cosign() {
      const [verb, ...rest] = args;
      if (verb === 'version') return say('GitVersion:    v3.1.4'), 0;
      if (verb !== 'verify') return unsupported();
      const parsed = split(rest, ['--certificate-identity', '--certificate-identity-regexp', '--certificate-oidc-issuer',
        '--certificate-oidc-issuer-regexp', '--certificate-github-workflow-repository', '--certificate-github-workflow-ref',
        '-o', '--output', '--key', '--trusted-root', '--bundle', '--timeout', '-a', '--annotations', '--attachment']);
      const identity = one(parsed, '--certificate-identity');
      const identityPattern = one(parsed, '--certificate-identity-regexp');
      const issuer = one(parsed, '--certificate-oidc-issuer');
      const issuerPattern = one(parsed, '--certificate-oidc-issuer-regexp');
      if (identity === undefined && identityPattern === undefined) {
        return complain('Error: --certificate-identity or --certificate-identity-regexp is required for verification in keyless mode'), 1;
      }
      if (issuer === undefined && issuerPattern === undefined) {
        return complain('Error: --certificate-oidc-issuer or --certificate-oidc-issuer-regexp is required for verification in keyless mode'), 1;
      }
      if (parsed.plain.length === 0) return unsupported();
      let status = 0;
      for (const reference of parsed.plain) {
        const digest = resolve(reference);
        if (!digest) {
          complain(`Error: ${reference}: MANIFEST_UNKNOWN: manifest unknown`);
          status = 1;
          continue;
        }
        subjects.push(digest);
        const matched = state.signatures.filter(
          (s) =>
            s.digest === digest &&
            (identity === undefined || s.identity === identity) &&
            (identityPattern === undefined || new RegExp(identityPattern).test(s.identity)) &&
            (issuer === undefined || s.issuer === issuer) &&
            (issuerPattern === undefined || new RegExp(issuerPattern).test(s.issuer)) &&
            (one(parsed, '--certificate-github-workflow-ref') === undefined || s.identity.endsWith(`@${one(parsed, '--certificate-github-workflow-ref')}`)),
        );
        if (matched.length === 0) {
          complain('Error: no matching signatures: none of the expected identities matched what was in the certificate');
          status = 1;
        } else {
          complain(`\nVerification for ${reference} --\nThe following checks were performed on each of these signatures:`);
          say(JSON.stringify([{ critical: { image: { 'docker-manifest-digest': digest } }, optional: { Subject: matched[0].identity } }]));
        }
      }
      return status;
    },
    docker() {
      if (args[0] === 'version' || args[0] === 'login') return 0;
      if (args[0] !== 'buildx' || args[1] !== 'imagetools' || args[2] !== 'inspect') return unsupported();
      const parsed = split(args.slice(3), ['--format', '--builder']);
      const reference = parsed.plain[0];
      const digest = resolve(reference);
      if (!digest) return complain(`ERROR: ${reference}: not found`), 1;
      const descriptor = { mediaType: 'application/vnd.oci.image.index.v1+json', digest, size: 1609 };
      const format = one(parsed, '--format');
      if (parsed.named['--raw']) return say(JSON.stringify({ schemaVersion: 2, mediaType: descriptor.mediaType, manifests: [] })), 0;
      if (format === undefined) return say(`Name:      ${reference}\nMediaType: ${descriptor.mediaType}\nDigest:    ${digest}`), 0;
      if (/json\s+\.Manifest\b/.test(format)) return say(JSON.stringify(descriptor)), 0;
      if (/json\s+\./.test(format)) return say(JSON.stringify({ name: reference, manifest: descriptor })), 0;
      return say(digest), 0;
    },
    crane() {
      if (args[0] !== 'digest') return unsupported();
      const reference = split(args.slice(1), ['--platform']).plain[0];
      const digest = resolve(reference);
      return digest ? (say(digest), 0) : (complain(`Error: ${reference}: MANIFEST_UNKNOWN`), 1);
    },
    curl() {
      const rewritten = args.map((a) => a.replace(/^https:\/\/registry\.npmjs\.org\//, `${state.server}/npm/`));
      const elsewhere = rewritten.find((a) => /^[a-z]+:\/\//i.test(a) && !/^https?:\/\/(127\.0\.0\.1|localhost)[:/]/i.test(a));
      if (elsewhere) return complain(`curl: (6) Could not resolve host: ${new URL(elsewhere).host}`), 6;
      const result = spawnSync(state.curl, rewritten, { stdio: 'inherit' });
      return result.status ?? 1;
    },
    wget() {
      return complain('wget: the test has no network; the stand-in wget downloads nothing'), 4;
    },
  };

  // output <items> <parsed> <filter> <text> <single>: what `gh release list|view` prints: --json
  // fields, through --jq, or text.
  function output(items, parsed, filter, text, single = false) {
    const fields = one(parsed, '--json');
    if (fields === undefined || fields === true) return say(items.map(text).join('\n')), 0;
    const wanted = String(fields).split(',').map((f) => f.trim());
    const known = items.length > 0 ? items[0] : ghRelease({}, null);
    const unknownField = wanted.find((f) => !(f in known));
    if (unknownField) return complain(`Unknown JSON field: "${unknownField}"`), 1;
    const picked = items.map((item) => Object.fromEntries(wanted.map((f) => [f, item[f]])));
    const body = JSON.stringify(single ? picked[0] : picked);
    return filter ? jq(filter, body) : (say(body), 0);
  }

  // verifyAttestation <args>: `gh attestation verify`, with the policy its flags set.
  function verifyAttestation(rest) {
    const parsed = split(rest, ['-R', '--repo', '-o', '--owner', '--signer-workflow', '--signer-repo', '--source-ref',
      '--source-digest', '--cert-identity', '--cert-identity-regex', '--cert-oidc-issuer', '--predicate-type', '-b',
      '--bundle', '-d', '--digest-alg', '--format', '-q', '--jq', '--hostname', '--limit', '-L', '--bundle-from-oci']);
    const subject = parsed.plain[0];
    const repo = one(parsed, '-R', '--repo');
    const owner = one(parsed, '-o', '--owner');
    if (repo === undefined && owner === undefined) return complain('Error: one of --owner or --repo is required'), 1;
    if (subject === undefined) return complain('Error: accepts 1 arg(s), received 0'), 1;
    let digest;
    if (/^oci:\/\//.test(subject)) {
      digest = resolve(subject);
      if (!digest) return complain(`Error: failed to fetch ${subject}: not found`), 1;
    } else {
      if (!fs.statSync(subject, { throwIfNoEntry: false })?.isFile()) return complain(`Error: failed to read ${subject}`), 1;
      digest = digestOf(String(one(parsed, '-d', '--digest-alg') ?? 'sha256'), fs.readFileSync(subject));
    }
    subjects.push(digest);
    const bundle = one(parsed, '-b', '--bundle');
    let candidates;
    if (bundle !== undefined) {
      const text = fs.readFileSync(String(bundle), 'utf8');
      let entries;
      try {
        entries = [].concat(JSON.parse(text));
      } catch {
        entries = text.split('\n').filter((l) => l.trim()).map((l) => JSON.parse(l));
      }
      candidates = entries
        .map((e) => e?.standin ?? e?.bundle?.standin)
        .filter((s) => s && !s.npm && (s.subject?.sha512 === digest || s.subject?.sha256 === digest));
    } else candidates = state.attestations.filter((a) => a.digest === digest);
    const signerWorkflow = one(parsed, '--signer-workflow');
    const signerRepo = one(parsed, '--signer-repo');
    const sourceRef = one(parsed, '--source-ref');
    const identity = one(parsed, '--cert-identity');
    const identityPattern = one(parsed, '--cert-identity-regex');
    const issuer = one(parsed, '--cert-oidc-issuer');
    const predicate = one(parsed, '--predicate-type') ?? provenanceType;
    const workflowOf = (text) => String(text).replace(/^https?:\/\//, '').replace(/^github\.com\//, '').replace(/@.*$/, '');
    const matched = candidates.filter((a) => {
      const cert = certificateIdentity(a.workflow, a.ref);
      if (repo !== undefined && String(repo).toLowerCase() !== a.repo.toLowerCase()) return false;
      if (owner !== undefined && String(owner).toLowerCase() !== a.repo.split('/')[0].toLowerCase()) return false;
      if (signerWorkflow !== undefined) {
        if (workflowOf(signerWorkflow).toLowerCase() !== a.workflow.toLowerCase()) return false;
      } else if (signerRepo !== undefined) {
        if (!a.workflow.toLowerCase().startsWith(`${String(signerRepo).toLowerCase()}/`)) return false;
      } else if (identity === undefined && identityPattern === undefined) {
        if (!a.workflow.toLowerCase().startsWith(`${String(repo ?? owner).toLowerCase()}/`)) return false;
      }
      if (sourceRef !== undefined && sourceRef !== a.ref) return false;
      if (identity !== undefined && identity !== cert) return false;
      if (identityPattern !== undefined && !new RegExp(identityPattern).test(cert)) return false;
      if (issuer !== undefined && issuer !== oidcIssuer) return false;
      return a.predicateType === predicate;
    });
    if (matched.length === 0) {
      complain('Error: verifying with issuer "sigstore.dev": no attestations matched the verification policy');
      return 1;
    }
    say('✓ Verification succeeded!');
    return 0;
  }

  const run = tools[tool];
  const status = run ? await run() : unsupported();
  fs.appendFileSync(state.calls, `${JSON.stringify({ tool, args, status, subjects })}\n`);
  return status;
}

// ghRelease <API release> <API latest release>: the release as `gh release` names its JSON fields.
function ghRelease(r, latest) {
  return {
    id: String(r.id),
    databaseId: r.id,
    tagName: r.tag_name,
    name: r.name,
    isDraft: r.draft,
    isPrerelease: r.prerelease,
    isImmutable: r.immutable,
    isLatest: latest ? latest.tag_name === r.tag_name : false,
    createdAt: r.created_at,
    publishedAt: r.published_at,
    url: r.html_url,
    body: r.body,
    assets: (r.assets ?? []).map((a) => ({
      name: a.name,
      size: a.size,
      digest: a.digest,
      url: a.browser_download_url,
      apiUrl: a.url,
      contentType: a.content_type,
      state: a.state,
    })),
  };
}

if (process.argv[1] && import.meta.url === pathToFileURL(fs.realpathSync(process.argv[1])).href) {
  const [stateFile, tool, ...args] = process.argv.slice(2);
  process.exitCode = await command(stateFile, tool, args);
}
