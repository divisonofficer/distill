import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const sidebar = await readFile(new URL('../src/lib/components/Sidebar.svelte', import.meta.url), 'utf8');
const summary = await readFile(new URL('../src/lib/components/AccountSummary.svelte', import.meta.url), 'utf8');

test('mascot opens read-only account summary; gear alone navigates settings', () => {
  assert.match(sidebar, /<AccountSummary\s*\/>/);
  assert.match(sidebar, /href="\/settings"[^>]*aria-label="설정"/);
  assert.doesNotMatch(sidebar, /href="\/settings#accounts-h"/);
  assert.match(summary, /getJson<unknown>\('\/api\/accounts', target\)/);
  assert.doesNotMatch(summary, /postJson|\/api\/accounts\/(?:add|logout|rename|remove|usage)/);
});

test('summary distinguishes quota states and validates actual remaining percentage', () => {
  assert.match(summary, /quota\?\.status === 'unsupported'/);
  assert.match(summary, /quota\?\.status === 'unavailable'/);
  assert.match(summary, /Number\.isFinite\(account\.quota\.remaining_percent\)/);
  assert.match(summary, /잔여량 \{Math\.round\(account\.quota\.remaining_percent\)\}%/);
});

test('summary popup uses manual top-layer popover with bounded fixed placement', () => {
  assert.match(summary, /popover="manual"/);
  assert.match(summary, /showPopover\(\)/);
  assert.match(summary, /panel\?\.showPopover\(\);\s*positionPanel\(\);/);
  assert.match(summary, /const onResize = \(\) => close\(\);/);
  assert.match(summary, /panel\.style\.bottom/);
  assert.match(summary, /Math\.min\(320, Math\.max\(0, window\.innerWidth - 24\)\)/);
  assert.match(summary, /maxHeight/);
  assert.doesNotMatch(summary, /class="absolute bottom-full/);
  assert.doesNotMatch(summary, /size-8 shrink-0 rounded-full/);
});

test('summary pins session host and discards stale requests', () => {
  assert.match(summary, /sessionOrigin\(page\.url\.search\)/);
  assert.match(summary, /current !== request \|\| target !== host/);
  assert.match(summary, /event\.key === 'Escape'/);
  assert.match(summary, /event\.composedPath\(\)\.includes\(root\)/);
});
