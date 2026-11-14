// Reads GitHub Actions workflows and works out which jobs can run for a given event, for the workflow
// tests. A job's condition is evaluated with GitHub's expression rules in three-valued logic: what
// the event fixes (its name, ref, payload) is known; what only a run knows (step outputs, job
// results, secrets, variables, inputs of a manual run) is unknown. A job is skipped for an event
// only when that follows from what is known; otherwise it may run.
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';

// readWorkflow <path>: the workflow as an object. A pinned version: the YAML 1.2 parser keeps the
// `on` key a string.
export function readWorkflow(path) {
  const json = execFileSync('npx', ['--yes', '--package', 'yaml@2.9.1', '--', 'yaml', '--json', '--single'], {
    input: fs.readFileSync(path),
    encoding: 'utf8',
    stdio: ['pipe', 'pipe', 'inherit'],
  });
  return JSON.parse(json);
}

// readWorkflows <root>: every workflow of the repository at <root>, as an object from its path,
// .github/workflows/<file>, to the workflow.
export function readWorkflows(root) {
  const dir = `${root}/.github/workflows`;
  return Object.fromEntries(
    fs
      .readdirSync(dir)
      .filter((f) => /\.ya?ml$/.test(f))
      .map((f) => [`.github/workflows/${f}`, readWorkflow(`${dir}/${f}`)]),
  );
}

// Unknown values: `truthy` is true or false when only the truthiness is known.
const UNKNOWN = Symbol('unknown');
// An object whose missing keys are unknown rather than null.
const OPEN = Symbol('open');

export const unknown = (truthy) => ({ [UNKNOWN]: true, truthy });
export const isUnknown = (v) => v !== null && typeof v === 'object' && v[UNKNOWN] === true;
export const open = (fields) => ({ ...fields, [OPEN]: true });

// truth <value>: true, false, or undefined when unknown.
export function truth(v) {
  if (isUnknown(v)) return v.truthy;
  return !(v === null || v === undefined || v === false || v === 0 || v === '' || Number.isNaN(v));
}

function member(object, key) {
  if (isUnknown(object)) return unknown();
  if (object === null || object === undefined || typeof object !== 'object') return null;
  if (key === '*') return unknown();
  const name = Object.keys(object).find((k) => k.toLowerCase() === String(key).toLowerCase());
  if (name !== undefined) return object[name];
  return object[OPEN] ? unknown() : null;
}

function toNumber(v) {
  if (v === null) return 0;
  if (typeof v === 'boolean') return v ? 1 : 0;
  if (typeof v === 'number') return v;
  if (typeof v === 'string') return v.trim() === '' ? 0 : Number(v);
  return NaN;
}

function equal(a, b) {
  if (typeof a === 'string' && typeof b === 'string') return a.toLowerCase() === b.toLowerCase();
  if (typeof a === 'object' || typeof b === 'object') return a === b;
  if (typeof a === typeof b) return a === b;
  const x = toNumber(a);
  const y = toNumber(b);
  return !Number.isNaN(x) && x === y;
}

export function toText(v) {
  if (v === null || v === undefined) return '';
  if (typeof v === 'string') return v;
  if (typeof v === 'boolean' || typeof v === 'number') return String(v);
  return JSON.stringify(v);
}

// tokens <expression>: the expression's tokens.
function tokenize(text) {
  const pattern =
    /\s+|'((?:[^']|'')*)'|(0x[0-9a-fA-F]+|-?\d+(?:\.\d+)?(?:e[+-]?\d+)?)|(&&|\|\||==|!=|<=|>=|[<>!()[\],.*])|([A-Za-z_][A-Za-z0-9_-]*)/y;
  const tokens = [];
  let at = 0;
  while (at < text.length) {
    pattern.lastIndex = at;
    const m = pattern.exec(text);
    if (!m) throw new Error(`cannot read the expression at ${at}: ${text}`);
    at = pattern.lastIndex;
    if (m[1] !== undefined) tokens.push({ kind: 'value', value: m[1].replace(/''/g, "'") });
    else if (m[2] !== undefined) tokens.push({ kind: 'value', value: Number(m[2]) });
    else if (m[3] !== undefined) tokens.push({ kind: m[3] });
    else if (m[4] !== undefined) {
      const word = m[4];
      if (word === 'true' || word === 'false') tokens.push({ kind: 'value', value: word === 'true' });
      else if (word === 'null') tokens.push({ kind: 'value', value: null });
      else tokens.push({ kind: 'name', value: word });
    }
  }
  return tokens;
}

