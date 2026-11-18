import { after, before, test } from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
const require = createRequire(new URL('../package.json', import.meta.url));
const { createServer } = require('vite');
const { svelte } = require('@sveltejs/vite-plugin-svelte');

let vite;
let Executions, childExecutionState, childNeedsInput, matchChildExecution;
before(async () => {
 vite = await createServer({ root: new URL('..', import.meta.url).pathname, configFile: false, plugins: [svelte()], server: { middlewareMode: true }, appType: 'custom' });
 ({ Executions, childExecutionState, childNeedsInput, matchChildExecution } = await vite.ssrLoadModule('/src/lib/executions.svelte.ts'));
});
after(async () => { await vite?.close(); });

const agent = (status = 'running', durationMs = 3) => ({ subagentId: 'a', parentSessionId: 's', childSessionId: 'c', subagentType: 'worker', description: 'work', startedAtEpochMs: 10, durationMs, status });
const task = () => ({ taskId: 't', command: 'echo ok', cwd: '/', startedAtUnixMs: 1, endedAtUnixMs: null, output: '', outputFile: '', truncated: false, exitCode: null, signal: null, completed: false, kind: 'bash', outputTotalBytes: 0, canStop: true });
const deferred = () => { let resolve; const promise = new Promise((r) => { resolve = r; }); return { promise, resolve }; };
const json = (body) => Response.json(body);

async function withNetwork(handler, run) {
 const original = globalThis.fetch;
 globalThis.fetch = (url, init = {}) => handler(new URL(String(url), 'http://localhost').pathname, init);
 try { await run(); } finally { globalThis.fetch = original; }
}

test('child UI state maps only actual registry states and blocker evidence', () => {
 assert.equal(childExecutionState('running', true), 'running');
 assert.equal(childExecutionState('initializing', true), 'running');
 assert.equal(childExecutionState('running', true, true), 'blocked');
 assert.equal(childExecutionState('completed', false), 'done');
 assert.equal(childExecutionState('failed', false), 'failed');
 assert.equal(childExecutionState('cancelled', false), 'stopped');
 assert.equal(childExecutionState('running', false), 'unknown');
 assert.equal(childExecutionState(undefined, true), 'unknown');
 const child = agent('running');
 assert.equal(childNeedsInput(child, [{ sessionId: 'c', parentSessionId: 's', activity: 'needs_input' }], 's'), true);
 assert.equal(childNeedsInput(child, [{ sessionId: 'c', parentSessionId: 's', activity: 'working' }], 's'), false);
 assert.equal(childNeedsInput(child, [{ sessionId: 'c', parentSessionId: 'other', activity: 'needs_input' }], 's'), false);
 assert.equal(childNeedsInput(child, [{ sessionId: 'other-child', parentSessionId: 's', activity: 'needs_input' }], 's'), false);
});

test('child execution requires exact stable tool-call linkage', () => {
 const call = { kind: 'tool', key: 1, id: 'call-1', name: 'spawn_subagent', status: 'completed', input: { subagent_type: 'worker', description: 'Review parser' } };
 const candidate = { ...agent('running'), toolCallId: call.id };
 assert.equal(matchChildExecution(call, [candidate], 's'), candidate);
 assert.equal(matchChildExecution(call, [agent('running')], 's'), null);
 assert.equal(matchChildExecution(call, [candidate, { ...candidate, subagentId: 'b' }], 's'), null);
 assert.equal(matchChildExecution(call, [candidate], 'other-session'), null);
 assert.equal(matchChildExecution({ ...call, name: 'read_file' }, [candidate], 's'), null);
});

