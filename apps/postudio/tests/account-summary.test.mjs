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

test('summary colors real usage at 70/90 thresholds and puts highest connected quota in footer', () => {
  const block = summary.slice(summary.indexOf('const highestUsage = $derived.by(() => {'), summary.indexOf('function usageColor'));
  assert.match(block, /account.connected && account.quota\?\.status === 'available'/);
  assert.match(block, /Number.isFinite\(account.quota.remaining_percent\)/);
  assert.match(block, /remaining_percent >= 0 && account.quota.remaining_percent <= 100/);
  assert.match(block, /const used = 100 - account.quota!.remaining_percent!/);
  assert.match(block, /used > highest.used/);
  assert.match(summary, /used >= 90 \? '#c8392c' : used >= 70 \? '#d6a24a' : '#2b59c3'/);
  assert.match(summary, /사용량 기간·초기화 시각 정보 없음/);
  assert.doesNotMatch(summary, /주간 사용량|월요일|Plus|Max|로그아웃/);
  assert.match(summary, /href="\/settings#accounts-h"/);
});

test('summary pins session host and discards stale requests', () => {
  assert.match(summary, /sessionOrigin\(page\.url\.search\)/);
  assert.match(summary, /current !== request \|\| target !== host/);
  assert.match(summary, /event\.key === 'Escape'/);
  assert.match(summary, /event\.composedPath\(\)\.includes\(root\)/);
});
