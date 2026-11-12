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

test('selected chat ancestry preserves sibling branches and parent order', () => {
const messages = [
{ messageId: 'root', parentId: null, role: 'user', text: 'root', model: null, author: 'me', seq: 1 },
{ messageId: 'left', parentId: 'root', role: 'user', text: 'left', model: null, author: 'me', seq: 2 },
{ messageId: 'right', parentId: 'root', role: 'user', text: 'right', model: null, author: 'me', seq: 3 },
{ messageId: 'leaf', parentId: 'left', role: 'user', text: 'leaf', model: null, author: 'me', seq: 4 }
];
assert.deepEqual(chats.chatAncestry(messages, 'leaf').map((message) => message.messageId), ['root', 'left', 'leaf']);
assert.deepEqual(chats.chatChildren(messages, 'root').map((message) => message.messageId), ['left', 'right']);
assert.equal(chats.latestChatLeaf(messages), 'leaf');
assert.deepEqual(chats.chatRoots(messages).map((message) => message.messageId), ['root']);
});

test('chat response parsing rejects malformed branch links and inference status', () => {
const message = { messageId: 'm', parentId: null, role: 'user', text: 'saved', model: null, author: 'me', seq: 1 };
assert.deepEqual(chats.parseSavedMessage({ message, inference: { status: 'unavailable', reason: 'not_implemented' } }), message);
assert.throws(() => chats.parseChatHistory({ chatId: 'c', title: '', messages: [{ ...message, parentId: 4 }] }), /parentId/);
assert.throws(() => chats.parseSavedMessage({ message, inference: { status: 'complete', reason: 'ok' } }), /추론 상태/);
});

test('in-memory chat drafts and selected parents survive route switches', () => {
const chatId = 'retention-test';
const messages = [
{ messageId: 'root', parentId: null, role: 'user', text: 'root', model: null, author: 'me', seq: 1 },
{ messageId: 'branch', parentId: 'root', role: 'user', text: 'branch', model: null, author: 'me', seq: 2 }
];
chats.setChatDraft(chatId, 'unsaved draft');
chats.setChatParent(chatId, 'branch');
chats.setChatDraft('another-chat', 'other draft');
assert.equal(chats.getChatDraft(chatId), 'unsaved draft');
assert.equal(chats.restoreChatParent(chatId, messages), 'branch');
assert.equal(chats.restoreChatParent(chatId, messages), 'branch');
});

test('successful save clears only unchanged submitted draft', () => {
const chatId = 'cleanup-test';
chats.setChatDraft(chatId, 'submitted');
const submitted = chats.getChatDraftSnapshot(chatId);
assert.equal(chats.clearChatDraftIfUnchanged(chatId, submitted), true);
assert.equal(chats.getChatDraft(chatId), '');

chats.setChatDraft(chatId, 'submitted');
const stale = chats.getChatDraftSnapshot(chatId);
chats.setChatDraft(chatId, 'newer edit');
assert.equal(chats.clearChatDraftIfUnchanged(chatId, stale), false);
assert.equal(chats.getChatDraft(chatId), 'newer edit');

chats.setChatDraft(chatId, 'same text');
const oldRevision = chats.getChatDraftSnapshot(chatId);
chats.setChatDraft(chatId, 'same text');
assert.equal(chats.clearChatDraftIfUnchanged(chatId, oldRevision), false);
assert.equal(chats.getChatDraft(chatId), 'same text');
});

