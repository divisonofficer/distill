<script lang="ts">
import { page } from '$app/state';
import { afterNavigate } from '$app/navigation';
import { onMount, tick } from 'svelte';
import { isNearBottom } from '$lib/task-history-scroll';
import { getJson, postJson } from '$lib/api';
import {
advanceChatParentIfUnchanged,
beginChatOperation,
bindChatOperationMessage,
cancelChatComposerMode,
chatAncestry,
chatComposerModeStore,
chatBusyStore,
chatChildren,
chatDisplayTitle,
chatHistoryPath,
chatDraftStore,
chatTitleOverrides,
chatMessagePreview,
chatOperationKey,
chatOperationStore,
chatRoots,
classifyCompletionFailure,
clearChatDraftIfUnchanged,
finishChatOperation,
finishChatComposerMode,
getChatComposerMode,
getChatDraftSnapshot,
mergeChatHistory,
parseChatCapabilities,
parseChatHistory,
parseCreatedChat,
parseCompletedMessage,
parseSavedMessage,
restoreChatParent,
setChatDraft,
setChatParent,
setChatTitleOverride,
startChatEdit,
startChatFork,
startChatProgressPolling,
updateChatOperation,
type ChatCapabilities,
type ChatHistory,
type ChatMessage,
type ChatOperation
} from '$lib/chats';
import Icon from '$lib/components/Icon.svelte';
import HistorySkeleton from '$lib/components/HistorySkeleton.svelte';
import Markdown from '$lib/components/Markdown.svelte';
import ModelSelect from '$lib/components/ModelSelect.svelte';
import { ui } from '$lib/ui.svelte';

let activeChatId = $state<string>(page.params.id ?? '');
let history = $state.raw<ChatHistory | null>(null);
let selectedLeafId = $state('');
let draft = $state('');
let modelId = $state('');
let capabilities = $state<ChatCapabilities | null>(null);
let capabilityLoading = $state(true);
let capabilityError = $state('');
let loading = $state(true);
let notFound = $state(false);
let historyError = $state('');
let saveError = $state('');
let titleEditing = $state(false);
let titleDraft = $state('');
let titleSaving = $state(false);
let titleError = $state('');
let titleInput = $state<HTMLInputElement>();
let requestId = 0;
let messageScroller = $state<HTMLDivElement>();
let messageContent = $state<HTMLDivElement>();
let olderCursor = $state<string | null>(null);
let hasOlderMessages = $state(false);
let olderLoading = $state(false);
let olderError = $state('');
let olderRequestId = 0;
let newestPageFingerprint = '';
let followLatest = true;

async function scrollToLatest(id: string, force = false) {
if (id !== activeChatId || (!force && !followLatest)) return;
if (force) followLatest = true;
await tick();
if (id === activeChatId && followLatest && messageScroller) messageScroller.scrollTop = messageScroller.scrollHeight;
}

$effect(() => {
const content = messageContent;
if (!content) return;
const observer = new ResizeObserver(() => { void scrollToLatest(activeChatId); });
observer.observe(content);
return () => observer.disconnect();
});
let entranceTimer: ReturnType<typeof setTimeout> | undefined;
let enteringMessageId = $state('');
let unsubscribeDraft = () => {};
const ancestry = $derived(history ? chatAncestry(history.messages, selectedLeafId) : []);
const roots = $derived(history ? chatRoots(history.messages) : []);
const selectedParentMissing = $derived(Boolean(selectedLeafId && !history?.messages.some((message) => message.messageId === selectedLeafId)));
const saving = $derived($chatBusyStore[activeChatId] ?? false);
const completionConfigured = $derived(capabilities?.completion.configured === true);

async function refreshCapabilities() {
capabilityLoading = true;
capabilityError = '';
try {
capabilities = parseChatCapabilities(await getJson<unknown>('/api/chats/capabilities'));
} catch (cause) {
capabilities = null;
capabilityError = cause instanceof Error ? cause.message : '알 수 없는 오류';
} finally {
capabilityLoading = false;
}
}

function pageFingerprint(page: ChatHistory): string {
return JSON.stringify([page.hasMore, page.nextCursor, page.messages.map((message) => [message.messageId, message.parentId, message.author, message.seq, message.text, message.model])]);
}

async function refreshHistory(id: string, restartTraversal = true) {
if (!id) return;
const request = ++requestId;
olderRequestId++;
olderLoading = false;
if (history?.chatId !== id) loading = true;
try {
const next = parseChatHistory(await getJson<unknown>(chatHistoryPath(id)));
if (request !== requestId || id !== activeChatId) return;
history = mergeChatHistory(history, next);
const nextFingerprint = pageFingerprint(next);
if (restartTraversal || !newestPageFingerprint || nextFingerprint !== newestPageFingerprint) {
olderCursor = next.nextCursor;
hasOlderMessages = next.hasMore;
}
newestPageFingerprint = nextFingerprint;
olderError = '';
loading = false;
notFound = false;
historyError = '';
selectedLeafId = restoreChatParent(id, history.messages);
void scrollToLatest(id);
} catch (cause) {
if (request !== requestId || id !== activeChatId) return;
loading = false;
const detail = cause instanceof Error ? cause.message : '알 수 없는 오류';
if (/: 404(?:\s|$)/.test(detail)) {
history = null;
notFound = true;
historyError = '';
} else {
historyError = `채팅 기록을 불러오지 못했습니다. ${detail}`;
}
}
}

function refreshCurrent() {
void refreshHistory(activeChatId);
}

function beginTitleEdit() {
if (!history || titleSaving) return;
titleDraft = history.title;
titleError = '';
titleEditing = true;
void tick().then(() => titleInput?.focus());
}

function cancelTitleEdit() {
if (titleSaving) return;
titleEditing = false;
titleError = '';
titleDraft = history?.title ?? '';
}

function handleTitleKeydown(event: KeyboardEvent) {
if (event.isComposing || event.keyCode === 229) return;
if (event.key === 'Enter') {
event.preventDefault();
void saveChatTitle();
} else if (event.key === 'Escape') {
event.preventDefault();
cancelTitleEdit();
}
}

