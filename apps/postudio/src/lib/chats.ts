import { get, writable, type Writable } from 'svelte/store';
import type { TextFileIn } from './types';

export interface ChatSummary {
chatId: string;
title: string;
}

export interface ChatMessage {
messageId: string;
parentId: string | null;
role: string;
text: string;
model: string | null;
author: string;
seq: number;
textFiles: TextFileIn[];
}

export interface ChatHistory extends ChatSummary {
messages: ChatMessage[];
hasMore: boolean;
nextCursor: string | null;
}

const previewTitles = writable<Record<string, string>>({});
export const chatPreviewTitles = { subscribe: previewTitles.subscribe };
const titleOverrides = writable<Record<string, string>>({});
export const chatTitleOverrides = { subscribe: titleOverrides.subscribe };

export function setChatTitleOverride(chatId: string, title: string): void {
titleOverrides.update((titles) => ({ ...titles, [chatId]: title }));
}

export function chatDisplayTitle(
chat: ChatSummary,
messages: ChatMessage[] = [],
previews: Record<string, string> = {},
overrides: Record<string, string> = {}
): string {
if (overrides[chat.chatId]?.trim()) return overrides[chat.chatId];
if (chat.title.trim() && chat.title !== 'New chat') return chat.title;
const first = [...messages].sort((a, b) => a.seq - b.seq).find((message) => message.text.trim());
return first ? first.text.trim().replace(/\s+/g, ' ').slice(0, 64) : previews[chat.chatId] || '새 채팅';
}

const drafts = new Map<string, Writable<string>>();
const textDrafts = new Map<string, Writable<TextFileIn[]>>();
const draftRevisions = new Map<string, number>();
const selectedParents = new Map<string, string | null>();
const composerModes = writable<Record<string, ChatComposerMode>>({});
let nextDraftRevision = 0;
let nextComposerModeToken = 0;

export interface ChatComposerMode {
  token: number;
  kind: 'fork' | 'edit';
  messageId: string;
  parentId: string | null;
  previousDraft: ChatDraftSnapshot;
  previousParentId: string | null;
}

export const chatComposerModeStore = { subscribe: composerModes.subscribe };

export interface ChatCapabilities {
textAttachments?: { supported: true; maxFiles: 10; maxTotalBytes: 1048576 };
completion: { configured: boolean };
sync: {
available: boolean;
peerCount: number;
lastAttemptUnixSeconds: number | null;
lastSuccessUnixSeconds: number | null;
lastError: string | null;
};
}

export type ChatOperationStatus = 'saving' | 'generating' | 'observing' | 'pending' | 'completed' | 'failed' | 'unknown' | 'saved-only' | 'save-failed';

export type ChatProgressStatus = 'idle' | 'generating' | 'completed' | 'failed' | 'unknown';

export interface ChatProgress {
status: ChatProgressStatus;
text: string;
modelId?: string;
}

export interface ChatOperation {
chatId: string;
userMessageId: string | null;
modelId: string;
status: ChatOperationStatus;
error?: string;
progress?: ChatProgress;
}

export const chatOperationStore = writable<Record<string, ChatOperation>>({});
export const chatBusyStore = writable<Record<string, boolean>>({});
const runningChats = new Set<string>();
const progressPollers = new Map<string, { stop: () => void }>();

export function chatOperationKey(chatId: string, userMessageId: string | null): string {
return JSON.stringify([chatId, userMessageId]);
}

export function beginChatOperation(chatId: string, userMessageId: string | null, modelId: string, status: ChatOperationStatus): boolean {
if (runningChats.has(chatId)) return false;
runningChats.add(chatId);
chatBusyStore.update((busy) => ({ ...busy, [chatId]: true }));
const operation = { chatId, userMessageId, modelId, status };
chatOperationStore.update((operations) => ({ ...operations, [chatOperationKey(chatId, userMessageId)]: operation }));
return true;
}

export function bindChatOperationMessage(chatId: string, modelId: string, userMessageId: string, status: ChatOperationStatus): void {
chatOperationStore.update((operations) => {
const next = { ...operations };
delete next[chatOperationKey(chatId, null)];
next[chatOperationKey(chatId, userMessageId)] = { chatId, userMessageId, modelId, status };
return next;
});
}

