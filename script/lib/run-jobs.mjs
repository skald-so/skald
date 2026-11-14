// Runs jobs of a workflow as GitHub runs them, for the tests that run the release workflow's jobs:
// script/release-attest-test, script/release-image-test, script/release-npm-workflow-test. Each
// job's and step's `if`, `env` and `${{ }}` are evaluated as GitHub does, for each combination of a
// job's matrix; `run` steps run in bash with GITHUB_OUTPUT, GITHUB_ENV, GITHUB_PATH and
// GITHUB_STEP_SUMMARY; what an action does is the caller's.
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { condition, event, isUnknown, needsOf, open, template, toText, truth, unknown, usesStatus } from './workflows.mjs';

// combinations <matrix>: the matrix's combinations, [{}] without one.
export function combinations(matrix) {
  if (!matrix || typeof matrix !== 'object') return [{}];
  const keys = Object.keys(matrix).filter((k) => k !== 'include' && k !== 'exclude');
  let combos = [{}];
  for (const k of keys) combos = combos.flatMap((c) => [].concat(matrix[k] ?? []).map((v) => ({ ...c, [k]: v })));
  const include = [].concat(matrix.include ?? []);
  if (keys.length === 0 && include.length > 0) return include;
  return [...combos, ...include];
}

// git <dir> <argument>...: git in <dir> with none of the user's config; its output.
const git = (cwd, ...args) => {
  const r = spawnSync('git', ['-c', 'commit.gpgsign=false', '-c', 'tag.gpgsign=false', ...args], {
    cwd,
    encoding: 'utf8',
    env: {
      PATH: process.env.PATH,
      HOME: cwd,
      GIT_CONFIG_GLOBAL: '/dev/null',
      GIT_CONFIG_NOSYSTEM: '1',
      GIT_AUTHOR_NAME: 'test',
      GIT_AUTHOR_EMAIL: 'test@example.com',
      GIT_COMMITTER_NAME: 'test',
      GIT_COMMITTER_EMAIL: 'test@example.com',
    },
  });
  if (r.status !== 0) throw new Error(`git ${args.join(' ')}: ${r.stderr}`);
  return r.stdout.trim();
};

// commitAll <dir> [<tag>]: a new repository in <dir> with one commit of all its files, tagged <tag>
// if given; returns the commit.
export function commitAll(dir, tag) {
  git(dir, 'init', '-q');
  git(dir, 'add', '-A');
  git(dir, 'commit', '-q', '-m', 'release');
  if (tag) git(dir, 'tag', tag);
  return git(dir, 'rev-parse', 'HEAD');
}

// readOutputs <file>: the name=value and name<<delimiter lines of a GITHUB_OUTPUT or GITHUB_ENV file.
export const readOutputs = (file) => {
  const out = {};
  const text = fs.existsSync(file) ? fs.readFileSync(file, 'utf8').split('\n') : [];
  for (let i = 0; i < text.length; i++) {
    const heredoc = text[i].match(/^([^=<]+)<<(.+)$/);
    if (heredoc) {
      const value = [];
      for (i++; i < text.length && text[i] !== heredoc[2]; i++) value.push(text[i]);
      out[heredoc[1]] = value.join('\n');
    } else {
      const m = text[i].match(/^([^=]+)=(.*)$/);
      if (m) out[m[1]] = m[2];
    }
  }
  return out;
};

