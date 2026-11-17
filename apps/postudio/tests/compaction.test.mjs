import assert from 'node:assert/strict';
import { after, before, test } from 'node:test';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createServer } from 'vite';

let vite, SessionView, SessionReplay, Choice, Status, render;
before(async () => {
 const root = fileURLToPath(new URL('..', import.meta.url));
 vite = await createServer({ root, configFile: resolve(root, 'vite.config.ts'), server: { middlewareMode: true }, appType: 'custom', logLevel: 'silent' });
 ({ SessionView, SessionReplay } = await vite.ssrLoadModule('/src/lib/session.svelte.ts'));
 ({ default: Choice } = await vite.ssrLoadModule('/src/lib/components/CompactionChoice.svelte'));
 ({ default: Status } = await vite.ssrLoadModule('/src/lib/components/CompactionStatus.svelte'));
 ({ render } = await vite.ssrLoadModule('svelte/server'));
});
after(async () => { await vite?.close(); });
const start = { t: 'compaction.started', tokens_used: 143000, context_window: 200000, reason: 'cold return' };
const done = { t: 'compaction.completed', tokens_before: 143000, tokens_after: 56000, elapsed_ms: 1200 };

test('compaction tracks only real lifecycle results and resets with session', () => {
 const view = new SessionView();
 assert.equal(view.compaction, null);
 view.apply(start); assert.equal(view.compaction.status, 'running');
 view.apply(done); assert.equal(view.compaction.tokensAfter, 56000);
 view.apply({ t: 'turn.end', stop_reason: 'end_turn' }); assert.equal(view.compaction.status, 'completed');
 view.apply({ t: 'compaction.failed', error: 'actual failure' }); assert.equal(view.compaction.error, 'actual failure');
 view.apply({ t: 'compaction.cancelled', reason: 'user_cancelled' }); assert.equal(view.compaction.status, 'cancelled');
 view.reset(); assert.equal(view.compaction, null);
});
test('disconnect, turn end and errors never leave stale running indicator', () => {
 for (const event of [{ t: 'turn.end', stop_reason: 'end_turn' }, { t: 'error', message: 'connection error' }, { t: 'session.state', status: 'error' }]) {
  const view = new SessionView(); view.apply(start); view.apply(event); assert.equal(view.compaction.status, 'unknown');
 }
 const view = new SessionView(); view.apply(start); view.setConnected(false); assert.equal(view.compaction.status, 'unknown');
 view.setConnected(true); assert.equal(view.compaction.status, 'unknown');
});
test('initial and older history cannot resurrect historical running compaction or replace live result', () => {
 const view = new SessionView();
 view.applyHistoryPage([start], true); assert.equal(view.compaction, null);
 view.apply(start); view.applyHistoryPage([done], true); assert.equal(view.compaction.status, 'running');
 view.apply(done); view.applyHistoryPage([start], false); assert.equal(view.compaction.status, 'completed');
 const replay = new SessionReplay(view); replay.begin(); view.apply(start);
 replay.apply({ t: 'history.page', initial: true, items: [done], cursor: null, has_more: false });
 assert.equal(view.compaction.status, 'running');
 replay.apply(done); assert.equal(view.compaction.status, 'completed');
});
test('initial history restores terminal compaction without reviving an unfinished start', () => {
 const view = new SessionView();
 view.applyHistoryPage([start, done], true);
 assert.equal(view.compaction.status, 'completed');
 assert.equal(view.compaction.tokensAfter, 56000);
 const later = new SessionView();
 later.applyHistoryPage([start, done, start], true);
 assert.equal(later.compaction, null);
 for (const event of [{ t: 'compaction.failed', error: 'failure' }, { t: 'compaction.cancelled', reason: 'cancelled' }]) {
  const restored = new SessionView(); restored.applyHistoryPage([start, event], true);
  assert.equal(restored.compaction.status, event.t.slice('compaction.'.length));
 }
});
test('dedicated selector renders Korean choices from server options without invented savings', () => {
 const options = ['Compact and continue', 'Keep full history', "Don't ask again"].map(label => ({ label, description: label }));
 const body = render(Choice, { props: { request: { t: 'question.request', id: 'q', session_id: 's', tool_call_id: 'cold-return-test', mode: 'default', questions: [{ question: 'Cache expired', options, multiSelect: false }] }, connected: true, reply: () => true } }).body;
 assert.match(body, /먼저 대화를 압축할까요/); assert.match(body, /전체 기록 그대로 보내기/);
 assert.match(body, /다시 묻지 않기/); assert.match(body, /직접 답변 입력/);
 assert.equal((body.match(/role="radio"/g) ?? []).length, 4);
 assert.doesNotMatch(body, /56k|39%|질문에 답해주세요/);
});
test('status shows actual totals and errors, no fabricated progress percentage', () => {
 const view = new SessionView(); view.apply(start);
 const running = render(Status, { props: { state: view.compaction } }).body;
 assert.match(running, /대화 압축 중/); assert.match(running, /143,000/); assert.doesNotMatch(running, /\d+%/);
 view.apply(done); const completed = render(Status, { props: { state: view.compaction } }).body;
 assert.match(completed, /대화 압축 완료/); assert.match(completed, /56,000/);
 view.apply({ t: 'compaction.failed', error: 'fixture_failure' });
 assert.match(render(Status, { props: { state: view.compaction } }).body, /fixture_failure/);
});