export function updateChatOperation(chatId: string, userMessageId: string | null, update: Partial<ChatOperation>): void {
chatOperationStore.update((operations) => {
const key = chatOperationKey(chatId, userMessageId);
const current = operations[key];
return current ? { ...operations, [key]: { ...current, ...update } } : operations;
});
if (update.status && update.status !== 'generating') progressPollers.get(chatOperationKey(chatId, userMessageId))?.stop();
}

export function finishChatOperation(chatId: string): void {
runningChats.delete(chatId);
chatBusyStore.update((busy) => ({ ...busy, [chatId]: false }));
}

export function getChatOperation(chatId: string, userMessageId: string | null): ChatOperation | undefined {
return get(chatOperationStore)[chatOperationKey(chatId, userMessageId)];
}

const MAX_CHAT_PROGRESS_BYTES = 1024 * 1024;

export function parseChatProgress(value: unknown): ChatProgress {
const item = record(value);
const status = item.status;
if (status !== 'idle' && status !== 'generating' && status !== 'completed' && status !== 'failed' && status !== 'unknown') {
throw new Error('응답 진행 상태 형식이 올바르지 않습니다.');
}
const text = stringField(item.text, 'progress text');
if (text.length > MAX_CHAT_PROGRESS_BYTES || new TextEncoder().encode(text).byteLength > MAX_CHAT_PROGRESS_BYTES) {
throw new Error('응답 진행 텍스트가 허용 크기를 초과했습니다.');
}
if (item.modelId !== undefined && typeof item.modelId !== 'string') throw new Error('응답 진행 모델 형식이 올바르지 않습니다.');
return { status, text, ...(typeof item.modelId === 'string' ? { modelId: item.modelId } : {}) };
}

export function startChatProgressPolling(
chatId: string,
userMessageId: string,
modelId: string,
read: (path: string) => Promise<unknown>,
intervalMs = 250
): () => void {
const key = chatOperationKey(chatId, userMessageId);
const operation = getChatOperation(chatId, userMessageId);
if (!operation || operation.status !== 'generating' || operation.modelId !== modelId) return () => {};
progressPollers.get(key)?.stop();
let stopped = false;
let failures = 0;
const deadline = Date.now() + 300_000;
let timer: ReturnType<typeof setTimeout> | undefined;
let poller: { stop: () => void };
const stop = () => {
if (stopped) return;
stopped = true;
if (timer !== undefined) clearTimeout(timer);
if (progressPollers.get(key) === poller) progressPollers.delete(key);
};
poller = { stop };
progressPollers.set(key, poller);
const active = () => !stopped && progressPollers.get(key) === poller;
const path = `/api/chats/${encodeURIComponent(chatId)}/messages/${encodeURIComponent(userMessageId)}/progress?modelId=${encodeURIComponent(modelId)}`;
const poll = async () => {
if (!active()) return;
if (Date.now() >= deadline) { stop(); return; }
try {
const progress = parseChatProgress(await read(path));
if (!active()) return;
const current = getChatOperation(chatId, userMessageId);
if (!current || current.status !== 'generating' || current.modelId !== modelId) {
stop();
return;
}
if (progress.modelId === undefined || progress.modelId === modelId) {
updateChatOperation(chatId, userMessageId, { progress });
failures = 0;
if (['completed', 'failed', 'unknown'].includes(progress.status)) { stop(); return; }
}
} catch (cause) {
if (!active()) return;
if (++failures >= 3 || (cause instanceof Error && /: (?:401|403|404)(?:\s|$)/.test(cause.message))) {
stop();
return;
}
}
if (active()) timer = setTimeout(() => { void poll(); }, intervalMs);
};
void poll();
return stop;
}

export interface ChatDraftSnapshot {
text: string;
revision: number;
}

export function canSaveChatTextFiles(capabilities: ChatCapabilities | null, loadError: string, files: TextFileIn[]): boolean {
return files.length === 0 || (!loadError && capabilities?.textAttachments?.supported === true);
}