test('terminal snapshot freezes; missing snapshot errors make agent stale; task failures independent', async () => {
 let phase = 0;
 await withNetwork(async (path) => {
  if (path.endsWith('/subagents')) return json({ subagents: phase === 0 ? [agent()] : [] });
  if (path.endsWith('/subagents/a')) return phase === 1 ? new Response('offline', { status: 502 }) : json({ snapshot: agent('completed', 42) });
  if (path.endsWith('/tasks')) return phase === 1 ? new Response('unsupported', { status: 502 }) : json({ tasks: [task()] });
  throw Error(path);
 }, async () => {
  const state = new Executions('http://localhost', 's');
  await state.refresh(); assert.equal(state.agentFresh.a, true); assert.equal(state.tasksFresh, true);
  phase = 1; await state.refresh();
  assert.equal(state.agents[0].status, 'running'); assert.equal(state.agentFresh.a, undefined);
  assert.match(state.agentSnapshotErrors.a, /502/); assert.match(state.taskError, /unsupported/);
  assert.equal(await state.stopAgent('a'), null); assert.equal(await state.stopTask('t'), null);
  phase = 2; await state.refresh(); assert.equal(state.agents[0].durationMs, 42); assert.equal(state.agents[0].status, 'completed');
  phase = 0; await state.refresh(); assert.equal(state.agents[0].durationMs, 42); assert.equal(state.agents[0].status, 'completed');
  assert.equal(await state.stopAgent('a'), null); state.stop();
 });
});

test('pending, foreign ID and actual ACK outcomes', async () => {
 const pending = deferred(); let posts = 0; let ack = pending.promise;
 await withNetwork(async (path, init) => {
  if (init.method === 'POST') { posts++; return await ack; }
  return path.endsWith('/subagents') ? json({ subagents: [agent()] }) : json({ tasks: [task()] });
 }, async () => {
  const state = new Executions('http://localhost', 's'); await state.refresh();
  assert.equal(await state.stopAgent('foreign'), null); assert.equal(await state.stopTask('foreign'), null);
  const first = state.stopAgent('a'); assert.equal(await state.stopAgent('a'), null); assert.equal(posts, 1);
  pending.resolve(json({ outcome: { kind: 'already_finished', status: 'completed' }, cancelled: false }));
  assert.equal(await first, 'already_finished'); assert.equal(state.agentStopErrors.a, 'already_finished');
  for (const outcome of ['cancelled', 'not_found']) { ack = Promise.resolve(json({ outcome: { kind: outcome } })); assert.equal(await state.stopAgent('a'), outcome); }
  for (const outcome of ['killed', 'already_exited', 'not_found']) { ack = Promise.resolve(json({ outcome })); assert.equal(await state.stopTask('t'), outcome); }
  ack = Promise.resolve(json({ outcome: 'invented' })); assert.equal(await state.stopTask('t'), null); assert.match(state.taskStopErrors.t, /Invalid stop acknowledgment/);
  assert.equal(state.tasks.length, 1); state.stop();
 });
});

test('late poll and stop acknowledgments cannot write after cleanup or unlock new generation', async () => {
 const pendingPost = deferred(), oldPoll = deferred(), newPoll = deferred(); let phase = 0, taskPolls = 0;
 await withNetwork(async (path, init) => {
  if (init.method === 'POST') return pendingPost.promise;
  if (path.endsWith('/tasks')) { taskPolls++; if (phase === 1) return oldPoll.promise; if (phase === 2) return newPoll.promise; return json({ tasks: [task()] }); }
  return json({ subagents: [agent()] });
 }, async () => {
  const state = new Executions('http://localhost', 's'); await state.refresh();
  const post = state.stopTask('t'); phase = 1; const old = state.refresh();
  state.stop(); phase = 2; const newer = state.refresh();
  oldPoll.resolve(json({ tasks: [] })); await old;
  const concurrent = state.refresh(); assert.equal(taskPolls, 3);
  pendingPost.resolve(json({ outcome: 'killed' })); assert.equal(await post, null); assert.deepEqual(state.pendingTaskStops, []);
  newPoll.resolve(json({ tasks: [task()] })); await Promise.all([newer, concurrent]); assert.equal(state.tasks.length, 1); state.stop();
 });
});

test('malformed and unsupported registry responses are errors, not empty success', async () => {
 let mode = 0;
 await withNetwork(async (path) => {
  if (mode === 0) return path.endsWith('/tasks') ? json({ tasks: [task()] }) : json({ subagents: [agent()] });
  if (mode === 1) return path.endsWith('/tasks') ? new Response('unsupported', { status: 502 }) : json({ subagents: [{ ...agent(), parentSessionId: 'foreign' }] });
  return path.endsWith('/tasks') ? json({ tasks: {} }) : json({ subagents: [{ ...agent(), status: 'mystery' }] });
 }, async () => {
  const state = new Executions('http://localhost', 's'); await state.refresh();
  for (mode = 1; mode <= 2; mode++) { await state.refresh(); assert.ok(state.agentError); assert.ok(state.taskError); assert.equal(state.agents.length, 1); assert.equal(state.tasks.length, 1); assert.equal(await state.stopAgent('a'), null); assert.equal(await state.stopTask('t'), null); }
  state.stop();
 });
});