test('chat titles localize defaults and reuse only loaded real first-message previews', () => {
const chat = { chatId: 'preview-title-test', title: 'New chat' };
assert.equal(chats.chatDisplayTitle(chat), '새 채팅');
assert.equal(chats.chatDisplayTitle({ ...chat, title: 'Chosen title' }), 'Chosen title');
const messages = [
{ messageId: 'later', parentId: 'first', role: 'user', text: 'Later', model: null, author: 'me', seq: 2 },
{ messageId: 'first', parentId: null, role: 'user', text: '  Useful\n first message  ', model: null, author: 'me', seq: 1 }
];
assert.equal(chats.chatDisplayTitle(chat, messages), 'Useful first message');
chats.parseChatHistory({ ...chat, messages });
let titles;
const unsubscribe = chats.chatPreviewTitles.subscribe((value) => (titles = value));
assert.equal(chats.chatDisplayTitle(chat, [], titles), 'Useful first message');
unsubscribe();
assert.deepEqual(messages.map((message) => message.messageId), ['later', 'first']);
assert.equal(chats.chatDisplayTitle({ ...chat, title: '' }, [{ ...messages[0], text: '   ' }]), '새 채팅');
assert.equal(chats.chatDisplayTitle(chat, [{ ...messages[0], text: 'a'.repeat(80) }]).length, 64);
});

test('clearing selected parent persists deliberate root branch choice', () => {
const chatId = 'clear-parent-test';
const messages = [{ messageId: 'existing-root', parentId: null, role: 'user', text: 'saved', model: null, author: 'me', seq: 1 }];
chats.setChatParent(chatId, 'existing-root');
assert.equal(chats.restoreChatParent(chatId, messages), 'existing-root');
chats.setChatParent(chatId, null);
assert.equal(chats.restoreChatParent(chatId, messages), '');
});

test('chat capability and completed-response parsing validates backend status', () => {
assert.deepEqual(chats.parseChatCapabilities({
completion: { configured: true },
sync: { available: true, peerCount: 2, lastAttemptUnixSeconds: 10, lastSuccessUnixSeconds: 9, lastError: null }
}), {
completion: { configured: true },
sync: { available: true, peerCount: 2, lastAttemptUnixSeconds: 10, lastSuccessUnixSeconds: 9, lastError: null }
});
assert.throws(() => chats.parseChatCapabilities({ completion: { configured: 1 }, sync: { available: false, peerCount: 0 } }), /기능 상태/);
assert.throws(() => chats.parseChatCapabilities({ completion: { configured: false }, sync: { available: false, peerCount: -1 } }), /피어 수/);
const assistant = { messageId: 'assistant', parentId: 'user', role: 'assistant', text: '**answer**', model: 'provider/model', author: 'ai', seq: 2 };
assert.deepEqual(chats.parseCompletedMessage({ message: assistant, inference: { status: 'completed' } }), assistant);
assert.throws(() => chats.parseCompletedMessage({ message: assistant, inference: { status: 'unavailable' } }), /응답 생성 상태/);
assert.throws(() => chats.parseCompletedMessage({ message: { ...assistant, role: 'user' }, inference: { status: 'completed' } }), /역할/);
});

test('completion failure classification prevents unsafe retries', () => {
assert.equal(chats.classifyCompletionFailure(new Error('/completion: 409 completion_pending')), 'pending');
assert.equal(chats.classifyCompletionFailure(new Error('/completion: 502 completion_outcome_unknown')), 'unknown');
assert.equal(chats.classifyCompletionFailure(new Error('network disconnected')), 'unknown');
assert.equal(chats.classifyCompletionFailure(new Error('/completion: 400 invalid_model')), 'failed');
assert.equal(chats.classifyCompletionFailure(new Error('/completion: 502 completion_failed')), 'failed');
});