async function saveChatTitle() {
const id = activeChatId;
if (!history || history.chatId !== id || titleSaving) return;
const title = titleDraft.trim();
if (!title) {
titleError = '제목을 입력하세요.';
return;
}
if (new TextEncoder().encode(title).length > 300) {
titleError = '제목은 300 UTF-8 바이트 이하여야 합니다.';
return;
}
titleSaving = true;
titleError = '';
try {
const saved = parseCreatedChat(await postJson<unknown>(`/api/chats/${encodeURIComponent(id)}/title`, { title }));
if (saved.chatId !== id) throw new Error('저장한 채팅 제목이 일치하지 않습니다.');
setChatTitleOverride(id, saved.title);
if (activeChatId !== id || history?.chatId !== id) return;
history = { ...history, title: saved.title };
titleDraft = saved.title;
titleEditing = false;
} catch (cause) {
if (activeChatId === id) titleError = cause instanceof Error ? cause.message : '제목을 저장하지 못했습니다.';
} finally {
if (activeChatId === id) titleSaving = false;
}
}

async function loadOlderHistory() {
const id = activeChatId;
const cursor = olderCursor;
const scroller = messageScroller;
if (!id || !cursor || !history || history.chatId !== id || olderLoading) return;
const request = ++olderRequestId;
const beforeHeight = scroller?.scrollHeight ?? 0;
const beforeTop = scroller?.scrollTop ?? 0;
followLatest = false;
olderLoading = true;
olderError = '';
try {
const older = parseChatHistory(await getJson<unknown>(chatHistoryPath(id, cursor)));
if (request !== olderRequestId || id !== activeChatId || cursor !== olderCursor) return;
history = mergeChatHistory(history, older);
olderCursor = older.nextCursor;
hasOlderMessages = older.hasMore;
selectedLeafId = restoreChatParent(id, history.messages);
await tick();
if (id === activeChatId && messageScroller) {
messageScroller.scrollTop = beforeTop + (messageScroller.scrollHeight - beforeHeight);
followLatest = isNearBottom(messageScroller);
}
} catch (cause) {
if (request !== olderRequestId || id !== activeChatId || cursor !== olderCursor) return;
const detail = cause instanceof Error ? cause.message : '알 수 없는 오류';
olderError = `이전 기록을 불러오지 못했습니다. ${detail}`;
} finally {
if (request === olderRequestId && id === activeChatId) olderLoading = false;
}
}

function watchDraft(chatId: string) {
unsubscribeDraft();
if (!chatId) return;
unsubscribeDraft = chatDraftStore(chatId).subscribe((value) => (draft = value));
}

function afterRouteNavigation() {
const nextId = page.params.id ?? '';
if (nextId !== activeChatId) {
if (entranceTimer !== undefined) window.clearTimeout(entranceTimer);
entranceTimer = undefined;
enteringMessageId = '';
requestId++;
olderRequestId++;
activeChatId = nextId;
followLatest = true;
history = null;
selectedLeafId = '';
olderCursor = null;
hasOlderMessages = false;
newestPageFingerprint = '';
olderLoading = false;
olderError = '';
loading = true;
notFound = false;
historyError = '';
saveError = '';
titleEditing = false;
titleError = '';
titleDraft = '';
watchDraft(nextId);
selectedLeafId = '';
}
void refreshHistory(nextId);
}

afterNavigate(afterRouteNavigation);
onMount(() => {
watchDraft(activeChatId);
void refreshCapabilities();
const timer = window.setInterval(() => void refreshHistory(activeChatId, false), 15000);
return () => {
window.clearInterval(timer);
if (entranceTimer !== undefined) window.clearTimeout(entranceTimer);
unsubscribeDraft();
};
});

function updateDraft(event: Event) {
draft = (event.currentTarget as HTMLTextAreaElement).value;
setChatDraft(activeChatId, draft);
saveError = '';
}

function chooseParent(messageId: string) {
selectedLeafId = messageId;
setChatParent(activeChatId, messageId);
}

function clearParent() {
selectedLeafId = '';
setChatParent(activeChatId, null);
}

async function focusComposer(id: string) {
await tick();
if (activeChatId === id) document.getElementById('chat-message')?.focus();
}

function forkFromMessage(message: ChatMessage) {
const id = activeChatId;
const mode = startChatFork(id, message.messageId, selectedLeafId || null);
selectedLeafId = mode.parentId ?? '';
void focusComposer(id);
}

function editMessage(message: ChatMessage) {
if (message.parentId && !history?.messages.some((candidate) => candidate.messageId === message.parentId)) return;
const id = activeChatId;
const mode = startChatEdit(id, message, selectedLeafId || null);
selectedLeafId = mode.parentId ?? '';
void focusComposer(id);
}

function cancelComposerMode() {
const mode = cancelChatComposerMode(activeChatId);
if (mode) selectedLeafId = mode.previousParentId ?? '';
}

function selectBranch(message: ChatMessage) {
selectedLeafId = message.messageId;
setChatParent(activeChatId, message.messageId);
}

function moveSibling(messages: ChatMessage[], selectedId: string | undefined, delta: number) {
if (messages.length < 2) return;
const current = messages.findIndex((message) => message.messageId === selectedId);
const next = current < 0 ? (delta > 0 ? 0 : messages.length - 1) : (current + delta + messages.length) % messages.length;
selectBranch(messages[next]);
}

function applyHistoryRecord(id: string, next: ChatHistory) {
if (id !== activeChatId || next.chatId !== id) return;
olderRequestId++;
olderLoading = false;
history = mergeChatHistory(history, next);
olderCursor = next.nextCursor;
hasOlderMessages = next.hasMore;
newestPageFingerprint = pageFingerprint(next);
olderError = '';
loading = false;
notFound = false;
historyError = '';
selectedLeafId = restoreChatParent(id, history.messages);
}

function appendMessage(id: string, message: ChatMessage) {
if (id !== activeChatId || history?.chatId !== id) return;
const isNew = !history.messages.some((current) => current.messageId === message.messageId);
const messages = [...history.messages.filter((current) => current.messageId !== message.messageId), message];
history = { ...history, messages };
selectedLeafId = restoreChatParent(id, messages);
if (isNew) {
enteringMessageId = message.messageId;
if (entranceTimer !== undefined) window.clearTimeout(entranceTimer);
entranceTimer = window.setTimeout(() => {
if (enteringMessageId === message.messageId) enteringMessageId = '';
}, 350);
}
}