// runJobs <workflow> <keys> <options>: runs the jobs <keys> of <workflow>, in that order, as GitHub
// runs them on the event options.eventName on the ref options.ref of skald-so/skald, with the
// workspace at options.version:
//   options.work    the checkout, at the commit options.sha;
//   options.bin     a directory first on PATH, of the stand-ins;
//   options.runner  RUNNER_TEMP, also for the step files;
//   options.env     variables of every step, over those GitHub sets;
//   options.action  ({step, kind, index, key, where, input, env}) => {outputs, passed, problem}:
//                   what an action step does; <kind> is its action without the version, <input>
//                   gives each of its `with` values; it passes unless <passed> is false, and
//                   <problem>, if any, is one of the run's problems.
// A job it needs that is not among <keys> succeeds, its outputs that name a tag set to the ref's tag,
// or v<version> on another ref, those that name a version to <version>, the others unknown. Returns
// {results, problems}: each job's {result, outputs, outcomes}, <outcomes> each step's outcome by its
// index, and the steps that failed or could not run.
export function runJobs(workflow, keys, options) {
  const { eventName, ref, version, sha, work, bin, runner } = options;
  const jobs = workflow.jobs ?? {};
  const ev = event(eventName, ref);
  const tag = ev.refName.startsWith('v') ? ev.refName : `v${version}`;
  const problems = [];
  const github = open({
    ...ev.github,
    sha,
    workspace: work,
    token: 'stub-token',
    actor: 'skald-release[bot]',
    server_url: 'https://github.com',
    run_id: '1',
    run_number: '1',
    run_attempt: '1',
  });
  const baseEnv = {
    PATH: `${bin}${path.delimiter}${process.env.PATH}`,
    HOME: process.env.HOME,
    TMPDIR: process.env.TMPDIR ?? os.tmpdir(),
    CI: 'true',
    GITHUB_ACTIONS: 'true',
    GITHUB_WORKSPACE: work,
    GITHUB_EVENT_NAME: eventName,
    GITHUB_REF: ref,
    GITHUB_REF_NAME: ev.refName,
    GITHUB_REF_TYPE: ref.startsWith('refs/tags/') ? 'tag' : 'branch',
    GITHUB_REPOSITORY: 'skald-so/skald',
    GITHUB_REPOSITORY_OWNER: 'skald-so',
    GITHUB_SHA: sha,
    GITHUB_ACTOR: 'skald-release[bot]',
    GITHUB_SERVER_URL: 'https://github.com',
    GITHUB_RUN_ID: '1',
    RUNNER_TEMP: runner,
    RUNNER_OS: 'Linux',
    RUNNER_ARCH: 'X64',
    ...options.env,
  };
  const value = (text, contexts, status) => {
    const v = template(text, { contexts, status });
    return isUnknown(v) ? '' : toText(v);
  };
  const results = {};
  let counter = 0;
  for (const key of keys) {
    const job = jobs[key];
    const needs = {};
    for (const n of needsOf(job)) {
      if (results[n]) needs[n] = results[n];
      else {
        const outputs = {};
        for (const name of Object.keys(jobs[n]?.outputs ?? {})) {
          outputs[name] = /tag/i.test(name) ? tag : /version/i.test(name) ? version : unknown();
        }
        needs[n] = { result: 'success', outputs: open(outputs) };
      }
    }
    const needResults = Object.values(needs).map((n) => n.result);
    const jobStatus = {
      success: needResults.every((r) => r === 'success'),
      failure: needResults.includes('failure'),
      always: true,
      cancelled: false,
    };
    const jobContexts = { github, needs, inputs: eventName === 'workflow_dispatch' ? open({}) : {}, vars: open({}) };
    let runs = usesStatus(job.if) ? true : jobStatus.success;
    if (job.if !== undefined && truth(condition(job.if, { contexts: jobContexts, status: jobStatus })) === false) runs = false;
    if (!runs) {
      results[key] = { result: 'skipped', outputs: {}, outcomes: {} };
      continue;
    }
    let ok = true;
    let jobOutputs = {};
    const outcomes = {};
    for (const matrix of combinations(job.strategy?.matrix)) {
      const steps = {};
      const env = { ...baseEnv, GITHUB_JOB: key };
      let failed = false;
      for (const [index, step] of (job.steps ?? []).entries()) {
        const status = { success: !failed, failure: failed, always: true, cancelled: false };
        const contexts = {
          ...jobContexts,
          env: open({}),
          steps: open(steps),
          matrix: open(matrix),
          runner: open({ temp: runner, os: 'Linux', arch: 'X64' }),
          secrets: open({ GITHUB_TOKEN: 'stub-token' }),
          job: open({}),
          strategy: open({}),
        };
        let go = usesStatus(step.if) ? true : !failed;
        if (step.if !== undefined && truth(condition(step.if, { contexts, status })) === false) go = false;
        if (!go) {
          outcomes[index] = 'skipped';
          if (step.id) steps[step.id] = { outputs: {}, outcome: 'skipped', conclusion: 'skipped' };
          continue;
        }
        const stepEnv = { ...env };
        for (const source of [workflow.env, job.env, step.env]) {
          for (const [k, v] of Object.entries(source ?? {})) stepEnv[k] = value(v, { ...contexts, env: open({ ...stepEnv }) }, status);
        }
        const stepContexts = { ...contexts, env: open({ ...stepEnv }) };
        const where = `job ${key} step ${step.name ?? step.id ?? index + 1}`;
        let outputs = {};
        let passed = true;
        if (step.run !== undefined) {
          const shell = step.shell ?? job.defaults?.run?.shell ?? workflow.defaults?.run?.shell ?? 'bash';
          if (!/^bash\b/.test(shell)) {
            problems.push(`${where}: the test runs steps in bash only, not ${shell}`);
            passed = false;
          } else {
            const n = counter++;
            const files = Object.fromEntries(['output', 'env', 'path', 'summary'].map((f) => [f, path.join(runner, `${f}-${n}`)]));
            Object.values(files).forEach((f) => fs.writeFileSync(f, ''));
            const script = path.join(runner, `step-${n}.sh`);
            fs.writeFileSync(script, value(step.run, stepContexts, status));
            const dir = step['working-directory'] ?? job.defaults?.run?.['working-directory'] ?? workflow.defaults?.run?.['working-directory'] ?? '.';
            const r = spawnSync('bash', ['--noprofile', '--norc', '-eo', 'pipefail', script], {
              cwd: path.resolve(work, value(dir, stepContexts, status)),
              env: { ...stepEnv, GITHUB_OUTPUT: files.output, GITHUB_ENV: files.env, GITHUB_PATH: files.path, GITHUB_STEP_SUMMARY: files.summary },
              encoding: 'utf8',
              timeout: 300000,
            });
            if (r.status !== 0) {
              problems.push(`${where} exits ${r.status}: ${(r.stderr || r.stdout || String(r.error ?? '')).trim().split('\n').slice(-3).join(' | ')}`);
              passed = false;
            }
            outputs = readOutputs(files.output);
            Object.assign(env, readOutputs(files.env));
            // Added directories go after the stand-ins, before the rest of PATH.
            const added = fs.readFileSync(files.path, 'utf8').split('\n').filter(Boolean);
            if (added.length > 0) env.PATH = [bin, ...added.reverse(), env.PATH.slice(bin.length + 1)].join(path.delimiter);
          }
        } else {
          const input = (name) => value(step.with?.[name], stepContexts, status);
          const kind = String(step.uses ?? '').split('@')[0].toLowerCase();
          const done = options.action({ step, kind, index, key, where, input, env: stepEnv }) ?? {};
          if (done.problem) problems.push(`${where}: ${done.problem}`);
          if (done.passed === false) passed = false;
          outputs = done.outputs ?? {};
        }
        outcomes[index] = passed ? 'success' : 'failure';
        if (step.id) steps[step.id] = { outputs: open(outputs), outcome: passed ? 'success' : 'failure', conclusion: passed ? 'success' : 'failure' };
        const ignored = step['continue-on-error'];
        if (!passed && (ignored === undefined || ignored === null || ignored === false || ignored === '' || String(ignored).toLowerCase() === 'false')) {
          failed = true;
        }
      }
      if (failed) ok = false;
      const contexts = { ...jobContexts, steps: open(steps), matrix: open(matrix), env: open({}) };
      const status = { success: !failed, failure: failed, always: true, cancelled: false };
      jobOutputs = Object.fromEntries(Object.entries(job.outputs ?? {}).map(([name, v]) => [name, value(v, contexts, status)]));
    }
    results[key] = { result: ok ? 'success' : 'failure', outputs: open(jobOutputs), outcomes };
  }
  return { results, problems };
}
