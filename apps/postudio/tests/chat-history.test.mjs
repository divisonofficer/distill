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

function message(messageId, parentId, seq) {
	return { messageId, parentId, role: 'user', text: messageId, model: null, author: 'node', seq };
}

function history(messages, nextCursor = null) {
	return {
		chatId: 'paging-chat',
		title: 'Paging test',
		messages,
		hasMore: nextCursor !== null,
		nextCursor
	};
}

test('history parser keeps legacy responses and validates paging cursors', () => {
	const legacy = chats.parseChatHistory({
		chatId: 'legacy', title: 'Legacy', messages: [message('root', null, 1)]
	});
	assert.equal(legacy.hasMore, false);
	assert.equal(legacy.nextCursor, null);
	assert.throws(() => chats.parseChatHistory({
		chatId: 'bad', title: 'Bad', messages: [], hasMore: true, nextCursor: null
	}), /페이지 커서/);
	assert.throws(() => chats.parseChatHistory({
		chatId: 'bad', title: 'Bad', messages: [], hasMore: true, nextCursor: 'x'.repeat(4097)
	}), /커서 형식/);
});

test('history URL resets cursor by default and safely encodes older cursor', () => {
	assert.equal(chats.chatHistoryPath('chat/2'), '/api/chats/chat%2F2');
	assert.equal(chats.chatHistoryPath('chat/2', null), '/api/chats/chat%2F2');
	assert.equal(chats.chatHistoryPath('chat/2', 'opaque+/=token'), '/api/chats/chat%2F2?cursor=opaque%2B%2F%3Dtoken');
});

test('refresh restarts older traversal from latest page while retaining loaded history', () => {
	const alreadyLoaded = chats.parseChatHistory(history(
		Array.from({ length: 100 }, (_, index) => message(`message-${index + 1}`, index ? `message-${index}` : null, index + 1))
	));
	const latestPage = chats.parseChatHistory(history(
		Array.from({ length: 50 }, (_, index) => message(`message-${index + 111}`, `message-${index + 110}`, index + 111)),
		'fresh-position-110'
	));
	const refreshed = chats.mergeChatHistory(alreadyLoaded, latestPage);
	assert.equal(refreshed.hasMore, true);
	assert.equal(refreshed.nextCursor, 'fresh-position-110');
	assert.equal(refreshed.messages.length, 150);
	assert.ok(!refreshed.messages.some((item) => item.messageId === 'message-105'));

	const olderPage = chats.parseChatHistory(history([
		...Array.from({ length: 50 }, (_, index) => message(`message-${index + 61}`, `message-${index + 60}`, index + 61)),
		{ ...message('imported-earlier-author', null, 1), author: 'aaa-imported-author' }
	], 'fresh-position-60'));
	const traversed = chats.mergeChatHistory(refreshed, olderPage);
	assert.equal(traversed.nextCursor, 'fresh-position-60');
	assert.ok(traversed.messages.some((item) => item.messageId === 'message-105'));
	assert.ok(traversed.messages.some((item) => item.author === 'aaa-imported-author'));

	const beforeOldest = chats.mergeChatHistory(traversed, chats.parseChatHistory(history(
		Array.from({ length: 50 }, (_, index) => message(`message-${index + 11}`, `message-${index + 10}`, index + 11)),
		'fresh-position-10'
	)));
	const complete = chats.mergeChatHistory(beforeOldest, chats.parseChatHistory(history(
		Array.from({ length: 10 }, (_, index) => message(`message-${index + 1}`, index ? `message-${index}` : null, index + 1))
	)));
	assert.equal(complete.hasMore, false);
	assert.equal(complete.messages.length, 161);
	assert.equal(new Set(complete.messages.map((item) => item.messageId)).size, 161);
	assert.ok(complete.messages.some((item) => item.author === 'aaa-imported-author'));
});

test('selected older parent and draft survive route changes while parent is paged out', () => {
	const chatId = 'paged-parent-route-switch';
	const parent = message('old-parent', null, 1);
	const latestMessages = Array.from({ length: 50 }, (_, index) => message(`latest-${index + 1}`, null, index + 101));
	chats.setChatDraft(chatId, 'unsent draft');
	chats.setChatParent(chatId, parent.messageId);

	assert.equal(chats.restoreChatParent(chatId, latestMessages), parent.messageId);
	assert.equal(chats.getChatDraft(chatId), 'unsent draft');
	assert.ok(!latestMessages.some((item) => item.messageId === parent.messageId));

	const loadedOlder = chats.mergeChatHistory(
		history(latestMessages, 'older-cursor'),
		history([parent, message('old-child', parent.messageId, 2)])
	);
	assert.equal(chats.restoreChatParent(chatId, loadedOlder.messages), parent.messageId);
	assert.equal(chats.getChatDraft(chatId), 'unsent draft');
});

test('page boundary stays a missing parent, then loaded page restores branch navigation', () => {
	const latest = [message('selected', 'shared-root', 4), message('selected-leaf', 'selected', 5)];
	const truncated = chats.chatAncestry(latest, 'selected-leaf');
	assert.deepEqual(truncated.map((item) => item.messageId), ['selected', 'selected-leaf']);
	assert.equal(truncated[0].parentId, 'shared-root');
	assert.deepEqual(chats.chatRoots(latest), []);

	const all = chats.mergeChatHistory(
		history(latest),
		history([message('shared-root', null, 1), message('alternate', 'shared-root', 2), message('selected', 'shared-root', 4)])
	);
	assert.deepEqual(chats.chatAncestry(all.messages, 'selected-leaf').map((item) => item.messageId), ['shared-root', 'selected', 'selected-leaf']);
	assert.deepEqual(chats.chatChildren(all.messages, 'shared-root').map((item) => item.messageId), ['alternate', 'selected']);
});