function assistantReply(messages: ChatMessage[], userMessageId: string): ChatMessage | undefined {
return chatChildren(messages, userMessageId).find((message) => message.role === 'assistant');
}

function acceptAssistant(id: string, userMessageId: string, assistant: ChatMessage, next?: ChatHistory) {
advanceChatParentIfUnchanged(id, userMessageId, assistant.messageId);
updateChatOperation(id, userMessageId, { status: 'completed', error: undefined, progress: undefined });
if (next) applyHistoryRecord(id, next);
else appendMessage(id, assistant);
if (!next && id === activeChatId) void refreshHistory(id);
}

async function pollCompletion(id: string, userMessageId: string): Promise<{ message?: ChatMessage; next?: ChatHistory; error?: string }> {
for (let attempt = 0; attempt < 20; attempt++) {
await new Promise((resolve) => window.setTimeout(resolve, 1000));
try {
const next = parseChatHistory(await getJson<unknown>(chatHistoryPath(id)));
const assistant = assistantReply(next.messages, userMessageId);
if (assistant) return { message: assistant, next };
if (id === activeChatId) applyHistoryRecord(id, next);
} catch (cause) {
return { error: cause instanceof Error ? cause.message : '알 수 없는 오류' };
}
}
return {};
}

async function requestCompletion(id: string, userMessageId: string, requestedModelId: string) {
updateChatOperation(id, userMessageId, { status: 'generating', error: undefined, progress: undefined });
const completionRequest = postJson<unknown>(
`/api/chats/${encodeURIComponent(id)}/messages/${encodeURIComponent(userMessageId)}/completion`,
{ modelId: requestedModelId }
);
const stopProgressPolling = startChatProgressPolling(id, userMessageId, requestedModelId, (path) => getJson<unknown>(path));
try {
const assistant = parseCompletedMessage(await completionRequest);
stopProgressPolling();
if (assistant.parentId !== userMessageId) throw new Error('응답 메시지 부모가 올바르지 않습니다.');
acceptAssistant(id, userMessageId, assistant);
} catch (cause) {
stopProgressPolling();
const detail = cause instanceof Error ? cause.message : '알 수 없는 오류';
const disposition = classifyCompletionFailure(cause);
if (disposition === 'pending') {
updateChatOperation(id, userMessageId, { status: 'observing', error: '', progress: undefined });
const observed = await pollCompletion(id, userMessageId);
if (observed.message) {
acceptAssistant(id, userMessageId, observed.message, 'next' in observed ? observed.next : undefined);
} else {
updateChatOperation(id, userMessageId, {
status: 'pending',
error: observed.error ? `기록을 확인하지 못했습니다. ${observed.error}` : '응답이 아직 기록되지 않았습니다. 요청은 다시 보내지 않았습니다.'
});
}
} else {
updateChatOperation(id, userMessageId, {
status: disposition,
progress: undefined,
error: disposition === 'unknown'
? `요청 결과를 확인할 수 없습니다. 자동 재요청하지 않았습니다. ${detail}`
: detail
});
}
}
}

async function saveMessage(generate: boolean) {
const id = activeChatId;
const text = draft;
const requestedModelId = generate ? modelId : '';
if (!history || history.chatId !== id || !text.trim() || saving) return;
if (selectedParentMissing) {
saveError = '선택한 부모 메시지를 불러온 뒤 저장할 수 있습니다. 이전 기록을 불러오세요.';
return;
}
if (generate && (!completionConfigured || !requestedModelId)) return;
const parentId = selectedLeafId || null;
const submittedDraft = getChatDraftSnapshot(id);
const submittedMode = getChatComposerMode(id);
if (!beginChatOperation(id, null, requestedModelId, 'saving')) return;
saveError = '';
try {
let saved: ChatMessage;
try {
saved = parseSavedMessage(await postJson<unknown>(`/api/chats/${encodeURIComponent(id)}/messages`, { text, parentId }));
} catch (cause) {
const detail = cause instanceof Error ? cause.message : '알 수 없는 오류';
updateChatOperation(id, null, { status: 'save-failed', error: detail });
if (id === activeChatId) saveError = `메시지를 저장하지 못했습니다. ${detail}`;
return;
}
const submittedContextStillCurrent = submittedMode
? finishChatComposerMode(id, submittedMode)
: !getChatComposerMode(id);
if (submittedContextStillCurrent) {
clearChatDraftIfUnchanged(id, submittedDraft);
advanceChatParentIfUnchanged(id, parentId, saved.messageId);
}
bindChatOperationMessage(id, requestedModelId, saved.messageId, generate ? 'generating' : 'saved-only');
appendMessage(id, saved);
if (submittedContextStillCurrent) void scrollToLatest(id, true);
if (generate) await requestCompletion(id, saved.messageId, requestedModelId);
else if (id === activeChatId) void refreshHistory(id);
} finally {
finishChatOperation(id);
}
}

async function sendMessage(event: SubmitEvent) {
event.preventDefault();
await saveMessage(true);
}

function saveOnlyMessage() {
void saveMessage(false);
}

function onComposerKeydown(event: KeyboardEvent) {
if (event.key !== 'Enter' || event.shiftKey || event.isComposing || event.keyCode === 229) return;
event.preventDefault();
(event.currentTarget as HTMLTextAreaElement).form?.requestSubmit();
}

async function generateForMessage(userMessageId: string) {
const id = activeChatId;
const requestedModelId = modelId;
if (!completionConfigured || !requestedModelId || saving) return;
chooseParent(userMessageId);
if (!beginChatOperation(id, userMessageId, requestedModelId, 'generating')) return;
try {
await requestCompletion(id, userMessageId, requestedModelId);
} finally {
finishChatOperation(id);
}
}

async function retryCompletion(operation: ChatOperation) {
if (!operation.userMessageId || !operation.modelId || saving) return;
if (!beginChatOperation(operation.chatId, operation.userMessageId, operation.modelId, 'generating')) return;
try {
await requestCompletion(operation.chatId, operation.userMessageId, operation.modelId);
} finally {
finishChatOperation(operation.chatId);
}
}