test('task stop requires scoped capability and validates optional fields', async () => {
 let row = { ...task(), canStop: false }; let posts = 0;
 await withNetwork(async (path, init) => {
  if (init.method === 'POST') { posts++; return json({ outcome: 'killed' }); }
  return path.endsWith('/subagents') ? json({ subagents: [] }) : json({ tasks: [row] });
 }, async () => {
  const state = new Executions('http://localhost', 's');
  await state.refresh(); assert.equal(await state.stopTask('t'), null);
  const { canStop, ...withoutCapability } = task(); row = withoutCapability;
  await state.refresh(); assert.equal(await state.stopTask('t'), null); assert.equal(posts, 0);
  for (row of [{ ...task(), canStop: 'true' }, { ...task(), description: 3 }]) {
   await state.refresh(); assert.match(state.taskError, /Invalid task registry/);
   assert.equal(await state.stopTask('t'), null); assert.equal(posts, 0);
  }
  row = { ...task(), description: 'real label' }; await state.refresh();
  assert.equal(state.tasks[0].description, 'real label');
  assert.equal(await state.stopTask('t'), 'killed'); assert.equal(posts, 1);
  state.stop();
 });
});


test('empty 404 pauses only unsupported resource and retry recovers', async () => {
 let unavailable = true; let agents = 0; let tasks = 0;
 await withNetwork(async path => {
  if (path.endsWith('/subagents')) { agents++; return unavailable ? new Response(null, { status: 404 }) : json({ subagents: [] }); }
  tasks++; return json({ tasks: [task()] });
 }, async () => {
  const state = new Executions('http://localhost', 's');
  await state.refresh(); await state.refresh();
  assert.equal(state.agentUnavailable, true); assert.equal(state.taskUnavailable, false);
  assert.equal(agents, 1); assert.equal(tasks, 2); assert.equal(state.tasksFresh, true);
  unavailable = false; await state.retry();
  assert.equal(state.agentUnavailable, false); assert.equal(state.agentError, null); assert.equal(agents, 2);
  state.stop();
 });
});

test('session, permission, network and malformed errors remain errors', async () => {
 for (const response of [() => Response.json({ error: 'session not found' }, { status: 404 }), () => new Response(null, { status: 401 }), () => new Response(null, { status: 403 }), () => new Response(null, { status: 500 }), () => { throw Error('offline'); }, () => new Response('bad json')]) {
  await withNetwork(response, async () => {
   const state = new Executions('http://localhost', 's'); await state.refresh();
   assert.equal(state.agentUnavailable, false); assert.equal(state.taskUnavailable, false);
   assert.ok(state.agentError); assert.ok(state.taskError); state.stop();
  });
 }
});

test('API errors retain status path detail and message for GET and POST', async () => {
 const { getJson, postJson, ApiError } = await vite.ssrLoadModule('/src/lib/api.ts');
 await withNetwork(() => Response.json({ error: 'session not found' }, { status: 404 }), async () => {
  for (const request of [() => getJson('/api/test'), () => postJson('/api/test', {})]) {
   await assert.rejects(request, error => error instanceof ApiError && error.status === 404 && error.path === '/api/test' && error.detail === 'session not found' && error.message === '/api/test: 404 session not found');
  }
 });
});

test('fresh client restores completed child history from the real list contract', async () => {
 await withNetwork(async (path) => path.endsWith('/tasks') ? json({ tasks: [] }) : json({ subagents: [{ ...agent(), status: 'completed', output: 'done' }] }), async () => {
  const state = new Executions('http://localhost', 's');
  await state.refresh();
  assert.equal(state.agentError, null);
  assert.equal(state.agents[0].status, 'completed');
  assert.equal(state.agentFresh.a, true);
  state.stop();
 });
});
