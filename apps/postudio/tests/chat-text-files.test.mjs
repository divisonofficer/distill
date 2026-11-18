import assert from 'node:assert/strict';
import { after, before, test } from 'node:test';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createServer } from 'vite';

const root = fileURLToPath(new URL('..', import.meta.url));
let vite, chats;
before(async () => {
  vite = await createServer({ root, configFile: resolve(root, 'vite.config.ts'), server: { middlewareMode: true }, appType: 'custom', logLevel: 'silent' });
  chats = await vite.ssrLoadModule('/src/lib/chats.ts');
});
after(async () => vite?.close());
const message = { messageId: 'm', parentId: null, role: 'user', text: 'hi', model: null, author: 'me', seq: 1 };

test('history accepts legacy and bounded textFiles fields', () => {
  assert.deepEqual(chats.parseChatHistory({ chatId: 'c', title: 'c', messages: [message] }).messages[0].textFiles, []);
  assert.deepEqual(chats.parseChatHistory({ chatId: 'c', title: 'c', messages: [{ ...message, textFiles: [{ name: 'a.md', text: 'ok' }] }] }).messages[0].textFiles, [{ name: 'a.md', text: 'ok' }]);
  assert.throws(() => chats.parseChatHistory({ chatId: 'c', title: 'c', messages: [{ ...message, textFiles: [{ name: 'x.png', text: '' }] }] }));
  assert.throws(() => chats.parseChatHistory({ chatId: 'c', title: 'c', messages: [{ ...message, textFiles: [{ name: 'x.txt', text: 'x'.repeat(1048577) }] }] }));
});

test('attachment capability omission disables feature and validates exact limits', () => {
  const base = { completion: { configured: false }, sync: { available: false, peerCount: 0 } };
  assert.equal(chats.parseChatCapabilities(base).textAttachments, undefined);
  assert.equal(chats.parseChatCapabilities({ ...base, textAttachments: { supported: false } }).textAttachments, undefined);
  assert.equal(chats.parseChatCapabilities({ ...base, textAttachments: { supported: true, maxFiles: 10, maxTotalBytes: 1048576 } }).textAttachments.supported, true);
  assert.throws(() => chats.parseChatCapabilities({ ...base, textAttachments: { supported: true, maxFiles: 11, maxTotalBytes: 1048576 } }));
});

test('text attachment drafts isolate chats and clear only acknowledged snapshot', () => {
  const one = [{ name: 'one.txt', text: 'one' }], changed = [{ name: 'two.txt', text: 'two' }];
  chats.setChatTextFiles('one', one); chats.setChatTextFiles('two', changed);
  assert.equal(chats.clearChatTextFilesIfUnchanged('one', changed), false);
  assert.deepEqual(chats.getChatTextFiles('one'), one);
  assert.deepEqual(chats.getChatTextFiles('two'), changed);
  assert.equal(chats.clearChatTextFilesIfUnchanged('one', one), true);
  assert.deepEqual(chats.getChatTextFiles('one'), []);
});

test('capability loss blocks files and acknowledged saves require all files', () => {
  const files = [{ name: 'notes.txt', text: 'body' }];
  assert.equal(chats.canSaveChatTextFiles(null, '', files), false);
  assert.equal(chats.canSaveChatTextFiles({ textAttachments: { supported: true, maxFiles: 10, maxTotalBytes: 1048576 } }, 'offline', files), false);
  assert.equal(chats.canSaveChatTextFiles({ textAttachments: { supported: true, maxFiles: 10, maxTotalBytes: 1048576 } }, '', files), true);
  assert.equal(chats.savedTextFilesMatch([{ name: 'notes.txt', text: 'body' }], files), true);
  assert.equal(chats.savedTextFilesMatch([], files), false);
  assert.equal(chats.savedTextFilesMatch([{ name: '_.txt', text: 'changed' }], files), false);
});

test('repeated identical names retain distinct removable attachment identities', () => {
  const first = { name: 'same.txt', text: 'first' }, second = { name: 'same.txt', text: 'second' };
  chats.setChatTextFiles('identity', [first, second]);
  const reentered = chats.getChatTextFiles('identity');
  assert.equal(reentered[0], first);
  assert.equal(reentered[1], second);
  chats.setChatTextFiles('identity', reentered.filter((_, index) => index !== 0));
  assert.deepEqual(chats.getChatTextFiles('identity'), [second]);
});
