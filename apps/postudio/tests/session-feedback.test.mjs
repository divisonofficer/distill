import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const source = readFileSync(new URL('../src/lib/components/SessionConversation.svelte', import.meta.url), 'utf8');

test('session feedback only confirms matching pending model and mode acknowledgements', () => {
  assert.match(source, /const pending = modelPending;\s*if \(pending\?\.id === id && m\.model_id === pending\.requested\) ui\.notify/);
  assert.match(source, /if \(pending\?\.id === id && modeFromRow\(row\) === pending\.mode\) \{\s*ui\.notify/);
  assert.match(source, /else if \(m\.t === 'error' && modelPending\?\.id === id\)[\s\S]*?ui\.notify\([^\n]+, 'error'\)/);
  assert.match(source, /modePending = \{ id, mode: next \};[\s\S]*?modeTimer = setTimeout/);
  assert.match(source, /if \(modeRequest === request[\s\S]*?ui\.notify\([^\n]+, 'error'\)/);
  assert.match(source, /if \(modelRequest === request[\s\S]*?ui\.notify\([^\n]+, 'error'\)/);
  assert.match(source, /function cancelCurrent\(\)[\s\S]*?if \(!view\.connected \|\| !view\.running\) return;[\s\S]*?send\(\{ t: 'cancel' \}\)/);
  assert.match(source, /cancel: \(\) => view\.connected && view\.running && !hostError && send\(\{ t: 'cancel' \}\)/);
  assert.doesNotMatch(source, /ui\.notify\([^\n]*m\.message/);
});
