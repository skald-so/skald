// A stand-in for the registry tools a step of the release workflow may run, for
// script/release-image-test: docker, crane, skopeo, regctl and oras against a stand-in GHCR, cosign,
// gh (its API for the image's package versions, and attestation verify), and curl and wget, which
// fail: the test has no network.
// The stand-in GHCR holds one image, its tags and its digests; the docker daemon, the images built
// with their local tags. Both live in <state>, a JSON file each call reads and writes back:
//   image       the image's name, ghcr.io/skald-so/skald-server
//   tags        each tag of the image to its digest
//   manifests   the digests the registry has
//   built       the digests pushed from a build of this run
//   local       each local tag to its image ID; ids, each image ID to its pushed digest
//   writes      every tag or digest written to the image: {tool, ref, tag, digest, before}
//   elsewhere   every write to another image: {tool, ref}
//   builds      every image build: {tool, file, context, buildArgs, target, cacheFrom, push, tags}
//   signatures  every cosign sign or attest: {command, ref, ours, tag, byDigest, digest, key}
//   attestations  GitHub Artifact Attestations of the image, which the test adds: {name, digest}
//   unsupported every call this stand-in cannot answer
// Usage: node registry-stub.mjs <state> <tool> <args...>
import { spawnSync } from 'node:child_process';
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';

const [stateFile, tool, ...args] = process.argv.slice(2);
const state = JSON.parse(fs.readFileSync(stateFile, 'utf8'));
let status = 0;

const out = (text) => process.stdout.write(`${text}\n`);
const err = (text) => process.stderr.write(`${text}\n`);
const newDigest = () => `sha256:${crypto.randomBytes(32).toString('hex')}`;
const unsupported = () => {
  state.unsupported.push(`${tool} ${args.join(' ')}`);
  err(`registry-stub: the test's stand-in ${tool} cannot run: ${tool} ${args.join(' ')}`);
  status = 1;
};
const notFound = (ref) => {
  err(`${tool}: ${ref}: not found: MANIFEST_UNKNOWN: manifest unknown`);
  status = 1;
};