async function refreshOperationHistory(operation: ChatOperation) {
if (!operation.userMessageId) return;
try {
const next = parseChatHistory(await getJson<unknown>(`/api/chats/${encodeURIComponent(operation.chatId)}`));
const assistant = assistantReply(next.messages, operation.userMessageId);
if (assistant) acceptAssistant(operation.chatId, operation.userMessageId, assistant, next);
else if (operation.chatId === activeChatId) applyHistoryRecord(operation.chatId, next);
} catch (cause) {
const detail = cause instanceof Error ? cause.message : '알 수 없는 오류';
updateChatOperation(operation.chatId, operation.userMessageId, { error: `기록을 새로고침하지 못했습니다. ${detail}` });
}
}

function dismissMessageActions(event: MouseEvent | FocusEvent | KeyboardEvent) {
if (event instanceof KeyboardEvent && event.key !== 'Escape') return;
for (const menu of document.querySelectorAll<HTMLDetailsElement>('details.message-actions[open]')) {
if (event instanceof KeyboardEvent) {
if (!menu.contains(document.activeElement)) continue;
event.preventDefault();
menu.open = false;
menu.querySelector('summary')?.focus();
} else if (!menu.contains(event.target as Node | null)) menu.open = false;
}
}

function toggleDrawer() {
ui.drawerOpen = true;
}
</script>

<svelte:head>
<title>{history ? chatDisplayTitle(history, history.messages, {}, $chatTitleOverrides) : '새 채팅'} · Postudio</title>
</svelte:head>

<svelte:window onfocus={refreshCurrent} onclick={dismissMessageActions} onfocusin={dismissMessageActions} onkeydown={dismissMessageActions} />