// parse <expression>: its syntax tree.
export function parse(text) {
  const tokens = tokenize(text);
  let at = 0;
  const peek = () => tokens[at]?.kind;
  const take = (kind) => {
    if (peek() !== kind) throw new Error(`expected ${kind} in the expression: ${text}`);
    return tokens[at++];
  };
  const binary = (next, ops) => () => {
    let left = next();
    while (ops.includes(peek())) {
      const op = tokens[at++].kind;
      left = { op, left, right: next() };
    }
    return left;
  };
  const primary = () => {
    const token = tokens[at];
    if (!token) throw new Error(`the expression ends early: ${text}`);
    if (token.kind === 'value') {
      at++;
      return { op: 'value', value: token.value };
    }
    if (token.kind === '(') {
      at++;
      const inner = or();
      take(')');
      return inner;
    }
    if (token.kind === 'name') {
      at++;
      if (peek() === '(') {
        at++;
        const args = [];
        while (peek() !== ')') {
          args.push(or());
          if (peek() === ',') at++;
          else break;
        }
        take(')');
        return { op: 'call', name: token.value.toLowerCase(), args };
      }
      return { op: 'context', name: token.value.toLowerCase() };
    }
    throw new Error(`unexpected ${token.kind} in the expression: ${text}`);
  };
  const postfix = () => {
    let node = primary();
    for (;;) {
      if (peek() === '.') {
        at++;
        if (peek() === '*') {
          at++;
          node = { op: 'member', object: node, key: { op: 'value', value: '*' } };
        } else node = { op: 'member', object: node, key: { op: 'value', value: take('name').value } };
      } else if (peek() === '[') {
        at++;
        const key = peek() === '*' ? (at++, { op: 'value', value: '*' }) : or();
        take(']');
        node = { op: 'member', object: node, key };
      } else return node;
    }
  };
  const unary = () => {
    if (peek() === '!') {
      at++;
      return { op: '!', operand: unary() };
    }
    return postfix();
  };
  const comparison = binary(unary, ['<', '<=', '>', '>=']);
  const equality = binary(comparison, ['==', '!=']);
  const and = binary(equality, ['&&']);
  const or = binary(and, ['||']);
  const tree = or();
  if (at !== tokens.length) throw new Error(`unexpected ${peek()} in the expression: ${text}`);
  return tree;
}

const functions = {
  contains: (search, item) =>
    Array.isArray(search)
      ? search.some((x) => equal(x, item))
      : toText(search).toLowerCase().includes(toText(item).toLowerCase()),
  startswith: (s, p) => toText(s).toLowerCase().startsWith(toText(p).toLowerCase()),
  endswith: (s, p) => toText(s).toLowerCase().endsWith(toText(p).toLowerCase()),
  format: (f, ...args) =>
    toText(f).replace(/\{\{|\}\}|\{(\d+)\}/g, (m, n) => (m === '{{' ? '{' : m === '}}' ? '}' : toText(args[n]))),
  join: (a, sep = ',') => (Array.isArray(a) ? a.map(toText).join(toText(sep)) : toText(a)),
  tojson: (v) => JSON.stringify(v, null, 2),
  fromjson: (s) => {
    try {
      return JSON.parse(toText(s));
    } catch {
      return unknown();
    }
  },
};