// parse <reference>: {repo, tag, digest} of an image reference, with or without a transport.
function parse(text) {
  let rest = String(text).replace(/^(docker|oci):\/\//, '');
  let digest = null;
  const at = rest.indexOf('@');
  if (at >= 0) {
    digest = rest.slice(at + 1);
    rest = rest.slice(0, at);
  }
  let tag = null;
  const colon = rest.lastIndexOf(':');
  if (colon > rest.lastIndexOf('/')) {
    tag = rest.slice(colon + 1);
    rest = rest.slice(0, colon);
  }
  return { repo: rest, tag, digest };
}
const ours = (ref) => parse(ref).repo === state.image;
// resolve <reference>: the digest the registry has for it, or null.
function resolve(ref) {
  const r = parse(ref);
  if (r.repo !== state.image) return null;
  if (r.digest) return state.manifests.includes(r.digest) ? r.digest : null;
  return state.tags[r.tag ?? 'latest'] ?? null;
}
// local <reference>: the reference as the docker daemon keys it, with :latest when it has no tag.
const local = (ref) => {
  const r = parse(ref);
  return r.tag || r.digest ? String(ref) : `${ref}:latest`;
};
// write <reference> <digest>: the registry stores <digest> under the reference.
function write(ref, digest) {
  const r = parse(ref);
  if (r.repo !== state.image) {
    state.elsewhere.push({ tool, ref: String(ref) });
    return;
  }
  if (!state.manifests.includes(digest)) state.manifests.push(digest);
  const before = r.tag ? (state.tags[r.tag] ?? null) : null;
  state.writes.push({ tool, ref: String(ref), tag: r.tag, digest, before });
  if (r.tag) state.tags[r.tag] = digest;
}
const tagsOf = (digest) => Object.keys(state.tags).filter((t) => state.tags[t] === digest);

// options <args> <valued flags>: {flags: {name: [values]}, positional}; a flag not in <valued> takes
// no value unless it is written --flag=value.
function options(list, valued) {
  const flags = {};
  const positional = [];
  for (let i = 0; i < list.length; i++) {
    const a = list[i];
    if (a === '--') {
      positional.push(...list.slice(i + 1));
      break;
    }
    if (/^-/.test(a) && a !== '-') {
      const eq = a.indexOf('=');
      const name = eq >= 0 ? a.slice(0, eq) : a;
      let value = eq >= 0 ? a.slice(eq + 1) : true;
      if (eq < 0 && valued.includes(name)) value = list[++i];
      (flags[name] ??= []).push(value);
    } else positional.push(a);
  }
  return { flags, positional };
}
const values = (o, ...names) => names.flatMap((n) => o.flags[n] ?? []);
const last = (o, ...names) => values(o, ...names).at(-1);
const has = (o, ...names) => values(o, ...names).some((v) => v !== false && v !== 'false');

const buildValued = ['-f', '--file', '-t', '--tag', '--build-arg', '--target', '-o', '--output', '--platform',
  '--cache-from', '--cache-to', '--metadata-file', '--iidfile', '--label', '--progress', '--secret', '--ssh',
  '--network', '--builder', '--add-host', '--allow', '--annotation', '--attest', '--build-context', '--shm-size',
  '--ulimit', '--call', '--cgroup-parent'];

function build(list) {
  const o = options(list, buildValued);
  const context = o.positional[0] ?? '.';
  const file = last(o, '-f', '--file');
  const buildArgs = {};
  for (const a of values(o, '--build-arg')) {
    const eq = String(a).indexOf('=');
    if (eq >= 0) buildArgs[a.slice(0, eq)] = a.slice(eq + 1);
    else buildArgs[a] = process.env[a] ?? '';
  }
  const tags = values(o, '-t', '--tag').map(String);
  const outputs = values(o, '-o', '--output').map(String);
  const push = has(o, '--push') || outputs.some((x) => /push=true|type=registry/.test(x));
  const remote = /^[a-z]+:\/\/|^git@/.test(context);
  const contextDir = remote ? context : path.resolve(context);
  state.builds.push({
    tool,
    file: file === undefined ? path.join(contextDir, 'Dockerfile') : remote ? file : path.resolve(file),
    context: contextDir,
    buildArgs,
    target: last(o, '--target') ?? null,
    cacheFrom: values(o, '--cache-from').map(String),
    push,
    tags,
  });
  const id = newDigest();
  for (const t of tags) state.local[local(t)] = id;
  let digest = null;
  if (push) {
    digest = newDigest();
    state.ids[id] = digest;
    state.built.push(digest);
    for (const t of tags) write(t, digest);
    if (tags.length === 0) {
      err('ERROR: tag is needed when pushing to registry');
      status = 1;
    }
  }
  const metadata = last(o, '--metadata-file');
  if (metadata) {
    fs.writeFileSync(
      metadata,
      JSON.stringify({ 'containerimage.digest': digest ?? id, 'containerimage.config.digest': id, 'image.name': tags.join(',') }),
    );
  }
  const iid = last(o, '--iidfile');
  if (iid) fs.writeFileSync(iid, id);
  err(`#1 exporting to image${digest ? `\n#1 pushing manifest for ${tags.join(', ')}@${digest}` : ''}`);
}

// inspect <reference> <format>: what `docker buildx imagetools inspect` and the like print.
function inspect(ref, format) {
  const digest = resolve(ref);
  if (!digest) return notFound(ref);
  if (format && /json/i.test(format)) out(JSON.stringify({ name: ref, digest, manifest: { digest } }));
  else if (format) out(digest);
  else out(`Name:      ${ref}\nMediaType: application/vnd.oci.image.index.v1+json\nDigest:    ${digest}`);
}
const manifest = (ref) => {
  const digest = resolve(ref);
  if (!digest) return notFound(ref);
  out(JSON.stringify({ schemaVersion: 2, mediaType: 'application/vnd.oci.image.index.v1+json', manifests: [] }));
};
// copy <source> <destination>: the destination gets the source's digest.
function copy(source, destination) {
  let digest = resolve(source);
  if (!digest && /^docker-daemon:/.test(source)) {
    const id = state.local[local(source.replace(/^docker-daemon:/, ''))];
    if (id) {
      digest = state.ids[id] ?? newDigest();
      state.ids[id] = digest;
      state.built.push(digest);
    }
  }
  if (!digest) return notFound(source);
  write(String(destination).replace(/^docker:\/\//, ''), digest);
}
const listTags = () => out(Object.keys(state.tags).join('\n'));

function docker(list) {
  const [command, ...rest] = list;
  const sub = rest[0];
  if (['login', 'logout', 'version', 'info'].includes(command)) return;
  if (command === 'buildx' && ['version', 'create', 'use', 'inspect', 'ls', 'install'].includes(sub)) return;
  if (command === 'buildx' && sub === 'build') return build(rest.slice(1));
  if (command === 'build' || (['image', 'builder'].includes(command) && sub === 'build')) {
    return build(command === 'build' ? rest : rest.slice(1));
  }
  if (command === 'buildx' && sub === 'imagetools') {
    const [verb, ...more] = rest.slice(1);
    if (verb === 'inspect') {
      const o = options(more, ['--format', '--builder']);
      return inspect(o.positional[0], last(o, '--format'));
    }
    if (verb === 'create') {
      const o = options(more, ['-t', '--tag', '-f', '--file', '--annotation', '--builder', '--progress', '--platform']);
      const sources = o.positional.map((s) => [s, resolve(s)]);
      const missing = sources.find(([, d]) => !d);
      if (missing) return notFound(missing[0]);
      let digest = sources.length === 1 && !has(o, '--append') ? sources[0][1] : newDigest();
      if (sources.length > 1 && sources.every(([, d]) => state.built.includes(d))) state.built.push(digest);
      if (has(o, '--dry-run')) return out(JSON.stringify({ digest }));
      for (const t of values(o, '-t', '--tag')) write(t, digest);
      return;
    }
    return unsupported();
  }
  if (command === 'manifest' && sub === 'inspect') return manifest(options(rest.slice(1), []).positional[0]);
  const verb = command === 'image' ? sub : command;
  const more = command === 'image' ? rest.slice(1) : rest;
  if (verb === 'push') {
    const ref = options(more, []).positional[0];
    const id = state.local[local(ref)];
    if (!id) {
      err(`An image does not exist locally with the tag: ${ref}`);
      status = 1;
      return;
    }
    const digest = state.ids[id] ?? newDigest();
    state.ids[id] = digest;
    if (!state.built.includes(digest)) state.built.push(digest);
    write(local(ref), digest);
    return out(`${parse(local(ref)).tag}: digest: ${digest} size: 1234`);
  }
  if (verb === 'tag') {
    const [source, destination] = options(more, []).positional;
    const id = state.local[local(source)];
    if (!id) {
      err(`Error response from daemon: No such image: ${source}`);
      status = 1;
      return;
    }
    state.local[local(destination)] = id;
    return;
  }
  if (verb === 'pull') {
    const ref = options(more, ['--platform']).positional[0];
    const digest = resolve(ref);
    if (!digest) return notFound(ref);
    const id = `sha256:${crypto.createHash('sha256').update(digest).digest('hex')}`;
    state.ids[id] = digest;
    state.local[local(ref)] = id;
    return out(`Digest: ${digest}`);
  }
  if (verb === 'inspect') {
    const o = options(more, ['-f', '--format', '--type']);
    const id = state.local[local(o.positional[0])];
    if (!id) {
      err(`Error: No such object: ${o.positional[0]}`);
      status = 1;
      return;
    }
    const repoDigests = state.ids[id] ? [`${state.image}@${state.ids[id]}`] : [];
    const format = last(o, '-f', '--format');
    if (format && /RepoDigests/.test(format)) return out(repoDigests[0] ?? '');
    if (format && /\.Id\b/.test(format)) return out(id);
    return out(JSON.stringify([{ Id: id, RepoDigests: repoDigests }]));
  }
  return unsupported();
}

function crane([command, ...rest]) {
  const o = options(rest, ['--platform', '-u', '--username', '-p', '--password']);
  const [a, b] = o.positional;
  if (['auth', 'version', 'validate', 'config'].includes(command)) return;
  if (command === 'digest') {
    const digest = resolve(a);
    return digest ? out(digest) : notFound(a);
  }
  if (command === 'manifest') return manifest(a);
  if (command === 'ls') return listTags();
  if (command === 'copy' || command === 'cp') return copy(a, b);
  if (command === 'tag') return copy(a, `${parse(a).repo}:${b}`);
  if (command === 'push') {
    const digest = newDigest();
    state.built.push(digest);
    return write(b, digest);
  }
  return unsupported();
}

function skopeo([command, ...rest]) {
  const o = options(rest, ['--format', '-f', '--authfile', '--creds', '--username', '--password']);
  const [a, b] = o.positional;
  if (command === 'login' || command === 'logout') return;
  if (command === 'inspect') {
    const digest = resolve(a);
    if (!digest) return notFound(a);
    const format = last(o, '--format', '-f');
    return out(format ? digest : JSON.stringify({ Name: parse(a).repo, Digest: digest, RepoTags: Object.keys(state.tags) }));
  }
  if (command === 'list-tags') return out(JSON.stringify({ Repository: parse(a).repo, Tags: Object.keys(state.tags) }));
  if (command === 'copy') return copy(a, b);
  return unsupported();
}

function regctl([command, verb, ...rest]) {
  const o = options(rest, ['--format', '--platform']);
  const [a, b] = o.positional;
  if (command === 'registry' || command === 'version') return;
  if (command === 'manifest' && (verb === 'head' || verb === 'digest')) {
    const digest = resolve(a);
    return digest ? out(digest) : notFound(a);
  }
  if (command === 'manifest' && verb === 'get') return manifest(a);
  if (command === 'image' && verb === 'digest') {
    const digest = resolve(a);
    return digest ? out(digest) : notFound(a);
  }
  if (command === 'image' && (verb === 'copy' || verb === 'cp')) return copy(a, b);
  if (command === 'tag' && (verb === 'ls' || verb === 'list')) return listTags();
  return unsupported();
}

function oras([command, ...rest]) {
  if (command === 'login' || command === 'logout' || command === 'version') return;
  if (command === 'resolve') {
    const ref = options(rest, ['--platform']).positional[0];
    const digest = resolve(ref);
    return digest ? out(digest) : notFound(ref);
  }
  if (command === 'manifest' && rest[0] === 'fetch') return manifest(options(rest.slice(1), ['--platform']).positional[0]);
  if (command === 'repo' && (rest[0] === 'tags' || rest[0] === 'ls')) return listTags();
  if (command === 'tag') {
    const [source, ...tags] = options(rest, []).positional;
    for (const t of tags) copy(source, `${parse(source).repo}:${t}`);
    return;
  }
  if (command === 'cp' || command === 'copy') {
    const [a, b] = options(rest, []).positional;
    return copy(a, b);
  }
  return unsupported();
}

const cosignValued = ['--key', '-a', '--annotations', '--oidc-issuer', '--oidc-provider', '--oidc-client-id',
  '--oidc-client-secret-file', '--identity-token', '--output-signature', '--output-certificate', '--output-payload',
  '--output-file', '--certificate', '--certificate-chain', '--rekor-url', '--fulcio-url', '--registry-username',
  '--registry-password', '--registry-token', '--predicate', '--type', '--signing-config', '--trusted-root',
  '--bundle', '--timeout', '-t', '--attachment', '--payload', '--sk', '--slot', '--certificate-identity',
  '--certificate-identity-regexp', '--certificate-oidc-issuer', '--certificate-oidc-issuer-regexp', '--ca-roots',
  '--ca-intermediates'];

function cosign([command, ...rest]) {
  const o = options(rest, cosignValued);
  if (command === 'version') return out('GitVersion:    v2.6.1');
  if (['sign', 'attest', 'attach', 'copy', 'upload'].includes(command)) {
    if (command !== 'sign' && command !== 'attest') return unsupported();
    for (const ref of o.positional) {
      const r = parse(ref);
      const digest = resolve(ref);
      state.signatures.push({
        command,
        ref,
        ours: ours(ref),
        tag: r.tag,
        byDigest: Boolean(r.digest),
        digest,
        key:
          has(o, '--key', '--sk') ||
          Boolean(process.env.COSIGN_KEY || process.env.COSIGN_PRIVATE_KEY || process.env.COSIGN_PASSWORD),
      });
      if (!digest) notFound(ref);
    }
    if (o.positional.length === 0) unsupported();
    return;
  }
  if (command === 'verify' || command === 'verify-attestation') {
    const kind = command === 'verify' ? 'sign' : 'attest';
    for (const ref of o.positional) {
      const digest = resolve(ref);
      if (!digest || !state.signatures.some((s) => s.command === kind && s.digest === digest)) {
        err(`Error: no matching signatures: ${ref}`);
        status = 1;
      }
    }
    return;
  }
  if (command === 'triangulate') {
    const digest = resolve(o.positional[0]);
    return digest ? out(`${state.image}:${digest.replace(':', '-')}.sig`) : notFound(o.positional[0]);
  }
  return unsupported();
}

function gh([command, ...rest]) {
  if (command === 'api') {
    const o = options(rest, ['-X', '--method', '-H', '--header', '-f', '-F', '--field', '--raw-field', '--jq', '-q',
      '--template', '-t', '--input', '--hostname', '--cache', '-p', '--preview']);
    const method = String(last(o, '-X', '--method') ?? 'GET').toUpperCase();
    const route = String(o.positional[0] ?? '');
    if (method !== 'GET' || !/\/packages\/container\/skald-server\/versions(\?|$)/.test(route)) return unsupported();
    const digests = [...new Set(Object.values(state.tags))];
    const body = JSON.stringify(
      digests.map((digest, i) => ({
        id: i + 1,
        name: digest,
        metadata: { package_type: 'container', container: { tags: tagsOf(digest) } },
      })),
    );
    const filter = last(o, '--jq', '-q');
    if (!filter) return out(body);
    const jq = spawnSync('jq', ['-r', filter], { input: body, encoding: 'utf8' });
    if (jq.status !== 0) {
      err(jq.stderr);
      status = 1;
      return;
    }
    return process.stdout.write(jq.stdout);
  }
  if (command === 'attestation' && rest[0] === 'verify') {
    const ref = options(rest.slice(1), ['--repo', '-R', '--owner', '-o', '--signer-workflow', '--source-ref',
      '--cert-identity', '--cert-oidc-issuer', '--predicate-type', '--bundle', '--format', '--signer-repo']).positional[0];
    const digest = resolve(ref);
    if (!digest || !state.attestations.some((a) => a.digest === digest)) {
      err(`Error: no attestations found for ${ref}`);
      status = 1;
    }
    return;
  }
  return unsupported();
}

const tools = { docker, crane, skopeo, regctl, oras, cosign, gh };
if (tools[tool]) tools[tool](args);
else {
  unsupported();
  status = 7;
}
fs.writeFileSync(stateFile, JSON.stringify(state));
process.exit(status);