test('chat operation state survives navigation, blocks duplicate sends, and retries saved user with same model', () => {
const chatId = 'operation-flow-test';
const userMessageId = 'saved-user';
const modelId = 'provider/model';
assert.equal(chats.beginChatOperation(chatId, null, modelId, 'saving'), true);
assert.equal(chats.beginChatOperation(chatId, null, modelId, 'saving'), false);
chats.bindChatOperationMessage(chatId, modelId, userMessageId, 'generating');
chats.setChatParent(chatId, userMessageId);
const assistant = { messageId: 'saved-assistant', parentId: userMessageId, role: 'assistant', text: 'answer', model: modelId, author: 'ai', seq: 2 };
assert.deepEqual(chats.parseCompletedMessage({ message: assistant, inference: { status: 'completed' } }), assistant);
chats.updateChatOperation(chatId, userMessageId, { status: 'failed', error: 'temporary failure' });
chats.finishChatOperation(chatId);
const failed = chats.getChatOperation(chatId, userMessageId);
assert.equal(failed.status, 'failed');
assert.equal(failed.userMessageId, userMessageId);
assert.equal(chats.beginChatOperation(chatId, failed.userMessageId, failed.modelId, 'generating'), true);
assert.equal(chats.getChatOperation(chatId, userMessageId).modelId, modelId);
assert.equal(chats.advanceChatParentIfUnchanged(chatId, userMessageId, assistant.messageId), true);
assert.equal(chats.restoreChatParent(chatId, [
{ messageId: userMessageId, parentId: null, role: 'user', text: 'saved', model: null, author: 'me', seq: 1 },
assistant
]), assistant.messageId);
chats.updateChatOperation(chatId, userMessageId, { status: 'completed' });
chats.finishChatOperation(chatId);
assert.equal(chats.getChatOperation(chatId, userMessageId).status, 'completed');
});

test('late completion cannot move a parent the user changed while request ran', () => {
const chatId = 'parent-race-test';
const originalParent = 'saved-user';
const newerParent = 'other-branch';
chats.setChatParent(chatId, originalParent);
chats.setChatParent(chatId, newerParent);
assert.equal(chats.advanceChatParentIfUnchanged(chatId, originalParent, 'assistant'), false);
assert.equal(chats.restoreChatParent(chatId, [
{ messageId: newerParent, parentId: null, role: 'user', text: 'branch', model: null, author: 'me', seq: 1 },
{ messageId: originalParent, parentId: null, role: 'user', text: 'saved', model: null, author: 'me', seq: 2 }
]), newerParent);
});

test('stale history cannot remove saved messages or their selected parent', () => {
const root = { messageId: 'root', parentId: null, role: 'user', text: 'first', model: null, author: 'me', seq: 1 };
const latest = { ...root, messageId: 'latest', parentId: 'root', text: 'next', seq: 2 };
const current = { chatId: 'stale-history', title: 'Chat', messages: [root, latest] };
chats.setChatParent(current.chatId, latest.messageId);
const merged = chats.mergeChatHistory(current, { ...current, messages: [root] });
assert.deepEqual(merged.messages, [root, latest]);
assert.equal(chats.restoreChatParent(current.chatId, merged.messages), 'latest');
assert.equal(chats.mergeChatHistory(current, { ...current, chatId: 'other', messages: [] }).messages.length, 0);
});

test('editing message selects its original parent, including root, without changing the original', () => {
const rootId = 'edit-root-test';
const root = { messageId: 'root', parentId: null, role: 'user', text: 'immutable root', model: null, author: 'me', seq: 1 };
const child = { messageId: 'child', parentId: root.messageId, role: 'user', text: 'child', model: null, author: 'me', seq: 2 };
const childAssistant = { messageId: 'child-answer', parentId: child.messageId, role: 'assistant', text: 'answer', model: 'model', author: 'ai', seq: 3 };
chats.setChatDraft(rootId, 'existing draft');
chats.setChatParent(rootId, childAssistant.messageId);
const rootMode = chats.startChatEdit(rootId, root, childAssistant.messageId);
assert.equal(rootMode.parentId, null);
assert.equal(chats.restoreChatParent(rootId, [root, child, childAssistant]), '');
assert.equal(chats.getChatDraft(rootId), root.text);
assert.deepEqual(root, { messageId: 'root', parentId: null, role: 'user', text: 'immutable root', model: null, author: 'me', seq: 1 });
chats.cancelChatComposerMode(rootId);

const siblingId = 'edit-sibling-test';
const parent = { ...root, messageId: 'parent', text: 'parent' };
const original = { ...child, parentId: parent.messageId, text: 'immutable child' };
chats.setChatParent(siblingId, original.messageId);
const siblingMode = chats.startChatEdit(siblingId, original, original.messageId);
assert.equal(siblingMode.parentId, parent.messageId);
assert.equal(chats.restoreChatParent(siblingId, [parent, original]), parent.messageId);
assert.equal(chats.getChatDraft(siblingId), original.text);
assert.deepEqual(original, { ...child, parentId: parent.messageId, text: 'immutable child' });
chats.cancelChatComposerMode(siblingId);
});

