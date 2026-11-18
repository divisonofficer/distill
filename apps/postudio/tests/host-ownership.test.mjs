import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';

const source = (path) => readFile(new URL(`../src/routes/${path}/+page.svelte`, import.meta.url), 'utf8');

test('session metadata comes from pinned host when board follows another host', async () => {
  const page = await source('s/[id]');
  const conversation = await readFile(new URL('../src/lib/components/SessionConversation.svelte', import.meta.url), 'utf8');
  assert.match(page, /sessionOrigin\(page\.url\.search\)/);
  assert.match(page, /getJson<[^;]+>\('\/api\/sessions', host\)/);
  assert.match(page, /host=\{destination\.host\}/);
  assert.match(conversation, /getJson<[^;]+>\('\/api\/sessions', sessionHost\)/);
  assert.match(conversation, /pinnedRow = result\.sessions\.find\(\(item\) => item\.sessionId === id\)/);
  assert.match(conversation, /metadataGeneration/);
  assert.match(conversation, /if \(generation !== metadataGeneration\) return;/);
});

test('sidebar workspace context stays pinned when the board follows another host', async () => {
  const sidebar = await readFile(new URL('../src/lib/components/Sidebar.svelte', import.meta.url), 'utf8');
  const summary = await readFile(new URL('../src/lib/components/AccountSummary.svelte', import.meta.url), 'utf8');
  assert.match(sidebar, /sessionOrigin\(page\.url\.search\) \?\? page\.url\.origin/);
  assert.match(summary, /sessionOrigin\(page\.url\.search\) \?\? page\.url\.origin/);
  assert.match(sidebar, /getJson<[^;]+>\('\/api\/sessions', host\)/);
  assert.match(sidebar, /if \(!current\) return;/);
  assert.match(sidebar, /pinnedRow && pinnedRow\.sessionId === page\.params\.id && pinnedRow\.apiOrigin === activeHost/);
  assert.match(sidebar, /current = false; clearInterval\(timer\)/);
});

test('new task keeps selected host and blocks silent host migration', async () => {
  const page = await source('new');
  assert.match(page, /const origin = ownerHost \|\| currentHost;/);
  assert.match(page, /if \(origin !== currentHost\) \{[^}]+return;/);
  assert.match(page, /postJson<[^;]+>\('\/api\/sessions', body, origin \|\| undefined\)/);
  assert.match(page, /host=\{ownerHost \|\| undefined\} onselect=\{\(\) => \{ touched = true; \}\}/);
});
