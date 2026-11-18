import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';

const component = readFileSync(new URL('../src/lib/components/HistorySkeleton.svelte', import.meta.url), 'utf8');
const chatRoute = readFileSync(new URL('../src/routes/c/[id]/+page.svelte', import.meta.url), 'utf8');

test('history skeleton renders reference-shaped chat and work variants accessibly', () => {
  assert.match(component, /variant = 'chat'/);
  assert.match(component, /variant === 'chat'/);
  assert.match(component, /role="status" aria-label=\{statusLabel\}/);
  assert.match(component, /기록을 불러오는 중입니다/);
  assert.match(component, /prefers-reduced-motion: reduce/);
  const chat = component.split('{:else}')[0];
  const work = component.split('{:else}')[1].split('{\/if}')[0];
  assert.equal((chat.match(/class="skeleton-line/g) ?? []).length, 7);
  assert.equal((work.match(/class="skeleton-line/g) ?? []).length, 2);
  assert.equal((work.match(/class="skeleton-row/g) ?? []).length, 3);
  assert.match(chat, /w-\[44%\]/);
  assert.match(chat, /w-\[30%\]/);
  assert.match(chat, /w-\[94%\]/);
  assert.match(chat, /h-\[76px\] w-full/);
  assert.match(work, /w-\[52%\]/);
  assert.match(work, /w-\[92%\]/);
});

test('chat history skeleton replaces only initial empty-history loading state', () => {
  assert.match(chatRoute, /\{#if loading && !history\}[\s\S]*?<HistorySkeleton variant="chat" \/>/);
  assert.match(chatRoute, /\{#if historyError\}/);
});