test('cancel restores prior draft and parent after fork selection', () => {
const chatId = 'fork-cancel-restore-test';
const previous = { messageId: 'previous', parentId: null, role: 'user', text: 'previous', model: null, author: 'me', seq: 1 };
const forkTarget = { messageId: 'target', parentId: null, role: 'assistant', text: 'target', model: 'model', author: 'ai', seq: 2 };
chats.setChatDraft(chatId, 'draft not to lose');
chats.setChatParent(chatId, previous.messageId);
const mode = chats.startChatFork(chatId, forkTarget.messageId, previous.messageId);
assert.equal(chats.restoreChatParent(chatId, [previous, forkTarget]), forkTarget.messageId);
chats.setChatDraft(chatId, 'temporary fork draft');
assert.equal(chats.cancelChatComposerMode(chatId).token, mode.token);
assert.equal(chats.getChatDraft(chatId), 'draft not to lose');
assert.equal(chats.restoreChatParent(chatId, [previous, forkTarget]), previous.messageId);
assert.equal(chats.getChatComposerMode(chatId), undefined);
});

test('canceling edit during save restores prior draft and parent against late completion', () => {
const chatId = 'edit-cancel-async-test';
const parent = { messageId: 'parent', parentId: null, role: 'user', text: 'parent', model: null, author: 'me', seq: 1 };
const original = { messageId: 'original', parentId: parent.messageId, role: 'user', text: 'original text', model: null, author: 'me', seq: 2 };
const previousParent = { messageId: 'previous', parentId: null, role: 'user', text: 'previous', model: null, author: 'me', seq: 3 };
const messages = [parent, original, previousParent];
chats.setChatDraft(chatId, 'draft to preserve');
chats.setChatParent(chatId, previousParent.messageId);
const mode = chats.startChatEdit(chatId, original, previousParent.messageId);
const submittedDraft = chats.getChatDraftSnapshot(chatId);
const submittedParent = mode.parentId;

assert.equal(chats.getChatDraft(chatId), original.text);
assert.equal(chats.restoreChatParent(chatId, messages), parent.messageId);
assert.equal(chats.cancelChatComposerMode(chatId).token, mode.token);
assert.equal(chats.getChatDraft(chatId), 'draft to preserve');
assert.equal(chats.restoreChatParent(chatId, messages), previousParent.messageId);
assert.equal(chats.finishChatComposerMode(chatId, mode), false);
assert.equal(chats.clearChatDraftIfUnchanged(chatId, submittedDraft), false);
assert.equal(chats.advanceChatParentIfUnchanged(chatId, submittedParent, 'late-saved-edit'), false);
assert.equal(chats.getChatDraft(chatId), 'draft to preserve');
assert.equal(chats.restoreChatParent(chatId, messages), previousParent.messageId);
});

test('successful fork clears mode but preserves draft edits made after submission', () => {
const chatId = 'fork-success-preserves-edit-test';
const submittedText = 'submitted fork draft';
const submittedParent = 'fork-parent';
const savedId = 'saved-fork-message';
chats.setChatDraft(chatId, submittedText);
const mode = chats.startChatFork(chatId, submittedParent, null);
const submitted = chats.getChatDraftSnapshot(chatId);
chats.setChatDraft(chatId, 'newer draft edit');
assert.equal(chats.finishChatComposerMode(chatId, mode), true);
assert.equal(chats.clearChatDraftIfUnchanged(chatId, submitted), false);
assert.equal(chats.advanceChatParentIfUnchanged(chatId, submittedParent, savedId), true);
assert.equal(chats.getChatDraft(chatId), 'newer draft edit');
assert.equal(chats.getChatComposerMode(chatId), undefined);
assert.equal(chats.restoreChatParent(chatId, [
{ messageId: submittedParent, parentId: null, role: 'user', text: 'fork parent', model: null, author: 'me', seq: 1 },
{ messageId: savedId, parentId: submittedParent, role: 'user', text: submittedText, model: null, author: 'me', seq: 2 }
  ]), savedId);
});