{#snippet branchPicker(messages: ChatMessage[], selectedId: string | undefined, label: string)}
<div class="branch-picker my-1 flex min-h-11 max-w-full items-center justify-between gap-3 border-l-2 border-[#d0d0d4] pl-2 text-neutral-600 dark:text-neutral-300" aria-label={label + ' 탐색'}>
<div class="flex min-w-0 items-center gap-1">
<button type="button" aria-label={'이전 ' + label} disabled={messages.length < 2} class="flex size-11 shrink-0 items-center justify-center rounded-lg hover:bg-neutral-100 focus-visible:outline-2 focus-visible:outline-sky-700 disabled:opacity-30 dark:hover:bg-neutral-800" onclick={() => moveSibling(messages, selectedId, -1)}><Icon name="chevron" class="size-3 rotate-180" /></button>
<span class="min-w-[72px] text-center text-[13px] font-semibold tabular-nums">{Math.max(0, messages.findIndex((message) => message.messageId === selectedId) + 1)} / {messages.length}</span>
<button type="button" aria-label={'다음 ' + label} disabled={messages.length < 2} class="flex size-11 shrink-0 items-center justify-center rounded-lg hover:bg-neutral-100 focus-visible:outline-2 focus-visible:outline-sky-700 disabled:opacity-30 dark:hover:bg-neutral-800" onclick={() => moveSibling(messages, selectedId, 1)}><Icon name="chevron" class="size-3" /></button>
</div>
<details class="message-actions relative shrink-0">
<summary aria-label={label + ' 목록 열기'} class="flex min-h-10 cursor-pointer list-none items-center gap-2 rounded-lg px-3 text-xs hover:bg-neutral-100 focus-visible:outline-2 focus-visible:outline-sky-700 dark:hover:bg-neutral-800"><Icon name="chevron" class="size-3 rotate-90" />목록</summary>
<div class="absolute right-0 top-10 z-20 max-h-56 w-72 max-w-[min(80vw,22rem)] overflow-y-auto rounded-[14px] border border-neutral-200 bg-neutral-50 p-1.5 shadow-lg dark:border-neutral-700 dark:bg-neutral-900">
{#each messages as message, index (message.messageId)}
<button type="button" aria-pressed={selectedId === message.messageId} aria-current={selectedId === message.messageId ? 'true' : undefined} class="flex min-h-11 w-full items-center gap-3 rounded-[10px] px-2 text-left text-[13px] hover:bg-neutral-100 focus-visible:outline-2 focus-visible:outline-sky-700 aria-pressed:bg-neutral-100 dark:hover:bg-neutral-800 dark:aria-pressed:bg-neutral-800" onclick={(event) => { selectBranch(message); const menu = event.currentTarget.closest('details'); if (menu) { menu.open = false; menu.querySelector('summary')?.focus(); } }}><span class="flex size-[26px] shrink-0 items-center justify-center rounded-lg bg-neutral-200 text-xs font-semibold dark:bg-neutral-700">{index + 1}</span><span class="truncate">{chatMessagePreview(message)}</span></button>
{/each}
</div>
</details>
</div>
{/snippet}

<div class="flex min-h-0 min-w-0 flex-1 flex-col">
<header class="flex h-14 shrink-0 items-center justify-between gap-3 border-b border-neutral-200 bg-white px-4 pt-[env(safe-area-inset-top)] dark:border-neutral-800 dark:bg-neutral-950 sm:px-6">
<div class="flex min-w-0 items-center gap-2">
<button type="button" class="flex size-9 shrink-0 items-center justify-center rounded-lg hover:bg-neutral-100 lg:hidden" aria-label="메뉴 열기" onclick={toggleDrawer}><Icon name="menu" class="size-4" /></button>
{#if ui.sidebarHidden}<button type="button" class="hidden size-9 shrink-0 items-center justify-center rounded-lg hover:bg-neutral-100 lg:flex" aria-label="사이드바 펼치기" onclick={() => ui.toggleSidebar()}><Icon name="sidebar" class="size-4" /></button>{/if}
<span aria-hidden="true" class="hidden size-6 shrink-0 items-center justify-center rounded-full bg-neutral-900 text-xs text-white sm:flex">✦</span>
<div class="min-w-0 max-w-64"><ModelSelect id="chat-response-model" bind:value={modelId} label="응답 모델" compact initialFromCatalog disabled={saving} /></div>
</div>
<div class="flex min-w-0 items-center gap-2">
{#if titleEditing && history}
<form class="flex min-w-0 items-center gap-1" onsubmit={(event) => { event.preventDefault(); void saveChatTitle(); }}>
<input bind:this={titleInput} bind:value={titleDraft} aria-label="채팅 제목" aria-describedby={titleError ? 'chat-title-error' : undefined} maxlength="300" class="min-w-0 w-36 rounded-lg border border-neutral-300 bg-white px-2 py-1.5 text-sm outline-none focus:border-sky-600 focus:ring-2 focus:ring-sky-200 sm:w-56 dark:border-neutral-700 dark:bg-neutral-900" disabled={titleSaving} onkeydown={handleTitleKeydown} />
<button type="submit" class="min-h-9 rounded-lg px-2 text-xs font-medium text-sky-700 hover:bg-sky-50 disabled:opacity-50 dark:text-sky-300 dark:hover:bg-sky-950" disabled={titleSaving}>저장</button>
<button type="button" class="min-h-9 rounded-lg px-2 text-xs text-neutral-600 hover:bg-neutral-100 disabled:opacity-50 dark:text-neutral-300 dark:hover:bg-neutral-800" disabled={titleSaving} onclick={cancelTitleEdit}>취소</button>
</form>
{:else}
<h1 class="min-w-0 max-w-[48vw] truncate text-sm font-medium text-neutral-800 dark:text-neutral-100">{history ? chatDisplayTitle(history, history.messages, {}, $chatTitleOverrides) : '새 채팅'}</h1>
{#if history}<button type="button" class="flex size-9 shrink-0 items-center justify-center rounded-lg text-neutral-600 hover:bg-neutral-100 focus-visible:outline-2 focus-visible:outline-sky-700 dark:text-neutral-300 dark:hover:bg-neutral-800" aria-label="채팅 제목 수정" title="채팅 제목 수정" onclick={beginTitleEdit}><Icon name="edit" class="size-4" /></button>{/if}
{/if}
<details class="relative shrink-0">
<summary class="flex size-9 cursor-pointer list-none items-center justify-center rounded-full text-neutral-600 hover:bg-neutral-100 focus-visible:outline-2 focus-visible:outline-neutral-500 dark:hover:bg-neutral-800" aria-label="대화 옵션"><Icon name="sliders" /></summary>
<div class="absolute right-0 top-10 z-20 w-72 rounded-xl border border-neutral-200 bg-white p-3 text-xs text-neutral-700 shadow-lg dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-200">
<div class="flex flex-col gap-2"><button type="button" class="min-h-9 text-left underline underline-offset-4" onclick={refreshCurrent}>대화 기록 새로고침</button>
{#if capabilityLoading}<span role="status">채팅 기능 상태 확인 중</span>{:else if capabilityError}<span role="alert">기능 상태 확인 실패 · {capabilityError}</span>{:else if capabilities}<span>AI 응답 {capabilities.completion.configured ? '사용 가능' : '사용 불가'} · 서버 간 동기화 {capabilities.sync.available ? `가능 · 피어 ${capabilities.sync.peerCount}` : '사용 불가'}</span>{/if}
<button type="button" class="min-h-9 text-left underline underline-offset-4" onclick={refreshCapabilities}>기능 상태 새로고침</button>
{#if capabilities?.sync.lastError}<span role="alert">최근 동기화 오류 · {capabilities.sync.lastError}</span>{/if}
</div></div>
</details>
</div>
</header>
{#if titleError}<p id="chat-title-error" role="alert" class="mx-auto w-full max-w-3xl px-4 pt-2 text-sm text-red-700 dark:text-red-300">{titleError}</p>{/if}

{#if notFound}
<section class="mx-auto flex w-full max-w-3xl flex-1 flex-col items-center justify-center gap-3 px-4 text-center">
<h2 class="text-lg font-semibold">채팅을 찾을 수 없습니다.</h2>
<p class="text-sm text-neutral-600 dark:text-neutral-300">채팅이 삭제되었거나 주소가 올바르지 않습니다.</p>
<a href="/c" class="min-h-11 content-center rounded-xl border border-neutral-300 px-4 text-sm font-medium hover:bg-neutral-100 dark:border-neutral-700 dark:hover:bg-neutral-800">채팅 목록으로</a>
</section>
{:else}
{#if historyError}
<div role="alert" class="mx-auto mt-3 flex w-full max-w-3xl flex-wrap items-center justify-between gap-2 px-4 text-sm text-red-700 dark:text-red-300">
<span>{historyError}</span>
<button type="button" class="min-h-10 rounded-lg px-3 font-medium underline underline-offset-4" onclick={refreshCurrent}>다시 시도</button>
</div>
{/if}

{#if loading && !history}
<div class="mx-auto w-full max-w-3xl px-4 py-6"><HistorySkeleton variant="chat" /></div>
{:else if history}
<div bind:this={messageScroller} onscroll={() => { if (messageScroller) if (messageScroller) followLatest = isNearBottom(messageScroller); }} class="min-h-0 flex-1 overflow-y-auto">
<div bind:this={messageContent} class="mx-auto flex w-full max-w-[808px] flex-col gap-8 px-4 py-6 sm:px-6">
{#if hasOlderMessages || olderLoading || olderError}
<div class="flex flex-col items-center gap-2">
<button type="button" class="min-h-11 rounded-full border border-neutral-300 px-4 text-sm font-medium hover:bg-neutral-100 focus-visible:outline-2 focus-visible:outline-sky-700 disabled:opacity-50 dark:border-neutral-700 dark:hover:bg-neutral-800" disabled={!hasOlderMessages || olderLoading} onclick={loadOlderHistory}>
{olderLoading ? '이전 기록을 불러오는 중입니다.' : olderError ? '이전 기록 다시 불러오기' : '이전 메시지 불러오기'}
</button>
{#if olderError}<p role="alert" class="text-center text-sm text-red-700 dark:text-red-300">{olderError}</p>{/if}
</div>
{/if}
{#if selectedParentMissing}
<p role="note" class="text-center text-xs text-amber-800 dark:text-amber-200">저장된 부모 메시지가 아직 불러온 기록에 없습니다. 이전 기록을 불러와 분기 연결을 복구하세요.</p>
{:else if ancestry.length > 0 && ancestry[0].parentId !== null}
<p role="note" class="text-center text-xs text-neutral-600 dark:text-neutral-300">이 대화 위쪽에 이전 부모 메시지가 더 있습니다. 분기 흐름은 이전 기록을 불러오면 이어집니다.</p>
{/if}
{#if roots.length > 1}
{@render branchPicker(roots, ancestry[0]?.messageId, '시작 분기')}
{/if}

{#if ancestry.length === 0 && !selectedLeafId}
<section class="flex flex-col items-center gap-4 py-12 text-center">
<h2 class="text-[28px] font-semibold tracking-tight">무엇을 도와드릴까요?</h2>
<div class="flex flex-wrap justify-center gap-2">
{#each ['오늘 메모 저장', '아이디어 정리하기', '할 일 메모하기'] as example (example)}
<button type="button" class="min-h-10 rounded-full border border-neutral-300 px-4 text-sm hover:bg-neutral-100 focus-visible:outline-2 focus-visible:outline-neutral-500 dark:border-neutral-700 dark:hover:bg-neutral-800" onclick={() => { setChatDraft(activeChatId, example + '\n'); document.getElementById('chat-message')?.focus(); }}>{example}</button>
{/each}
</div>
</section>
{/if}

{#each ancestry as message, index (message.messageId)}
{@const children = chatChildren(history.messages, message.messageId)}
{@const selectedChildIndex = children.findIndex((child) => child.messageId === ancestry[index + 1]?.messageId)}
{@const operation = $chatOperationStore[chatOperationKey(activeChatId, message.messageId)]}
{@const hasAssistant = children.some((child) => child.role === 'assistant')}
<article class={['group relative max-w-full py-0', message.role === 'user' ? 'ml-auto w-full' : 'mr-auto w-full pl-11 text-neutral-900 dark:text-neutral-100', enteringMessageId === message.messageId && 'chat-message-enter']}>
{#if message.role === 'assistant'}<span aria-hidden="true" class="absolute left-0 top-1 flex size-8 items-center justify-center rounded-full border border-neutral-200 text-sm">✦</span>{/if}
{#if message.role === 'assistant'}
<header class="mb-2 text-[13px] text-neutral-600 dark:text-neutral-300"><span class="font-medium">{message.model || 'AI 응답'}</span></header>
{/if}
<div class={message.role === 'user' ? "ml-auto w-fit max-w-[80%] rounded-[20px] bg-[#f0f0f2] px-[18px] py-[11px] text-[15.5px] leading-[1.65] text-neutral-900 dark:bg-neutral-800 dark:text-neutral-100" : "break-words text-[15.5px] leading-[1.75]"}><Markdown text={message.text} lineHeight="inherit" /></div>
{#if message.role === 'user'}
{#if operation?.status === 'observing'}
<p role="status" class="mt-2 inline-flex items-center gap-2 text-xs text-neutral-700 dark:text-neutral-300"><span class="pending-dots" aria-hidden="true"><span></span><span></span><span></span></span>이미 시작된 요청 결과를 기록에서 확인 중입니다. 요청을 다시 보내지 않습니다.</p>
{:else if operation?.status === 'pending'}
<div class="mt-2 flex flex-wrap items-center gap-2 text-xs text-amber-800 dark:text-amber-200">
<span>{operation.error || '응답이 아직 기록되지 않았습니다.'} 재요청하지 않았습니다.</span>
<button type="button" class="min-h-8 underline underline-offset-4" onclick={() => refreshOperationHistory(operation)}>기록 새로고침</button>
</div>
{:else if operation?.status === 'unknown'}
<div role="alert" class="mt-2 flex flex-wrap items-center gap-2 text-xs text-amber-800 dark:text-amber-200">
<span>{operation.error || '응답 요청 결과를 확인할 수 없습니다.'} 기록만 새로고침하세요.</span>
<button type="button" class="min-h-8 underline underline-offset-4" onclick={() => refreshOperationHistory(operation)}>기록 새로고침</button>
</div>
{:else if operation?.status === 'failed'}
<div role="alert" class="mt-2 flex flex-wrap items-center gap-2 text-xs text-red-700 dark:text-red-300">
<span>AI 응답 생성 실패 · {operation.error}</span>
<button type="button" class="min-h-8 underline underline-offset-4" disabled={saving} onclick={() => retryCompletion(operation)}>같은 저장 메시지와 {operation.modelId} 모델로 다시 시도</button>
</div>
{:else if operation?.status === 'completed'}
<p role="status" class="sr-only">AI 응답 완료</p>
{:else if operation?.status === 'saved-only'}
<p class="mt-2 text-xs text-neutral-700 dark:text-neutral-300">메시지만 저장했습니다. AI 응답은 요청하지 않았습니다.</p>
{:else if !operation && !hasAssistant && capabilities && !completionConfigured}
<p class="mt-2 text-xs text-neutral-600 dark:text-neutral-300">이 메시지에 AI 응답이 없습니다. 서버에서 응답 생성을 사용할 수 없습니다.</p>
{/if}
{/if}
<div class="message-action-chips flex flex-wrap items-center gap-x-1 gap-y-1 text-[12.5px] text-neutral-600 dark:text-neutral-400" class:justify-end={message.role === 'user'}>
{#if message.role === 'user'}
<button type="button" aria-label="수정" disabled={saving || Boolean(message.parentId && !history.messages.some((parent) => parent.messageId === message.parentId))} title={message.parentId && !history.messages.some((parent) => parent.messageId === message.parentId) ? '이전 부모 메시지를 불러온 뒤 수정할 수 있습니다.' : undefined} class="inline-flex min-h-11 items-center gap-1.5 rounded-lg px-2 hover:bg-neutral-100 focus-visible:outline-2 focus-visible:outline-sky-700 disabled:opacity-40 dark:hover:bg-neutral-800" onclick={() => editMessage(message)}><Icon name="edit" class="size-3.5" /><span>수정</span></button>
{#if message.parentId && !history.messages.some((parent) => parent.messageId === message.parentId)}<span class="self-center text-xs text-amber-800 dark:text-amber-200">수정하려면 이전 부모 메시지를 불러오세요.</span>{/if}
{/if}
<button type="button" aria-label="여기서 포크하기" disabled={saving} class="inline-flex min-h-11 items-center gap-1.5 rounded-lg px-2 hover:bg-neutral-100 focus-visible:outline-2 focus-visible:outline-sky-700 disabled:opacity-40 dark:hover:bg-neutral-800" onclick={() => forkFromMessage(message)}><Icon name="fork" class="size-3.5" /><span>여기서 포크하기</span></button>
</div>
{#if message.role === 'user' && !hasAssistant && completionConfigured && modelId && (!operation || operation.status === 'saved-only')}
<details class="message-actions absolute right-0 top-full z-10 w-fit text-neutral-500">
<summary aria-label="추가 생성 작업" class="flex size-8 cursor-pointer list-none items-center justify-center rounded-md text-lg hover:bg-neutral-100 focus-visible:outline-2 focus-visible:outline-sky-700 dark:hover:bg-neutral-800">⋯</summary>
<div class="absolute right-0 top-8 z-20 w-52 rounded-xl border border-neutral-200 bg-white p-1 shadow-sm dark:border-neutral-700 dark:bg-neutral-900">
<button type="button" class="min-h-10 w-full rounded-lg px-3 text-left text-xs text-neutral-700 hover:bg-neutral-100 disabled:opacity-50 dark:text-neutral-200 dark:hover:bg-neutral-800" disabled={!modelId || saving} onclick={(event) => { const menu = event.currentTarget.closest('details'); if (menu) { menu.open = false; menu.querySelector('summary')?.focus(); } void generateForMessage(message.messageId); }}>AI 응답 생성</button>
</div>
</details>
{/if}
</article>
{#if message.role === 'user' && operation?.status === 'generating' && !hasAssistant}
<article class="group relative mr-auto w-full max-w-full pl-11 text-neutral-900 dark:text-neutral-100 chat-message-enter" aria-label="AI 응답 생성 중">
<span aria-hidden="true" class="absolute left-0 top-1 flex size-8 items-center justify-center rounded-full border border-neutral-200 text-sm">✦</span>
<header class="mb-2 text-[13px] text-neutral-600 dark:text-neutral-300"><span class="font-medium">{operation.modelId}</span></header>
{#if operation.progress?.text}
<div class="break-words text-[15.5px] leading-7" aria-live="polite"><Markdown text={operation.progress.text} />{#if operation.progress.status === 'generating'}<span class="stream-caret" aria-hidden="true"></span>{/if}</div>
{#if operation.progress.status === 'failed'}<p role="status" class="mt-2 text-xs text-red-700 dark:text-red-300">응답 생성 실패 · 요청 결과 확인 중입니다.</p>{:else if operation.progress.status === 'completed'}<p role="status" class="mt-2 text-xs text-neutral-600 dark:text-neutral-300">응답 완료 · 저장된 답변 확인 중입니다.</p>{/if}
{:else if operation.progress?.status === 'failed'}
<p role="status" class="mt-2 inline-flex items-center gap-2 text-xs text-red-700 dark:text-red-300">응답 생성 실패 · 요청 결과 확인 중입니다.</p>
{:else}
<p role="status" class="mt-2 inline-flex items-center gap-2 text-xs text-neutral-700 dark:text-neutral-300"><span class="pending-dots" aria-hidden="true"><span></span><span></span><span></span></span>{operation.modelId} 모델로 AI 응답 생성 중입니다.</p>
{/if}
</article>
{/if}
{#if children.length > 1 || (children.length === 1 && selectedChildIndex < 0)}
{@render branchPicker(children, ancestry[index + 1]?.messageId, '분기')}
{/if}
{/each}
</div>
</div>
{/if}

{#if history}
<footer class="mx-auto w-full max-w-[808px] shrink-0 bg-white px-4 pb-3 pt-2 sm:px-6 dark:bg-neutral-950">
{#if selectedLeafId}
{@const selectedParent = history.messages.find((message) => message.messageId === selectedLeafId)}
<div class="sr-only" aria-live="polite">
<span class="min-w-0 flex-1 truncate">부모 메시지 · {selectedParent ? chatMessagePreview(selectedParent) : '이전 부모 메시지를 불러오세요'}</span>
</div>
{:else}
<p class="sr-only">새 시작 분기로 저장합니다.</p>
{/if}
{#if selectedParentMissing}
<p role="status" class="mb-2 flex flex-wrap items-center justify-between gap-2 text-sm text-amber-800 dark:text-amber-200">
<span>선택한 부모 메시지가 기록에 없습니다. 부모를 불러올 때까지 메시지 저장과 수정은 잠깁니다.</span>
{#if hasOlderMessages}
<button type="button" class="min-h-10 rounded-lg px-3 font-medium underline underline-offset-4" disabled={olderLoading} onclick={loadOlderHistory}>{olderError ? '이전 기록 다시 불러오기' : '이전 기록 불러오기'}</button>
{:else}
<button type="button" class="min-h-10 rounded-lg px-3 font-medium underline underline-offset-4" onclick={refreshCurrent}>최신 기록에서 이전 부모 다시 찾기</button>
{/if}
</p>
{/if}
{#if saveError}
<p role="alert" class="mb-2 text-sm text-red-700 dark:text-red-300">{saveError}</p>
{/if}
{#if $chatComposerModeStore[activeChatId]}
{@const mode = $chatComposerModeStore[activeChatId]}
{@const target = history.messages.find((message) => message.messageId === mode.messageId)}
<div role="status" class="mb-2 flex min-h-11 items-center justify-between gap-3 rounded-[14px] bg-[#f0f0f2] px-3 text-sm text-neutral-800 dark:bg-neutral-800 dark:text-neutral-100">
<div class="flex min-w-0 items-center gap-2">
{#if mode.kind === 'fork'}<Icon name="fork" class="size-4 shrink-0" />{/if}
<span class="shrink-0">{#if mode.kind === 'edit'}메시지 수정 중{:else}선택한 메시지에서 이어쓰기{/if}</span>
<span class="max-w-[min(45vw,28rem)] truncate rounded-full border border-neutral-300 px-3 py-1 text-xs dark:border-neutral-600">{target ? chatMessagePreview(target) || '원본 메시지' : '선택한 메시지'}</span>
</div>
<button type="button" class="min-h-11 shrink-0 rounded-lg px-3 font-medium hover:bg-white focus-visible:outline-2 focus-visible:outline-sky-700 dark:hover:bg-neutral-700" onclick={cancelComposerMode}>취소</button>
</div>
{/if}
<form class="flex flex-col gap-1 rounded-[26px] border border-neutral-300 bg-white pb-[10px] pl-5 pr-[14px] pt-[14px] shadow-[0_2px_10px_rgba(20,20,30,0.06)] focus-within:border-neutral-500 dark:border-neutral-700 dark:bg-neutral-900" onsubmit={sendMessage}>
<label for="chat-message" class="sr-only">메시지</label>
<textarea
 id="chat-message"
 value={draft}
 oninput={updateDraft}
 onkeydown={onComposerKeydown}
 disabled={saving || selectedParentMissing}
 rows="2"
 placeholder={selectedParentMissing ? '선택한 부모 메시지를 불러온 뒤 작성할 수 있습니다.' : saving ? '저장 또는 응답 생성 중입니다…' : '메시지를 작성하세요. Enter 전송 · Shift+Enter 줄 바꿈'}
 class="max-h-48 w-full resize-none rounded-xl bg-transparent p-0 text-[15.5px] leading-[1.6] outline-none placeholder:text-neutral-600 dark:placeholder:text-neutral-300 disabled:opacity-60"
></textarea>
<div class="flex items-center justify-between gap-2 px-1">
<div class="flex min-w-0 flex-1 items-center gap-2">
<details class="relative">
<summary aria-label="추가 메시지 작업" class="flex size-9 cursor-pointer list-none items-center justify-center rounded-full border border-neutral-200 text-lg hover:bg-neutral-100 focus-visible:outline-2 focus-visible:outline-neutral-500 dark:border-neutral-700 dark:hover:bg-neutral-800">+</summary>
<div class="absolute bottom-11 left-0 z-20 w-64 max-w-[80vw] rounded-[14px] border border-neutral-200 bg-neutral-50 p-1.5 shadow-lg dark:border-neutral-700 dark:bg-neutral-900">
<button type="button" disabled={saving || selectedParentMissing || !draft.trim()} onclick={saveOnlyMessage} class="min-h-11 w-full rounded-[10px] px-3 text-left text-sm hover:bg-neutral-100 disabled:opacity-40 dark:hover:bg-neutral-800">메시지만 저장</button>
{#if selectedLeafId}<button type="button" class="min-h-11 w-full rounded-[10px] px-3 text-left text-sm hover:bg-neutral-100 dark:hover:bg-neutral-800" onclick={clearParent}>새 시작 분기</button>{/if}
</div></details>
<p class="text-xs text-neutral-500" class:sr-only={!saving && !capabilityLoading && !capabilityError && completionConfigured}>
{#if saving}요청 처리 중입니다. 중복 전송은 막혀 있습니다.
{:else if capabilityLoading}기능 상태 확인 중입니다. AI 요청은 상태 확인 뒤에 활성화됩니다.
{:else if capabilityError}AI 상태를 확인할 수 없습니다. 저장만 하거나 상태를 다시 확인하세요.
{:else if capabilities?.completion.configured}보내기는 메시지를 한 번 저장한 뒤 선택한 모델에 AI 응답을 요청합니다.
{:else}AI 응답을 사용할 수 없습니다. 저장만 하거나 서버 기능 상태를 다시 확인하세요.
{/if}
</p>
</div>
<div class="flex items-center gap-2">
<button
 type="submit"
 disabled={saving || selectedParentMissing || !draft.trim() || !completionConfigured || !modelId}
 class="flex size-9 items-center justify-center rounded-full bg-black text-sm font-medium text-white hover:bg-neutral-800 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-neutral-500 disabled:opacity-40 dark:bg-white dark:text-black"
>
{#if saving}<span class="sr-only">생성 중</span>{:else if capabilityLoading}<span class="sr-only">상태 확인 중</span>{:else if capabilityError}<span class="sr-only">AI 상태 확인 필요</span>{:else if !completionConfigured}<span class="sr-only">AI 응답 사용 불가</span>{:else if !modelId}<span class="sr-only">모델 선택 필요</span>{:else}<span class="sr-only">전송</span>{/if}<Icon name="chevron" class="size-4 -rotate-90" />
</button>
</div>
</div>
</form>
<p class="mt-2 text-center text-[12px] text-neutral-600 dark:text-neutral-300">AI는 실수할 수 있어요. 중요한 내용은 다시 확인하세요.</p>
</footer>
{/if}
{/if}
</div>

<style>
.chat-message-enter { animation: chat-message-enter 0.3s ease both; }

@keyframes chat-message-enter {
from { opacity: 0; transform: translateY(6px); }
to { opacity: 1; transform: none; }
}

.pending-dots { display: inline-flex; align-items: center; gap: 4px; }
.pending-dots > span {
width: 6px;
height: 6px;
border-radius: 9999px;
background: currentColor;
animation: pending-dot 1.1s ease-in-out infinite;
}
.pending-dots > span:nth-child(2) { animation-delay: 0.15s; }
.pending-dots > span:nth-child(3) { animation-delay: 0.3s; }

@keyframes pending-dot {
0%, 80%, 100% { opacity: 0.25; transform: translateY(0); }
40% { opacity: 1; transform: translateY(-3px); }
}

.stream-caret {
display: inline-block;
width: 1.5px;
height: 1em;
margin-left: 2px;
vertical-align: -0.12em;
background: currentColor;
animation: stream-caret 1s steps(2, end) infinite;
}

@keyframes stream-caret { to { visibility: hidden; } }

@media (hover: hover) and (pointer: fine) {
article .message-action-chips, article .message-actions { opacity: 0; pointer-events: none; }
article:hover .message-action-chips, article:focus-within .message-action-chips,
article:hover .message-actions, article:focus-within .message-actions,
article .message-actions:focus-within, article .message-actions[open] { opacity: 1; pointer-events: auto; }
}

@media (prefers-reduced-motion: reduce) {
.chat-message-enter, .pending-dots > span, .stream-caret { animation: none; }
}
</style>
