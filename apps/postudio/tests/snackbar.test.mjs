import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import ts from 'typescript';

const source = await readFile(new URL('../src/lib/ui.svelte.ts', import.meta.url), 'utf8');
const js = ts.transpileModule(source.replace(/\$state(?:\.raw)?(?:<[^>]+>)?\(/g, '(').replace(/\$state<[^\n]+>\(/g, '('), { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.None } }).outputText;
const match = js.match(/class Ui \{[\s\S]*?\n\}/);
assert.ok(match, 'compiled UI class exists');
const Ui = new Function(`${match[0]}; return Ui`)();

function withClock(run) {
  const original = { setTimeout: globalThis.setTimeout, clearTimeout: globalThis.clearTimeout, now: Date.now };
  const timers = [];
  let now = 0;
  globalThis.setTimeout = (callback, delay) => { timers.push({ callback, delay, cleared: false }); return timers.length; };
  globalThis.clearTimeout = (id) => { if (timers[id - 1]) timers[id - 1].cleared = true; };
  Date.now = () => now;
  try { run(new Ui(), timers, (value) => { now = value; }); }
  finally { globalThis.setTimeout = original.setTimeout; globalThis.clearTimeout = original.clearTimeout; Date.now = original.now; }
}

test('success lasts 2.6 seconds and undo/action lasts 6 seconds; errors and loading persist', () => {
  withClock((ui, timers) => {
    ui.notify('copied'); assert.equal(timers.at(-1).delay, 2600);
    ui.notify('archived', 'undo', { label: 'Undo', callback() {} }); assert.equal(timers.at(-1).delay, 6000);
    ui.notify('action', 'success', { label: 'Undo', callback() {} }); assert.equal(timers.at(-1).delay, 6000);
    const count = timers.length;
    ui.notify('error', 'error'); assert.equal(timers.length, count);
    ui.notify('working', 'loading'); assert.equal(timers.length, count);
    assert.equal(ui.snackbar.message, 'working');
  });
});

test('hover/focus pause preserves remaining time and ignores pre-pause callbacks', () => {
  withClock((ui, timers, advance) => {
    ui.notify('undo', 'undo'); advance(2000); ui.pauseSnackbar();
    assert.equal(timers[0].cleared, true);
    timers[0].callback(); assert.equal(ui.snackbar.message, 'undo');
    advance(9000); ui.resumeSnackbar();
    assert.equal(timers.at(-1).delay, 4000);
    timers[0].callback(); assert.equal(ui.snackbar.message, 'undo');
    timers.at(-1).callback(); assert.equal(ui.snackbar, null);
  });
});

test('progress completion is guarded by notification identity', () => {
  withClock((ui) => {
    const id = ui.notify('working', 'loading');
    ui.updateSnackbar(id, 'complete'); assert.equal(ui.snackbar.message, 'complete');
    const old = ui.notify('old', 'loading'); ui.notify('new');
    ui.updateSnackbar(old, 'late completion'); assert.equal(ui.snackbar.message, 'new');
  });
});

test('action clears old notification before invoking callback so its result survives', () => {
  withClock((ui) => {
    ui.notify('retry', 'error', { label: 'Retry', callback: () => ui.notify('retried') });
    ui.runSnackbarAction(); assert.equal(ui.snackbar.message, 'retried');
  });
});
test('dismiss clears toast and stale timer cannot clear newer toast', async () => {
  const originalSetTimeout = globalThis.setTimeout;
  const originalClearTimeout = globalThis.clearTimeout;
  const callbacks = [];
  globalThis.setTimeout = (callback) => { callbacks.push(callback); return callbacks.length; };
  globalThis.clearTimeout = () => {};
  try {
    const ui = new Ui();
    ui.notify('old');
    ui.dismissSnackbar();
    ui.notify('new');
    callbacks[0]();
    assert.equal(ui.snackbar.message, 'new');
    ui.dismissSnackbar();
  } finally {
    globalThis.setTimeout = originalSetTimeout;
    globalThis.clearTimeout = originalClearTimeout;
  }
});