test('chat progress parser validates status, model, and the one-megabyte UTF-8 bound', () => {
assert.deepEqual(chats.parseChatProgress({ status: 'generating', text: 'partial', modelId: 'provider/model' }), {
status: 'generating', text: 'partial', modelId: 'provider/model'
});
assert.deepEqual(chats.parseChatProgress({ status: 'completed', text: '' }), { status: 'completed', text: '' });
assert.throws(() => chats.parseChatProgress({ status: 'running', text: '' }), /진행 상태/);
assert.throws(() => chats.parseChatProgress({ status: 'idle', text: '😀'.repeat(524288) }), /허용 크기/);
assert.throws(() => chats.parseChatProgress({ status: 'idle', text: '', modelId: null }), /진행 모델/);
});

test('chat progress polling reads cumulative snapshots once at a time into shared operation state', async () => {
const chatId = 'progress-poll-test';
const userMessageId = 'progress-user';
const modelId = 'provider/model name';
assert.equal(chats.beginChatOperation(chatId, userMessageId, modelId, 'generating'), true);
let releaseFirst;
let stop = () => {};
let finishResolve;
const firstRead = new Promise((resolve) => { releaseFirst = resolve; });
const finished = new Promise((resolve) => { finishResolve = resolve; });
const paths = [];
const read = (path) => {
paths.push(path);
if (paths.length === 1) return firstRead;
if (paths.length === 4) {
stop();
finishResolve();
return Promise.resolve({ status: 'completed', text: 'canonical-looking but not final' });
}
return Promise.resolve({ status: 'generating', text: paths.length === 2 ? 'snapshot one' : 'full cumulative snapshot' });
};
stop = chats.startChatProgressPolling(chatId, userMessageId, modelId, read, 0);
assert.equal(paths.length, 1);
await new Promise((resolve) => setTimeout(resolve, 5));
assert.equal(paths.length, 1);
releaseFirst({ status: 'generating', text: 'first cumulative snapshot', modelId });
await finished;
await new Promise((resolve) => setTimeout(resolve, 5));
assert.deepEqual(paths, Array(4).fill(`/api/chats/${chatId}/messages/${userMessageId}/progress?modelId=provider%2Fmodel%20name`));
assert.deepEqual(chats.getChatOperation(chatId, userMessageId).progress, {
status: 'generating', text: 'full cumulative snapshot'
});
assert.equal(chats.getChatOperation(chatId, userMessageId).status, 'generating');
stop();
chats.finishChatOperation(chatId);
});

test('missing progress endpoint stops polling without changing completion state', async () => {
const chatId = 'progress-404-test';
const userMessageId = 'progress-user';
const modelId = 'provider/model';
assert.equal(chats.beginChatOperation(chatId, userMessageId, modelId, 'generating'), true);
const paths = [];
const stop = chats.startChatProgressPolling(chatId, userMessageId, modelId, (path) => {
paths.push(path);
return Promise.reject(new Error(`${path}: 404 Not Found`));
}, 0);
await new Promise((resolve) => setTimeout(resolve, 10));
assert.equal(paths.length, 1);
assert.equal(chats.getChatOperation(chatId, userMessageId).status, 'generating');
stop();
chats.finishChatOperation(chatId);
});

test('changing fork target preserves active draft and cancel restores original draft', () => {
const id = 'fork-switch-preserves';
chats.setChatDraft(id, 'original draft');
chats.startChatFork(id, 'a', 'root');
chats.setChatDraft(id, 'draft after fork');
chats.startChatFork(id, 'b', 'a');
assert.equal(chats.getChatDraftSnapshot(id).text, 'draft after fork');
chats.startChatFork(id, 'b', 'b');
assert.equal(chats.getChatDraftSnapshot(id).text, 'draft after fork');
chats.cancelChatComposerMode(id);
assert.equal(chats.getChatDraftSnapshot(id).text, 'original draft');
});
