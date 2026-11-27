import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import ts from 'typescript';

const card = await readFile(new URL('../src/lib/components/ApprovalCard.svelte', import.meta.url), 'utf8');
const plan = await readFile(new URL('../src/lib/components/InteractionCard.svelte', import.meta.url), 'utf8');
const source = card.slice(card.indexOf('function decide('), card.indexOf('</script>'));
const js = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText;
function harness({ connected = true, result = true } = {}) {
 const once = { id: 'server-allow-id', kind: 'allow_once' };
 const reject = { id: 'server-reject-id', kind: 'reject_once' };
 const messages = [];
 const create = new Function('connected', 'reply', 'request', 'actions', `let sent = false, error = '', menu; ${js}; return { decide, shortcut, get sent() { return sent }, get error() { return error } };`);
 const controller = create(connected, message => { messages.push(message); return result; }, { id: 42 }, { once, reject });
 return { controller, messages, once, reject };
}
function keyboard(key, child = false) {
 const target = {};
 return { key, target, currentTarget: child ? {} : target, preventDefault() { this.prevented = true; }, stopPropagation() {} };
}

test('approval uses the exact offered option id and suppresses duplicate submissions', () => {
 const { controller, messages, once, reject } = harness();
 controller.decide(once); controller.decide(reject);
 assert.deepEqual(messages, [{ t: 'permission', id: 42, option_id: 'server-allow-id' }]);
 assert.equal(controller.sent, true);
 assert.match(card, /서버 확인 대기/);
 assert.doesNotMatch(card, /허용했어요|거부했어요/);
});

test('disconnected or failed sends never report approval or lock out retry', () => {
 const offline = harness({ connected: false }); offline.controller.decide(offline.once);
 assert.equal(offline.messages.length, 0); assert.equal(offline.controller.sent, false);
 const failed = harness({ result: false }); failed.controller.decide(failed.once); failed.controller.decide(failed.once);
 assert.equal(failed.messages.length, 2); assert.equal(failed.controller.sent, false);
 assert.match(failed.controller.error, /보내지 못했어요/);
});

test('Enter/Escape only decide a focused card, never a composer or nested control', () => {
 for (const key of ['Enter', 'Escape']) {
  const { controller, messages } = harness();
  controller.shortcut(keyboard(key, true)); assert.equal(messages.length, 0);
  const modified = keyboard(key); modified.ctrlKey = true;
  controller.shortcut(modified); assert.equal(messages.length, 0);
  const repeat = keyboard(key); repeat.repeat = true;
  controller.shortcut(repeat); assert.equal(messages.length, 0);
  const event = keyboard(key); controller.shortcut(event);
  assert.equal(messages[0].option_id, key === 'Enter' ? 'server-allow-id' : 'server-reject-id');
  assert.equal(event.prevented, true);
 }
});

test('plan revision focuses feedback and keeps protocol outcomes without fabricated execution state', () => {
 assert.match(plan, /feedbackInput\?\.focus\(\)/);
 assert.match(plan, /!connected \|\| submitted/);
 assert.match(plan, /outcome === 'cancelled' && !feedback.trim\(\)/);
 assert.match(plan, /if \(reply\(msg\)\)/);
 assert.match(plan, /planReply\('approved'\)/);
 assert.match(plan, /planReply\('abandoned'\)/);
 assert.doesNotMatch(plan, /진행 중|완료했어요/);
});
