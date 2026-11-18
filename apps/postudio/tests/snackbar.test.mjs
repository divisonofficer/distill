import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import ts from 'typescript';

const source = await readFile(new URL('../src/lib/ui.svelte.ts', import.meta.url), 'utf8');
const js = ts.transpileModule(source.replace(/\$state(?:\.raw)?(?:<[^>]+>)?\(/g, '(').replace(/\$state<[^\n]+>\(/g, '('), { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.None } }).outputText;
const match = js.match(/class Ui \{[\s\S]*?\n\}/);
assert.ok(match, 'compiled UI class exists');
const Ui = new Function(`${match[0]}; return Ui`)();
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