// evaluate <tree> <scope>: the value; scope.contexts maps context names to values, scope.status maps
// success, always, failure and cancelled to their values.
export function evaluate(node, scope) {
  switch (node.op) {
    case 'value':
      return node.value;
    case 'context':
      return node.name in scope.contexts ? scope.contexts[node.name] : unknown();
    case 'member':
      return member(evaluate(node.object, scope), toText(evaluate(node.key, scope)));
    case '!': {
      const v = evaluate(node.operand, scope);
      if (isUnknown(v)) return unknown(v.truthy === undefined ? undefined : !v.truthy);
      return !truth(v);
    }
    case '&&': {
      const left = evaluate(node.left, scope);
      const t = truth(left);
      if (t === false) return left;
      const right = evaluate(node.right, scope);
      if (t === true) return right;
      return unknown(truth(right) === false ? false : undefined);
    }
    case '||': {
      const left = evaluate(node.left, scope);
      const t = truth(left);
      if (t === true) return left;
      const right = evaluate(node.right, scope);
      if (t === false) return right;
      return unknown(truth(right) === true ? true : undefined);
    }
    case '==':
    case '!=':
    case '<':
    case '<=':
    case '>':
    case '>=': {
      const a = evaluate(node.left, scope);
      const b = evaluate(node.right, scope);
      if (isUnknown(a) || isUnknown(b)) return unknown();
      if (node.op === '==') return equal(a, b);
      if (node.op === '!=') return !equal(a, b);
      const x = toNumber(a);
      const y = toNumber(b);
      if (Number.isNaN(x) || Number.isNaN(y)) return false;
      return { '<': x < y, '<=': x <= y, '>': x > y, '>=': x >= y }[node.op];
    }
    case 'call': {
      if (node.name in scope.status) return scope.status[node.name];
      const fn = functions[node.name];
      const args = node.args.map((a) => evaluate(a, scope));
      if (!fn || args.some(isUnknown)) return unknown();
      return fn(...args);
    }
    default:
      throw new Error(`unknown node ${node.op}`);
  }
}

// template <text> <scope>: a value with ${{ }} interpolations, as a string, or unknown.
export function template(text, scope) {
  if (typeof text !== 'string') return text;
  const parts = text.split(/\$\{\{([\s\S]*?)\}\}/);
  if (parts.length === 1) return text;
  // A value that is one interpolation keeps the value's type, as in a condition.
  if (parts.length === 3 && parts[0].trim() === '' && parts[2].trim() === '') return evaluate(parse(parts[1]), scope);
  let known = true;
  let literal = '';
  const values = parts.map((part, i) => {
    if (i % 2 === 0) {
      literal += part;
      return part;
    }
    const v = evaluate(parse(part), scope);
    if (isUnknown(v)) known = false;
    return toText(v);
  });
  if (!known) return unknown(literal !== '' ? true : undefined);
  return values.join('');
}

// condition <if> <scope>: the value of an `if:`, with or without ${{ }}; null when there is none.
export function condition(value, scope) {
  if (value === undefined || value === null) return null;
  if (typeof value !== 'string') return value;
  const text = value.trim();
  if (text.startsWith('${{') && text.endsWith('}}') && !text.slice(3, -2).includes('${{')) {
    return evaluate(parse(text.slice(3, -2)), scope);
  }
  // Text around an interpolation makes a string, true when not empty.
  if (text.includes('${{')) return template(text, scope);
  return evaluate(parse(text), scope);
}

