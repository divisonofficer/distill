import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

const routePath = fileURLToPath(new URL('../src/routes/c/+page.svelte', import.meta.url));

test('chat list search trims query, ignores case, uses preview and renamed titles, and shows all for empty query', async () => {
const source = await readFile(routePath, 'utf8');
const expression = source.match(/const filteredChats = \$derived\((chats\.filter\(.*\))\);/)?.[1];
assert.ok(expression, 'route must define filtered chat expression');
const filterChats = new Function('chats', 'search', 'chatDisplayTitle', '$chatPreviewTitles', '$chatTitleOverrides', `return ${expression};`);
const chats = [{ chatId: 'one', title: 'Named chat' }, { chatId: 'two', title: '' }];
const titles = { two: 'Useful preview title' };
const titleFor = (chat, _messages, previewTitles, overrides = {}) => overrides[chat.chatId] || chat.title || previewTitles[chat.chatId] || '새 채팅';
const filter = (query) => filterChats(chats, query, titleFor, titles, {});

assert.deepEqual(filter('  nAmEd  ').map(({ chatId }) => chatId), ['one']);
assert.deepEqual(filter(' PREVIEW ').map(({ chatId }) => chatId), ['two']);
assert.deepEqual(filterChats(chats, 'renamed', titleFor, titles, { two: 'Renamed chat' }).map(({ chatId }) => chatId), ['two']);
assert.deepEqual(filter('no such chat'), []);
assert.deepEqual(filter('   ').map(({ chatId }) => chatId), ['one', 'two']);
});
