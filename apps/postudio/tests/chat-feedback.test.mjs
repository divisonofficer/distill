import assert from 'node:assert/strict';
import { test } from 'node:test';
import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));

test('chat feedback follows acknowledged title and message saves only', async () => {
const source = await readFile(resolve(root, 'src/routes/c/[id]/+page.svelte'), 'utf8');
const title = source.slice(source.indexOf('async function saveChatTitle()'), source.indexOf('async function loadOlderHistory()'));
const save = source.slice(source.indexOf('async function saveMessage('), source.indexOf('async function sendMessage('));
assert.match(title, /parseCreatedChat\([\s\S]*?setChatTitleOverride[\s\S]*?ui\.notify\('채팅 제목을 저장했습니다\.'/);
assert.match(title, /if \(activeChatId === id\) \{[\s\S]*?ui\.notify\('채팅 제목을 저장하지 못했습니다\.', 'error'\)/);
assert.match(save, /savedTextFilesMatch\(saved\.textFiles, originalFiles\)[\s\S]*?ui\.notify\('서버가 첨부 파일을 보존하지 않아 저장하지 않았습니다\.', 'error'\)/);
assert.match(save, /appendMessage\(id, saved\);\s*if \(id === activeChatId && !generate\) ui\.notify/);
assert.doesNotMatch(source.slice(0, source.indexOf('async function saveChatTitle()')), /ui\.notify/);
assert.match(save, /if \(id === activeChatId\) \{\s*saveError = `메시지를 저장하지 못했습니다\.[\s\S]*?ui\.notify\('메시지를 저장하지 못했습니다\.', 'error'\)/);
});
