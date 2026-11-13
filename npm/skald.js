#!/usr/bin/env node
// The launcher of the npm package skald-so: `npx skald-so <args>` runs skald of the package's
// release with <args> and exits with its exit status. The first run downloads the release's archive
// for this machine's OS and architecture from its GitHub Release, checks it against its checksum and
// unpacks it into this package's directory, with the file installed-by-npm beside skald, so
// `skald update` leaves that copy to npm. On a mismatch or a missing checksum it stops and runs
// nothing. script/release-npm sets the release's version; script/npm-package-test tests it.
// SKALD_DOWNLOAD_URL: where the release's files are instead of its GitHub Release. The downloads go
// through the proxy HTTPS_PROXY names, HTTP_PROXY for http, except to the hosts NO_PROXY names.
'use strict';

const { spawn, spawnSync } = require('node:child_process');
const crypto = require('node:crypto');
const fs = require('node:fs');
const http = require('node:http');
const path = require('node:path');

const { version } = require('./package.json');

const fail = (message) => {
  process.stderr.write(`skald-so: ${message}\n`);
  process.exit(1);
};

if (version === '0.0.0') fail('this is the source of the package; run the package of a release: npx skald-so');
// Node.js's own proxy support for fetch, set from the environment: Node.js 24.14+ or 25.4+. An
// older Node.js runs the launcher only with no proxy set.
if (typeof http.setGlobalProxyFromEnv === 'function') {
  http.setGlobalProxyFromEnv();
} else if (['HTTPS_PROXY', 'https_proxy', 'HTTP_PROXY', 'http_proxy'].some((name) => process.env[name])) {
  fail(`a proxy needs Node.js 24.14 or later, or 25.4 or later, not ${process.version}`);
}
const tag = `v${version}`;
const windows = process.platform === 'win32';
const targets = {
  'darwin arm64': 'aarch64-apple-darwin',
  'darwin x64': 'x86_64-apple-darwin',
  'linux arm64': 'aarch64-unknown-linux-gnu',
  'linux x64': 'x86_64-unknown-linux-gnu',
  'win32 arm64': 'aarch64-pc-windows-msvc',
  'win32 x64': 'x86_64-pc-windows-msvc',
};
const target = targets[`${process.platform} ${process.arch}`];
if (!target) fail(`no skald for ${process.platform} on ${process.arch}`);
const archive = `skald-client-${target}.${windows ? 'zip' : 'tar.gz'}`;
const base = (process.env.SKALD_DOWNLOAD_URL || `https://github.com/skald-so/skald/releases/download/${tag}`).replace(/\/+$/, '');
const marker = 'installed-by-npm';
// The unpacked archive, skald at its root: the zip has skald.exe at its root, the tar.gz has its
// files in skald-client-<target>/.
const installed = path.join(__dirname, 'release', target);
const skald = path.join(installed, windows ? 'skald.exe' : 'skald');

// download <url>: the file's bytes; over HTTPS only, unless SKALD_DOWNLOAD_URL names another scheme.
async function download(url) {
  const response = await fetch(url);
  if (base.startsWith('https://') && !response.url.startsWith('https://')) throw new Error(`redirected to ${response.url}`);
  if (!response.ok) throw new Error(`HTTP ${response.status}`);
  return Buffer.from(await response.arrayBuffer());
}

// install: downloads, checks and unpacks the archive into a directory of its own, then moves it to
// `installed` at once, so a run sees all of it or none of it.
async function install() {
  const releaseDir = path.dirname(installed);
  fs.mkdirSync(releaseDir, { recursive: true });
  const stage = fs.mkdtempSync(path.join(releaseDir, '.unpack-'));
  try {
    const bytes = await download(`${base}/${archive}`).catch((err) => {
      throw new Error(`cannot download ${base}/${archive}: ${err.message}`);
    });
    // The archive's checksum, <archive>.sha256 of the release as dist writes it: `<hex> *<name>`.
    const sums = await download(`${base}/${archive}.sha256`).catch((err) => {
      throw new Error(`cannot download the checksum of ${archive}, ${base}/${archive}.sha256: ${err.message}: nothing run`);
    });
    const expected = sums.toString('utf8').trim().split(/\s+/)[0];
    if (!/^[0-9a-f]{64}$/i.test(expected)) throw new Error(`the checksum of ${archive}, ${archive}.sha256, has no SHA-256: nothing run`);
    if (crypto.createHash('sha256').update(bytes).digest('hex') !== expected.toLowerCase()) {
      throw new Error(`${archive} does not match its checksum, ${archive}.sha256: nothing run`);
    }
    const file = path.join(stage, archive);
    fs.writeFileSync(file, bytes);
    const unpacked = path.join(stage, 'archive');
    fs.mkdirSync(unpacked);
    // Windows' own tar unpacks a zip; Git's, which may come first on the PATH, does not.
    const tar = windows ? path.join(process.env.SystemRoot || 'C:\\Windows', 'System32', 'tar.exe') : 'tar';
    const result = spawnSync(tar, [windows ? '-xf' : '-xzf', file, '-C', unpacked], { stdio: ['ignore', 'ignore', 'pipe'] });
    if (result.status !== 0) throw new Error(`cannot unpack ${archive}: ${result.error?.message ?? String(result.stderr).trim()}`);
    const root = windows ? unpacked : path.join(unpacked, `skald-client-${target}`);
    if (!fs.existsSync(path.join(root, path.basename(skald)))) throw new Error(`${archive} has no ${path.basename(skald)}`);
    fs.writeFileSync(path.join(root, marker), 'Installed by the npm package skald-so: update it through npm.\n');
    try {
      fs.renameSync(root, installed);
    } catch (err) {
      // Another run moved its copy there first.
      if (!fs.existsSync(path.join(installed, marker))) throw new Error(`cannot install skald in ${installed}: ${err.message}`);
    }
  } finally {
    fs.rmSync(stage, { recursive: true, force: true });
  }
}

async function main() {
  if (!fs.existsSync(path.join(installed, marker))) await install();
  const child = spawn(skald, process.argv.slice(2), { stdio: 'inherit' });
  // Ctrl-C and Ctrl-\ reach skald from the terminal: this process waits for it to exit.
  for (const signal of windows ? ['SIGINT'] : ['SIGINT', 'SIGQUIT']) process.on(signal, () => {});
  if (!windows) for (const signal of ['SIGTERM', 'SIGHUP']) process.on(signal, () => child.kill(signal));
  child.on('error', (err) => fail(`cannot run ${skald}: ${err.message}`));
  child.on('exit', (code, signal) => {
    if (signal) {
      process.removeAllListeners(signal);
      process.kill(process.pid, signal);
    } else {
      process.exit(code);
    }
  });
}

main().catch((err) => fail(err.message));