// usesStatus <if>: the condition calls success(), always(), failure() or cancelled().
export const usesStatus = (value) =>
  typeof value === 'string' && /\b(success|always|failure|cancelled)\s*\(/i.test(value);

// glob <pattern>: a regular expression for a GitHub filter pattern: * no slash, ** anything,
// ? and + the preceding character, [] a class.
export function glob(pattern) {
  let re = '';
  for (let i = 0; i < pattern.length; i++) {
    const c = pattern[i];
    if (c === '*') {
      if (pattern[i + 1] === '*') {
        re += '.*';
        i++;
      } else re += '[^/]*';
    } else if (c === '?' || c === '+') re += c;
    else if (c === '[') {
      const end = pattern.indexOf(']', i);
      if (end < 0) re += '\\[';
      else {
        re += pattern.slice(i, end + 1);
        i = end;
      }
    } else re += c.replace(/[.^$|(){}\\/-]/g, '\\$&');
  }
  return new RegExp(`^${re}$`);
}

// matches <patterns> <name>: the last pattern that matches decides; `!` negates.
export function matches(patterns, name) {
  let matched = false;
  for (const p of [].concat(patterns)) {
    const text = String(p);
    const negated = text.startsWith('!');
    if (glob(negated ? text.slice(1) : text).test(name)) matched = !negated;
  }
  return matched;
}

// filtered <config> <key> <name>: the name passes the filter <key> and <key>-ignore of the config.
function filtered(config, key, name) {
  if (config[key] !== undefined) return matches(config[key], name);
  if (config[`${key}-ignore`] !== undefined) return !matches(config[`${key}-ignore`], name);
  return true;
}

// triggers <workflow>: its events, as an object from each event to its config, {} when none.
export function triggers(workflow) {
  const on = workflow.on;
  if (typeof on === 'string') return { [on]: {} };
  if (Array.isArray(on)) return Object.fromEntries(on.map((e) => [e, {}]));
  return Object.fromEntries(Object.entries(on ?? {}).map(([k, v]) => [k, v ?? {}]));
}

// pathTriggers <config> <file>: a change of only <file> passes the paths filter of the config.
export const pathTriggers = (config, file) => filtered(config, 'paths', file);

// runsFor <workflow> <event>: the workflow starts for the event, by its triggers and their branch
// and tag filters; paths filters are taken to pass.
export function runsFor(workflow, event) {
  const on = triggers(workflow);
  if (!(event.name in on)) return false;
  const config = on[event.name];
  if (event.name === 'push') {
    const branches = 'branches' in config || 'branches-ignore' in config;
    const tags = 'tags' in config || 'tags-ignore' in config;
    if (event.ref.startsWith('refs/tags/')) return tags ? filtered(config, 'tags', event.refName) : !branches;
    return branches ? filtered(config, 'branches', event.refName) : !tags;
  }
  if (event.name === 'pull_request' || event.name === 'pull_request_target') {
    return filtered(config, 'branches', event.baseRef);
  }
  return true;
}

// event <name> <ref>: an event of skald-so/skald on <ref>. A PR is one from a branch named like a
// release tag, v1.2.3, into dev.
export function event(name, ref) {
  const refName = ref.replace(/^refs\/(heads|tags|pull)\//, '');
  const pr = name === 'pull_request' || name === 'pull_request_target';
  const payload = pr
    ? open({ pull_request: open({ head: open({ ref: 'v1.2.3' }), base: open({ ref: 'dev' }) }), number: 7 })
    : open({ pull_request: null, ...(name === 'push' ? { ref } : {}), ...(name === 'workflow_dispatch' ? { ref } : {}) });
  const github = open({
    event_name: name,
    event: payload,
    ref,
    ref_name: refName,
    ref_type: ref.startsWith('refs/tags/') ? 'tag' : 'branch',
    head_ref: pr ? 'v1.2.3' : '',
    base_ref: pr ? 'dev' : '',
    repository: 'skald-so/skald',
    repository_owner: 'skald-so',
  });
  // The inputs of a manual run are whatever the person running it chooses.
  const inputs = name === 'workflow_dispatch' || name === 'workflow_call' ? unknown() : {};
  return { name, ref, refName, baseRef: 'dev', github, inputs };
}

export const needsOf = (job) => [].concat(job?.needs ?? []);

// allNeeds <jobs> <key>: the keys of the jobs the job needs, directly or through other jobs, as a Set.
export function allNeeds(jobs, key, seen = new Set()) {
  for (const n of needsOf(jobs[key])) {
    if (!seen.has(n)) {
      seen.add(n);
      allNeeds(jobs, n, seen);
    }
  }
  return seen;
}

// matrixTargets <jobs> <keys>: the targets the matrices of the jobs include, sorted, each once.
export const matrixTargets = (jobs, keys) =>
  [...new Set(keys.flatMap((k) => [].concat(jobs[k].strategy?.matrix?.include ?? []).map((i) => i?.target)))].filter(Boolean).sort();

// The events of skald-so/skald that publish no release, for event(): pushes of no release tag, a
// PR, and manual runs, on a release tag's ref too.
export const unpublishedEvents = [
  ['push', 'refs/heads/dev'],
  ['push', 'refs/heads/main'],
  ['push', 'refs/heads/v1.2.3'],
  ['push', 'refs/tags/1.2.3'],
  ['push', 'refs/tags/release-1.2.3'],
  ['push', 'refs/tags/skald-v1.2.3'],
  ['pull_request', 'refs/pull/7/merge'],
  ['workflow_dispatch', 'refs/heads/dev'],
  ['workflow_dispatch', 'refs/tags/v1.2.3'],
  ['workflow_dispatch', 'refs/tags/v1.2.3-beta.1'],
];

// jobsFor <workflow> <event>: for each job key, {skipped}: true when the job is skipped for the event
// whatever happens in the run, false when it may run.
export function jobsFor(workflow, ev) {
  const jobs = workflow.jobs ?? {};
  const states = {};
  const starts = runsFor(workflow, ev);
  const visit = (key, path = []) => {
    if (states[key]) return states[key];
    if (path.includes(key)) throw new Error(`the jobs need each other: ${[...path, key].join(' → ')}`);
    const job = jobs[key];
    const direct = needsOf(job);
    direct.forEach((n) => visit(n, [...path, key]));
    const needs = {};
    for (const n of direct) {
      const skipped = states[n].skipped;
      const outputs = {};
      for (const [name, value] of Object.entries(jobs[n]?.outputs ?? {})) {
        outputs[name] = skipped ? '' : states[n].output(value);
      }
      needs[n] = { result: skipped ? 'skipped' : unknown(), outputs };
    }
    const needSkipped = direct.some((n) => states[n].skipped);
    const scope = {
      contexts: { github: ev.github, inputs: ev.inputs, needs },
      status: {
        success: direct.length === 0 ? true : needSkipped ? false : unknown(),
        always: true,
        failure: direct.length === 0 ? false : unknown(),
        cancelled: unknown(),
      },
    };
    // A condition without a status function runs only after its needs succeed, as success() && it.
    const skipped =
      !starts ||
      (!usesStatus(job.if) && truth(scope.status.success) === false) ||
      (job.if !== undefined && truth(condition(job.if, scope)) === false);
    states[key] = {
      skipped,
      scope,
      // The value of an output of this job, a string; outputs of its steps are unknown.
      output: (value) => {
        const v = template(value, scope);
        return isUnknown(v) ? v : toText(v);
      },
    };
    return states[key];
  };
  Object.keys(jobs).forEach((k) => visit(k));
  return Object.fromEntries(Object.entries(states).map(([k, s]) => [k, { skipped: s.skipped, scope: s.scope }]));
}

// stepSkipped <step> <job state>: the step is skipped whatever happens in the run.
export function stepSkipped(step, state) {
  if (state.skipped) return true;
  if (step.if === undefined) return false;
  const scope = { ...state.scope, status: { success: unknown(), always: true, failure: unknown(), cancelled: unknown() } };
  return truth(condition(step.if, scope)) === false;
}

// The events of a release workflow's dry run, for event(): a PR and a manual run.
export const dryRunEvents = [
  ['pull_request', 'refs/pull/7/merge'],
  ['workflow_dispatch', 'refs/heads/dev'],
];

// jobsRunning <workflow> <event> <line>: the keys of the jobs that may run, for the event, a line of
// a step that <line> is true for, in a step that may run, and whose failure fails the run: neither
// the job nor the step continues on error.
export function jobsRunning(workflow, ev, line) {
  const states = jobsFor(workflow, ev);
  return Object.entries(workflow.jobs ?? {})
    .filter(([key, job]) => !states[key].skipped && !job['continue-on-error'])
    .filter(([key, job]) =>
      (job.steps ?? []).some((step) => !step['continue-on-error'] && !stepSkipped(step, states[key]) && commands(step.run).some(line)),
    )
    .map(([key]) => key);
}

// permissions <workflow> <job>: the job's token permissions as {scope: level}, or null when neither
// the job nor the workflow sets them.
export function permissions(workflow, job) {
  const p = job.permissions ?? workflow.permissions;
  if (p === undefined || p === null) return null;
  if (p === 'read-all') return { all: 'read' };
  if (p === 'write-all') return { all: 'write' };
  if (typeof p !== 'object') return { all: String(p) };
  return p;
}

// writes <permissions>: the scopes the permissions write.
export const writes = (p) => (p ? Object.entries(p).filter(([, level]) => level === 'write').map(([scope]) => scope) : []);

// commands <run>: the lines of a run script, with continued lines joined.
export const commands = (run) => String(run ?? '').replace(/\\\r?\n/g, ' ').replace(/`\r?\n/g, ' ').split(/\r?\n/);

// words <text>: shell words, quotes kept.
export const words = (text) => text.match(/"(?:[^"\\]|\\.)*"|'[^']*'|[^\s"']+(?:"(?:[^"\\]|\\.)*"|'[^']*'|[^\s"']+)*/g) ?? [];
