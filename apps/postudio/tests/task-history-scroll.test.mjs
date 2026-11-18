import assert from 'node:assert/strict';
import { after, before, test } from 'node:test';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createServer } from 'vite';

const root = fileURLToPath(new URL('..', import.meta.url));
let vite;
let TaskHistoryScroll;
let isNearBottom;

before(async () => {
vite = await createServer({
root,
configFile: resolve(root, 'vite.config.ts'),
server: { middlewareMode: true },
appType: 'custom',
logLevel: 'silent'
});
({ TaskHistoryScroll, isNearBottom } = await vite.ssrLoadModule('/src/lib/task-history-scroll.ts'));
});

after(async () => {
await vite?.close();
});

test('initial replay and route change target latest history', () => {
const history = new TaskHistoryScroll();
const tallHistory = { scrollTop: 0, scrollHeight: 1600, clientHeight: 600 };

const firstLoad = history.begin('session-a');
assert.equal(firstLoad, 'initial');
assert.deepEqual(history.complete(firstLoad, tallHistory), { kind: 'bottom' });
assert.equal(history.shouldFollow(), true);

assert.equal(history.begin('session-a'), 'reconnect');
assert.equal(history.begin('session-b'), 'initial');
assert.deepEqual(history.complete('initial', tallHistory), { kind: 'bottom' });
});

test('reconnect preserves older reading and follows only within bottom threshold', () => {
const history = new TaskHistoryScroll();
const bottom = { scrollTop: 600, scrollHeight: 1000, clientHeight: 400 };
history.complete(history.begin('session-a'), bottom);
history.update({ scrollTop: 120, scrollHeight: 1000, clientHeight: 400 });
assert.equal(history.shouldFollow(), false);

const reconnect = history.begin('session-a');
assert.equal(reconnect, 'reconnect');
assert.deepEqual(history.complete(reconnect, { scrollTop: 120, scrollHeight: 1000, clientHeight: 400 }), {
kind: 'position',
top: 120
});
assert.equal(history.shouldFollow(), false);
assert.equal(isNearBottom({ scrollTop: 504, scrollHeight: 1000, clientHeight: 400 }), true);
assert.equal(isNearBottom({ scrollTop: 503, scrollHeight: 1000, clientHeight: 400 }), false);
});
