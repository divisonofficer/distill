import assert from 'node:assert/strict';
import { after, before, test } from 'node:test';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createServer } from 'vite';

const root = fileURLToPath(new URL('..', import.meta.url));
let vite;
let chats;

before(async () => {
vite = await createServer({
root,
configFile: resolve(root, 'vite.config.ts'),
server: { middlewareMode: true },
appType: 'custom',
logLevel: 'silent'
});
chats = await vite.ssrLoadModule('/src/lib/chats.ts');
});

after(async () => {
await vite?.close();
});

test('chat list refresh and saved title override update shared display immediately', () => {
let overrides;
const unsubscribe = chats.chatTitleOverrides.subscribe((value) => (overrides = value));
const oldDefault = chats.parseChatList({ chats: [{ chatId: 'title-test', title: 'New chat' }] });
assert.equal(chats.chatDisplayTitle(oldDefault[0], [], { 'title-test': 'Message preview' }, overrides), 'Message preview');
const list = chats.parseChatList({ chats: [{ chatId: 'title-test', title: 'Server title' }] });
assert.equal(chats.chatDisplayTitle(list[0], [], {}, overrides), 'Server title');
chats.setChatTitleOverride('title-test', 'Saved title');
assert.equal(chats.chatDisplayTitle(list[0], [], {}, overrides), 'Saved title');
unsubscribe();
});