export function savedTextFilesMatch(saved: TextFileIn[], submitted: TextFileIn[]): boolean {
const canonicalName = (name: string) => Array.from(name.trim().split(/[\\/]/).at(-1)!.split(/[?#]/)[0]).map(ch => /^[a-zA-Z0-9._-]$/.test(ch) ? ch : '_').join('');
return saved.length === submitted.length && saved.every((file, index) =>
file.text === submitted[index].text && canonicalName(file.name) === canonicalName(submitted[index].name));
}

export function chatTextFilesStore(chatId: string): Writable<TextFileIn[]> {
let store = textDrafts.get(chatId);
if (!store) { store = writable([]); textDrafts.set(chatId, store); }
return store;
}
export function setChatTextFiles(chatId: string, files: TextFileIn[]): void {
chatTextFilesStore(chatId).set(files);
}
export function getChatTextFiles(chatId: string): TextFileIn[] { return get(chatTextFilesStore(chatId)); }
export function clearChatTextFilesIfUnchanged(chatId: string, files: TextFileIn[]): boolean {
const current = getChatTextFiles(chatId);
if (current.length !== files.length || current.some((f, i) => f !== files[i])) return false;
setChatTextFiles(chatId, []); return true;
}

export function chatDraftStore(chatId: string): Writable<string> {
let store = drafts.get(chatId);
if (!store) {
store = writable('');
drafts.set(chatId, store);
}
return store;
}

export function getChatDraft(chatId: string): string {
return get(chatDraftStore(chatId));
}

export function getChatDraftSnapshot(chatId: string): ChatDraftSnapshot {
return { text: getChatDraft(chatId), revision: draftRevisions.get(chatId) ?? 0 };
}

export function setChatDraft(chatId: string, text: string): void {
draftRevisions.set(chatId, ++nextDraftRevision);
chatDraftStore(chatId).set(text);
}

export function clearChatDraftIfUnchanged(chatId: string, submitted: ChatDraftSnapshot): boolean {
const current = getChatDraftSnapshot(chatId);
if (current.text !== submitted.text || current.revision !== submitted.revision) return false;
setChatDraft(chatId, '');
return true;
}

export function setChatParent(chatId: string, parentId: string | null): void {
selectedParents.set(chatId, parentId);
}

export function advanceChatParentIfUnchanged(chatId: string, previousParentId: string | null, nextParentId: string): boolean {
if (!selectedParents.has(chatId) || selectedParents.get(chatId) !== previousParentId) return false;
selectedParents.set(chatId, nextParentId);
return true;
}

export function restoreChatParent(chatId: string, messages: ChatMessage[]): string {
const selected = selectedParents.get(chatId);
if (selected === null) return '';
if (selected !== undefined) return selected;
const latest = latestChatLeaf(messages) ?? '';
setChatParent(chatId, latest || null);
return latest;
}

export function getChatComposerMode(chatId: string): ChatComposerMode | undefined {
return get(composerModes)[chatId];
}

function beginChatComposerMode(
chatId: string,
kind: ChatComposerMode['kind'],
messageId: string,
parentId: string | null,
currentParentId: string | null,
editText?: string
): ChatComposerMode {
const previousMode = getChatComposerMode(chatId);
if (previousMode?.kind === kind && previousMode.messageId === messageId) return previousMode;
if (previousMode) currentParentId = previousMode.previousParentId;
const mode: ChatComposerMode = {
token: ++nextComposerModeToken,
kind,
messageId,
parentId,
previousDraft: previousMode?.previousDraft ?? getChatDraftSnapshot(chatId),
previousParentId: currentParentId
};
setChatParent(chatId, parentId);
if (kind === 'edit') setChatDraft(chatId, editText ?? '');
composerModes.update((modes) => ({ ...modes, [chatId]: mode }));
return mode;
}

export function startChatFork(chatId: string, messageId: string, currentParentId: string | null): ChatComposerMode {
return beginChatComposerMode(chatId, 'fork', messageId, messageId, currentParentId);
}

export function startChatEdit(chatId: string, message: ChatMessage, currentParentId: string | null): ChatComposerMode {
if (message.role !== 'user') throw new Error('수정할 수 있는 메시지가 아닙니다.');
return beginChatComposerMode(chatId, 'edit', message.messageId, message.parentId, currentParentId, message.text);
}

export function cancelChatComposerMode(chatId: string): ChatComposerMode | undefined {
const mode = getChatComposerMode(chatId);
if (!mode) return undefined;
setChatDraft(chatId, mode.previousDraft.text);
setChatParent(chatId, mode.previousParentId);
composerModes.update((modes) => {
const next = { ...modes };
delete next[chatId];
return next;
});
return mode;
}

export function finishChatComposerMode(chatId: string, submittedMode: ChatComposerMode): boolean {
if (getChatComposerMode(chatId)?.token !== submittedMode.token) return false;
composerModes.update((modes) => {
const next = { ...modes };
delete next[chatId];
return next;
});
return true;
}

function record(value: unknown): Record<string, unknown> {
if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error('채팅 응답 형식이 올바르지 않습니다.');
return value as Record<string, unknown>;
}

function stringField(value: unknown, name: string): string {
if (typeof value !== 'string') throw new Error(`채팅 응답의 ${name} 값이 올바르지 않습니다.`);
return value;
}

function parseSummary(value: unknown): ChatSummary {
const item = record(value);
return { chatId: stringField(item.chatId, 'chatId'), title: stringField(item.title, 'title') };
}

function parseTextFiles(value: unknown): TextFileIn[] {
if (value === undefined) return [];
if (!Array.isArray(value) || value.length > 10) throw new Error('텍스트 파일 첨부 형식이 올바르지 않습니다.');
let total = 0;
return value.map((entry) => {
const item = record(entry);
const name = stringField(item.name, 'text file name');
const text = stringField(item.text, 'text file text');
if (!/\.(txt|md)$/i.test(name)) throw new Error('TXT 또는 MD 파일만 첨부할 수 있습니다.');
const size = new TextEncoder().encode(text).byteLength; total += size;
if (total > 1048576) throw new Error('첨부 전체 크기는 1 MiB 이하여야 합니다.');
return { name, text };
});
}

function parseMessage(value: unknown): ChatMessage {
const item = record(value);
if (item.parentId !== null && typeof item.parentId !== 'string') throw new Error('채팅 응답의 parentId 값이 올바르지 않습니다.');
if (item.model !== null && typeof item.model !== 'string') throw new Error('채팅 응답의 model 값이 올바르지 않습니다.');
if (typeof item.seq !== 'number' || !Number.isFinite(item.seq)) throw new Error('채팅 응답의 seq 값이 올바르지 않습니다.');
return {
messageId: stringField(item.messageId, 'messageId'),
parentId: item.parentId,
role: stringField(item.role, 'role'),
text: stringField(item.text, 'text'),
model: item.model,
author: stringField(item.author, 'author'),
seq: item.seq,
textFiles: parseTextFiles(item.textFiles)
};
}

export function parseChatList(value: unknown): ChatSummary[] {
const chats = record(value).chats;
if (!Array.isArray(chats)) throw new Error('채팅 목록 응답 형식이 올바르지 않습니다.');
const summaries = chats.map(parseSummary);
titleOverrides.update((titles) => {
const next = { ...titles };
for (const { chatId, title } of summaries) {
if (title.trim() && title !== 'New chat') next[chatId] = title;
else delete next[chatId];
}
return next;
});
return summaries;
}

export function parseCreatedChat(value: unknown): ChatSummary {
return parseSummary(value);
}

export function parseChatHistory(value: unknown): ChatHistory {
const item = record(value);
if (!Array.isArray(item.messages)) throw new Error('채팅 기록 응답 형식이 올바르지 않습니다.');
const hasMore = item.hasMore ?? false;
const nextCursor = item.nextCursor ?? null;
if (typeof hasMore !== 'boolean') throw new Error('채팅 기록 페이지 상태가 올바르지 않습니다.');
if (nextCursor !== null && (typeof nextCursor !== 'string' || !nextCursor || nextCursor.length > 4096)) {
throw new Error('채팅 기록 커서 형식이 올바르지 않습니다.');
}
if (hasMore !== (nextCursor !== null)) throw new Error('채팅 기록 페이지 커서가 올바르지 않습니다.');
const history = { ...parseSummary(item), messages: item.messages.map(parseMessage), hasMore, nextCursor };
previewTitles.update((titles) => ({ ...titles, [history.chatId]: chatDisplayTitle(history, history.messages) }));
return history;
}

export function parseSavedMessage(value: unknown): ChatMessage {
const item = record(value);
const inference = record(item.inference);
if (inference.status !== 'unavailable' || inference.reason !== 'not_implemented') {
throw new Error('저장 응답의 추론 상태가 올바르지 않습니다.');
}
return parseMessage(item.message);
}

export function parseCompletedMessage(value: unknown): ChatMessage {
const item = record(value);
if (record(item.inference).status !== 'completed') throw new Error('응답 생성 상태가 올바르지 않습니다.');
const message = parseMessage(item.message);
if (message.role !== 'assistant') throw new Error('응답 메시지 역할이 올바르지 않습니다.');
return message;
}

export function parseChatCapabilities(value: unknown): ChatCapabilities {
const item = record(value);
const completion = record(item.completion);
const sync = record(item.sync);
if (typeof completion.configured !== 'boolean' || typeof sync.available !== 'boolean') {
throw new Error('채팅 기능 상태 형식이 올바르지 않습니다.');
}
if (typeof sync.peerCount !== 'number' || !Number.isInteger(sync.peerCount) || sync.peerCount < 0) {
throw new Error('동기화 피어 수 형식이 올바르지 않습니다.');
}
const timestamp = (candidate: unknown, name: string): number | null => {
if (candidate === null || candidate === undefined) return null;
if (typeof candidate !== 'number' || !Number.isFinite(candidate)) throw new Error(`${name} 형식이 올바르지 않습니다.`);
return candidate;
};
const lastError = sync.lastError;
if (lastError !== null && lastError !== undefined && typeof lastError !== 'string') {
throw new Error('동기화 오류 상태 형식이 올바르지 않습니다.');
}
const rawTextAttachments = item.textAttachments;
let textAttachments: ChatCapabilities['textAttachments'];
if (rawTextAttachments !== undefined) {
const text = record(rawTextAttachments);
if (text.supported !== false && text.supported !== true) throw new Error('텍스트 첨부 기능 상태 형식이 올바르지 않습니다.');
if (text.supported === true) {
if (text.maxFiles !== 10 || text.maxTotalBytes !== 1048576) throw new Error('텍스트 첨부 제한 형식이 올바르지 않습니다.');
textAttachments = { supported: true, maxFiles: 10, maxTotalBytes: 1048576 };
}
}
return {
...(textAttachments ? { textAttachments } : {}),
completion: { configured: completion.configured },
sync: {
available: sync.available,
peerCount: sync.peerCount,
lastAttemptUnixSeconds: timestamp(sync.lastAttemptUnixSeconds, 'lastAttemptUnixSeconds'),
lastSuccessUnixSeconds: timestamp(sync.lastSuccessUnixSeconds, 'lastSuccessUnixSeconds'),
lastError: typeof lastError === 'string' ? lastError : null
}
};
}

export type CompletionFailure = 'pending' | 'unknown' | 'failed';

export function classifyCompletionFailure(cause: unknown): CompletionFailure {
const message = cause instanceof Error ? cause.message : '';
if (/completion_pending/.test(message) || /:\s*409(?:\s|$)/.test(message)) return 'pending';
if (/:\s*502 completion_failed$/.test(message)) return 'failed';
const status = message.match(/:\s*(\d{3})(?:\s|$)/)?.[1];
if (message.includes('completion_outcome_unknown') || !status || Number(status) >= 500 || Number(status) === 408) return 'unknown';
return 'failed';
}

export function chatRoots(messages: ChatMessage[]): ChatMessage[] {
return messages.filter((message) => message.parentId === null).sort((a, b) => a.seq - b.seq);
}

export function chatChildren(messages: ChatMessage[], parentId: string): ChatMessage[] {
return messages.filter((message) => message.parentId === parentId).sort((a, b) => a.seq - b.seq);
}

export function chatAncestry(messages: ChatMessage[], leafId: string): ChatMessage[] {
const byId = new Map(messages.map((message) => [message.messageId, message]));
const path: ChatMessage[] = [];
const seen = new Set<string>();
let current = byId.get(leafId);
while (current && !seen.has(current.messageId)) {
seen.add(current.messageId);
path.push(current);
current = current.parentId === null ? undefined : byId.get(current.parentId);
}
return path.reverse();
}

export function latestChatLeaf(messages: ChatMessage[]): string | undefined {
const parents = new Set(messages.flatMap((message) => (message.parentId === null ? [] : [message.parentId])));
return messages.filter((message) => !parents.has(message.messageId)).sort((a, b) => b.seq - a.seq)[0]?.messageId;
}

export function chatMessagePreview(message: ChatMessage): string {
const text = message.text.replace(/\s+/g, ' ').trim();
return text.length > 100 ? `${text.slice(0, 100)}…` : text;
}

export function chatHistoryPath(chatId: string, cursor: string | null = null): string {
const path = `/api/chats/${encodeURIComponent(chatId)}`;
return cursor ? `${path}?cursor=${encodeURIComponent(cursor)}` : path;
}

export function mergeChatHistory(current: ChatHistory | null, incoming: ChatHistory): ChatHistory {
if (!current || current.chatId !== incoming.chatId) return incoming;
const messages = new Map(current.messages.map((message) => [message.messageId, message]));
for (const message of incoming.messages) messages.set(message.messageId, message);
return { ...incoming, messages: [...messages.values()] };
}
