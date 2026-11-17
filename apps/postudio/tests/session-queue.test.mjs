import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

globalThis.$state = (value) => value;
const { SessionView } = await import('../src/lib/session.svelte.ts');
const conversation = await readFile(new URL('../src/lib/components/SessionConversation.svelte', import.meta.url), 'utf8');
const types = await readFile(new URL('../src/lib/types.ts', import.meta.url), 'utf8');

test('queue snapshot accepts versioned prompts and stale completion cannot show idle', () => {
  const view = new SessionView();
  view.apply({
    t: 'queue.changed',
    entries: [{ id: 'queued-2', version: 4, owner: 'postudio-node', kind: 'prompt', text: 'next', position: 0 }],
    running_prompt_id: 'running-1'
  });
  assert.equal(view.running, true);
  assert.equal(view.activePromptId, 'running-1');
  assert.equal(view.queue[0].version, 4);
  view.apply({ t: 'turn.end', prompt_id: 'older-turn', stop_reason: 'end_turn' });
  assert.equal(view.running, true);
  view.reset(true);
  view.setConnected(false);
  view.setConnected(true);
  assert.equal(view.queue[0].id, 'queued-2');
  assert.equal(view.running, true);
  view.apply({ t: 'prompt.accepted', prompt_id: 'steering-1' });
  assert.equal(view.running, true);
  view.apply({ t: 'turn.end', prompt_id: 'running-1', stop_reason: 'end_turn' });
  assert.equal(view.running, false);
  view.apply({ t: 'queue.changed', entries: [{ id: 'old-session', version: 0, owner: 'postudio-node', kind: 'prompt', text: 'old', position: 0 }] });
  view.reset();
  assert.deepEqual(view.queue, []);
  assert.equal(view.activePromptId, null);
  view.apply({ t: 'queue.changed', entries: [], running_prompt_id: 'disconnected-turn' });
  view.apply({ t: 'queue.changed', entries: [], snapshot: true });
  assert.equal(view.activePromptId, null);
  assert.equal(view.running, false);
});

test('composer stays editable during work and sends acknowledged queue protocol', () => {
  assert.match(conversation, /placeholder=\{!view\.connected \?[^\n]+실행 중이에요\. 메시지를 보내면 다음 단계 전에 전달해요/);
  assert.match(conversation, /if \(e\.metaKey \|\| e\.ctrlKey\)[\s\S]*?submitPrompt\(true\)/);
  assert.match(conversation, /prompt_id: promptId/);
  assert.match(conversation, /const acceptedId = pendingPromptId \?\? deliveryUnknownPromptId;[\s\S]*m\.running_prompt_id === acceptedId/);
  assert.match(conversation, /m\.snapshot && pendingPromptId[\s\S]*?deliveryUnknownPromptId = pendingPromptId/);
  assert.match(conversation, /deliveryUnknownPromptId && !window\.confirm\([\s\S]*?중복될 수 있습니다/);
  assert.match(conversation, /m\.t === 'prompt\.accepted'[\s\S]*?acknowledgePrompt\(m\.prompt_id\)/);
  const rejected = conversation.match(/else if \(m\.t === 'prompt\.rejected'[\s\S]*?\n\}\nconst wasRunning/)?.[0] ?? '';
  assert.ok(rejected);
  assert.doesNotMatch(rejected, /text = ''|attachments = \[\]/);
  assert.match(conversation, /t: 'queue\.remove', id: entry\.id, expected_version: entry\.version/);
  assert.ok(/m\.t === 'replay\.end'[\s\S]*?requestQueueSnapshot\(\)/.test(conversation));
  assert.ok(/function requestQueueSnapshot\(\)[\s\S]*?postudio-queue-snapshot-\$\{crypto\.randomUUID\(\)\}/.test(conversation));
  assert.match(conversation, /title=\{view\.running && !hasDraft \? "실행 중지"/);
  assert.match(conversation, /aria-label=\{`실행 현황 열기[^\n]+onclick=\{\(\) => onOpenMonitor\?\.\(\)\}/);
  assert.match(conversation, /@container \(max-width: 560px\)/);
  assert.match(conversation, /prefers-reduced-motion: reduce/);
  assert.doesNotMatch(conversation, /sending && m\.t === 'session\.state'[\s\S]{0,180}attachments = \[\]/);
  const textarea = conversation.match(/<textarea[\s\S]*?<\/textarea>/)?.[0] ?? '';
  assert.ok(textarea);
  assert.doesNotMatch(textarea, /disabled=/);
  assert.match(types, /t: 'queue\.remove'; id: string; expected_version: number/);
  assert.match(types, /t: 'prompt';[^\n]+prompt_id: string; send_now\?: boolean/);
});

 test('composer reflects fresh background executions without stopping them', () => {
  assert.match(conversation, /executions \? \(executions\.tasksFresh \? executions\.tasks\.filter\(\(task\) => !task\.completed\)\.length : 0\) : tools\.filter/);
  assert.match(conversation, /const busy = \$derived\(view\.running \|\| runningAgentCount > 0 \|\| runningCommandCount > 0\)/);
  assert.match(conversation, /class:composer-running=\{busy\}/);
  assert.match(conversation, /\{#if busy\}<span[^\n]+서브에이전트 \$\{runningAgentCount\}개 실행 중[^\n]+명령 \$\{runningCommandCount\}개 실행 중/);
  assert.match(conversation, /<button type=\{view\.running && !hasDraft \? "button" : "submit"\}[^\n]+cancelCurrent\(\)/);
  assert.match(conversation, /navigator\.platform[^\n]+⌘Enter[^\n]+Ctrl\+Enter/);
  assert.match(conversation, /다음 단계 전에 전달해요 · \{shortcut\}로 지금 끼어들기/);
  assert.match(conversation, /\.composer-running, \.composer-running:focus-within \{ border-color: #b9cbf3/);
  assert.match(conversation, /gap: 6px; padding: 0 9px; border-radius: 14px; background: #e8eefc/);
  assert.match(conversation, /@container \(max-width: 560px\) \{ \.run-detail \{ display: none/);
});
