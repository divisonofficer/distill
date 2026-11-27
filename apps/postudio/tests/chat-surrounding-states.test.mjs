import assert from 'node:assert/strict';
import { after, before, test } from 'node:test';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { resolve } from 'node:path';
import { createServer } from 'vite';
import { compile } from 'svelte/compiler';
const root = fileURLToPath(new URL('..', import.meta.url));
let vite, attachments, page;
before(async () => {
 vite = await createServer({ root, configFile: resolve(root, 'vite.config.ts'), server: { middlewareMode: true }, appType: 'custom', logLevel: 'silent' });
 attachments = await vite.ssrLoadModule('/src/lib/chat-attachments.ts');
 page = await readFile(resolve(root, 'src/routes/c/[id]/+page.svelte'), 'utf8');
});
after(async () => vite?.close());
const file = (name, text = 'hello', size = new TextEncoder().encode(text).byteLength) => ({ name, size, text: async () => text });

test('general chat accepts only TXT/MD and reports each rejected file without reading binaries', async () => {
 const result = await attachments.readChatTextFiles([file('paper.pdf'), file('image.png'), file('notes.MD'), file('data.csv')], []);
 assert.deepEqual(result.files, [{ name: 'notes.MD', text: 'hello' }]);
 for (const name of ['paper.pdf', 'image.png', 'data.csv']) assert.ok(result.error.includes(name));
 assert.match(result.error, /TXT\/MD/);
});
test('general chat enforces actual UTF-8 aggregate bytes and count without mutating drafts', async () => {
 const existing = [{ name: 'original.txt', text: '가'.repeat(349525) }];
 const result = await attachments.readChatTextFiles([file('overflow.txt', '나', 1)], existing);
 assert.deepEqual(result.files, existing);
 assert.match(result.error, /1 MiB/);
 const ten = Array.from({ length: 10 }, (_, index) => ({ name: `${index}.txt`, text: '' }));
 assert.equal((await attachments.readChatTextFiles([file('eleven.md')], ten)).files.length, 10);
 assert.equal(existing.length, 1);
});
test('empty files survive and unreadable/binary text is rejected explicitly', async () => {
 const broken = { name: 'broken.txt', size: 1, text: async () => { throw Error('disk read failed'); } };
 const result = await attachments.readChatTextFiles([file('empty.txt', ''), file('binary.md', '\0'), file('invalid.txt', '\uFFFD'), broken], []);
 assert.deepEqual(result.files, [{ name: 'empty.txt', text: '' }]);
 assert.match(result.error, /UTF-8/);
 assert.match(result.error, /disk read failed/);
});
test('chat components compile and preserve independent citation and tool activity behavior', async () => {
 for (const path of ['src/routes/c/[id]/+page.svelte', 'src/routes/c/+page.svelte', 'src/lib/components/AttachmentPreview.svelte']) {
  const source = await readFile(resolve(root, path), 'utf8');
  assert.doesNotThrow(() => compile(source, { filename: resolve(root, path), generate: 'server' }));
 }
 assert.match(page, /jumpToCitation/);
 assert.match(page, /ChatToolActivity/);
});
test('connection retry is read-only, bounded and never replays completion or save requests', () => {
 const retry = page.slice(page.indexOf('function historyConnectionFailed()'), page.indexOf('function handleHistoryScroll()'));
 assert.match(retry, /connectionFailures >= 5/);
 assert.match(retry, /await refreshHistory\(activeChatId, false\)/);
 assert.doesNotMatch(retry, /postJson|requestCompletion|saveMessage|requestSubmit/);
 assert.match(page, /async function saveMessage\(generate: boolean\) \{\s*if \(readOnlyConnection\)/);
 assert.match(page, /if \(fileError\) \{ saveError/);
 assert.match(page, /자동 전송하지 않아요/);
 assert.match(page, /onoffline=\{onOffline\} ononline=\{onOnline\}/);
});
test('file drop and paste converge on supported text validation and history only follows the bottom', () => {
 assert.match(page, /ondrop=\{handleDrop\}/);
 assert.match(page, /onpaste=\{handlePaste\}/);
 assert.match(page, /readChatTextFiles\(files, getChatTextFiles\(id\)\)/);
 assert.match(page, /scrollTop <= 160/);
 assert.match(page, /scrollTop < 300 && hasOlderMessages/);
 assert.match(page, /beforeTop \+ \(messageScroller.scrollHeight - beforeHeight\)/);
 assert.match(page, /새 답변 ·/);
});
