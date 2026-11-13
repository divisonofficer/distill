<script lang="ts">
import { tick, untrack } from 'svelte';
import { onDestroy } from 'svelte';
import { connect, getJson, type Socket } from '$lib/api';
import AttachmentPreview from '$lib/components/AttachmentPreview.svelte';
import Icon from '$lib/components/Icon.svelte';
import InteractionCard from '$lib/components/InteractionCard.svelte';
import Markdown from '$lib/components/Markdown.svelte';
import ComposerRouting from '$lib/components/ComposerRouting.svelte';
import ToolRow from '$lib/components/ToolRow.svelte';
import HistorySkeleton from '$lib/components/HistorySkeleton.svelte';
import { modeFromRow } from '$lib/types';
import { MODE_LABEL, MODES } from '$lib/prefs';
import { sessionTitle } from '$lib/format';
import { ui } from '$lib/ui.svelte';
import { workspaceDisplayName } from '$lib/workspaces';
import { SessionReplay, SessionView, groupCompletedTools, restoredHistoryScrollTop, type ToolItem } from '$lib/session.svelte';
import {
	clipboardFiles,
	createTextDraft,
	isLongPaste,
	MAX_ATTACHMENTS,
	nextPastedTextName,
	readAttachmentFiles,
mergeDrafts,
	type DraftAttachment
} from '$lib/attachments';
import { permissionActions, permissionOptionLabel, permissionPromptState } from '$lib/permissions';
import { TaskHistoryScroll, type HistoryReplayMode, type HistoryScrollTarget, type ScrollMetrics } from '$lib/task-history-scroll';
import type { ClientMsg, InteractionRequest, Mode, Sep } from '$lib/types';

let { sessionId, host, blocked = false, initialDraft, onDraftChange, onTitle, onTools, onOpenTool, onPending, onControlChange, runningAgentCount = 0 }: { sessionId: string; host: string; blocked?: boolean; initialDraft?: { text: string; attachments: DraftAttachment[] }; onDraftChange?: (draft: { text: string; attachments: DraftAttachment[] }) => void; onTitle?: (title: string) => void; onTools?: (tools: ToolItem[]) => void; onOpenTool?: (tool: ToolItem) => void; onPending?: (pending: boolean) => void; onControlChange?: (control: { connected: boolean; running: boolean; cancel: () => boolean } | null) => void; runningAgentCount?: number } = $props();
const id = $derived(sessionId);
const composerId = $derived(`composer-${sessionId}`);
const modelId = $derived(`session-model-${sessionId}`);
const pendingId = $derived(`pending-interactions-${sessionId}`);
const sessionHost = $derived(host);
const hostError = $derived(blocked ? '호스트 주소가 설정 목록에 없거나 유효하지 않습니다. 설정에서 호스트를 확인하세요.' : '');
let destroyed = false;
onDestroy(() => {
	const callback = onDraftChange;
	if (callback) callback({ text, attachments: [...attachments] });
	if (onControlChange) untrack(() => onControlChange?.(null));
	destroyed = true;
});
const view = new SessionView();
let interactions = $state<InteractionRequest[]>([]);
let pinnedRow = $state<import('$lib/types').BoardRow | undefined>();
let metadataError = $state('');
let metadataTimer: ReturnType<typeof setInterval> | undefined;
let metadataGeneration = 0;
const row = $derived.by(() => {
	return pinnedRow;
});
const title = $derived(view.title || (row ? sessionTitle(row) : id));
const hostAddress = $derived(new URL(sessionHost).host);
const activePending = $derived(permissionPromptState(view.permissions.length, interactions.length));
const tools = $derived.by(() => { void view.revision; return view.items.filter((item): item is ToolItem => item.kind === 'tool'); });
const runningCommandCount = $derived(tools.filter((item) => item.status === 'in_progress' && ((item.toolKind ?? '').toLowerCase() === 'execute' || /(^|[_:/-])(bash|shell|execute|exec|terminal)([_:/-]|$)|(^|_)run_(?:terminal_)?command([_:/-]|$)/i.test(item.name ?? ''))).length);
$effect(() => { const value = title; if (onTitle) untrack(() => onTitle(value)); });
$effect(() => { const value = tools; if (onTools) untrack(() => onTools(value)); });
$effect(() => { const value = activePending; if (onPending) untrack(() => onPending(value)); });
$effect(() => { const connected = view.connected; const running = view.running; untrack(() => onControlChange?.({ connected, running, cancel: () => view.connected && view.running && !hostError && send({ t: 'cancel' }) })); });

function cancelCurrent() {
if (!view.connected || !view.running) return;
const sent = send({ t: 'cancel' });
ui.notify(`${title}: ${sent ? '중지 요청을 보냈습니다.' : '중지 요청을 보내지 못했습니다.'}`, sent ? 'success' : 'error');
}

$effect(() => {
	const sessionKey = `${id}\u0000${sessionHost ?? ''}`;
	const generation = ++metadataGeneration;
	pinnedRow = undefined;
	metadataError = '';
	if (metadataTimer !== undefined) clearInterval(metadataTimer);
	if (hostError || !id) return;
	const refreshMetadata = async () => {
		try {
			const result = await getJson<{ sessions: import('$lib/types').BoardRow[] }>('/api/sessions', sessionHost);
			if (generation !== metadataGeneration) return;
			pinnedRow = result.sessions.find((item) => item.sessionId === id);
			metadataError = '';
		} catch {
			if (generation === metadataGeneration) metadataError = '세션 호스트의 작업 정보를 불러오지 못했습니다.';
		}
	};
	void refreshMetadata();
	metadataTimer = setInterval(() => void refreshMetadata(), 2000);
	return () => { if (metadataTimer !== undefined) clearInterval(metadataTimer); };
});

const mode = $derived<Mode | undefined>(modeFromRow(row));
const modeLabel = $derived(mode === undefined ? '확인 중' : MODE_LABEL[mode]);
let modePending = $state<{ id: string; mode: Mode } | null>(null);
let modeError = $state<{ id: string; message: string } | null>(null);
let modeTimer: ReturnType<typeof setTimeout> | undefined;
let modeRequest = 0;
let modelPending = $state<{ id: string; requested: string } | null>(null);
let modelError = $state<{ id: string; message: string } | null>(null);
let modelTimer: ReturnType<typeof setTimeout> | undefined;
let modelRequest = 0;

$effect(() => {
const pending = modePending;
if (pending?.id === id && modeFromRow(row) === pending.mode) {
ui.notify(`${title}: 권한 모드가 ${MODE_LABEL[pending.mode]}(으)로 변경되었습니다.`);
modeRequest++;
if (modeTimer !== undefined) clearTimeout(modeTimer);
modeTimer = undefined;
modePending = null;
modeError = null;
}
});

$effect(() => {
const pending = modelPending;
if (pending?.id === id && view.modelId === pending.requested) {
ui.notify(`${title}: 모델이 변경되었습니다.`);
modelRequest++;
if (modelTimer !== undefined) clearTimeout(modelTimer);
modelTimer = undefined;
modelPending = null;
modelError = null;
}
});

// One socket per session view: the effect closes the previous socket before opening the next.
let sock: Socket | undefined;
let historySessionKey = '';
let historyReplay: SessionReplay | undefined;
let historyReplayGeneration = 0;
let historyReplayFollow = false;
let historyReady = $state(false);
const timeline = $derived.by(() => { void view.revision; return historyReady ? groupCompletedTools(view.items) : []; });
let historyCursor = $state<string | null>(null);
let historyHasMore = $state(false);
let historyLoading = $state(false);
let historyError = $state('');
let historyAnchor: ScrollMetrics | undefined;
const historyScroll = new TaskHistoryScroll();
$effect(() => {
const sessionKey = `${id}\u0000${sessionHost ?? ''}`;
if (sessionKey !== historySessionKey) {
historySessionKey = sessionKey;
historyReady = false;
historyCursor = null;
historyHasMore = false;
historyLoading = false;
historyError = '';
historyAnchor = undefined;
historyReplay = undefined;
historyReplayGeneration++;
view.reset();
view.setConnected(false);
}
if (hostError) {
view.setConnected(false);
return;
}
let replayMode: HistoryReplayMode = 'initial';
let replayToken = 0;
let replayMetrics: ScrollMetrics = { scrollTop: 0, scrollHeight: 0, clientHeight: 0 };
const s = connect<Sep>(`/ws/session/${id}`, {
onOpen: () => {
replayMetrics = historyMetrics();
replayMode = historyScroll.begin(sessionKey);
replayToken = ++historyReplayGeneration;
historyReplayFollow = replayMode === 'initial' || historyScroll.shouldFollow();
historyReplay = new SessionReplay(view);
historyReplay.begin();
historyReady = false;
historyCursor = null;
historyHasMore = false;
historyLoading = false;
historyError = '';
historyAnchor = undefined;
view.reset();
interactions = [];
if (modelTimer !== undefined) clearTimeout(modelTimer);
modelPending = null;
modelError = null;
view.setConnected(true);
},
onClose: () => {
historyReplayGeneration++;
historyReplay = undefined;
historyReplayFollow = false;
historyReady = false;
historyLoading = false;
historyAnchor = undefined;
sending = false;
clearTimeout(sendTimer);
interactions = [];
view.setConnected(false);
},
onMessage: (m) => {
if (m.t === 'error' && !historyReady) historyError = `대화 기록을 불러오지 못했습니다. ${m.message}`;
if (sending && m.t === 'session.state' && m.status === 'running') {
sending = false;
clearTimeout(sendTimer);
text = '';
attachments = [];
grow();
} else if (sending && m.t === 'error') {
sending = false;
clearTimeout(sendTimer);
}
const replay = historyReplay;
let replayComplete = false;
if (m.t === 'history.page') {
const anchor = m.initial ? historyMetrics() : historyAnchor ?? historyMetrics();
historyAnchor = undefined;
historyLoading = false;
historyCursor = m.cursor;
historyHasMore = m.has_more;
historyError = m.error ?? '';
if (m.initial && replay) replay.apply(m);
else view.applyHistoryPage(m.items, m.initial);
if (!m.initial) {
const token = historyReplayGeneration;
void tick().then(() => {
if (token === historyReplayGeneration && scroller) {
scroller.scrollTo({ top: restoredHistoryScrollTop(anchor, historyMetrics().scrollHeight) });
historyScroll.update(historyMetrics());
}
});
}
} else {
replayComplete = replay ? replay.apply(m) : (view.apply(m), false);
}
if (replayComplete) {
const target = historyScroll.complete(replayMode, replayMetrics);
historyReplay = undefined;
historyReplayFollow = false;
void finishHistoryReplay(replayToken, target);
} else if (isTranscriptUpdate(m)) {
followHistoryUpdate();
}
if (m.t === 'session.model') {
const pending = modelPending;
if (pending?.id === id && m.model_id === pending.requested) ui.notify(`${title}: 모델이 변경되었습니다.`);
if (modelTimer !== undefined) clearTimeout(modelTimer);
modelPending = null;
modelError = null;
} else if (m.t === 'error' && modelPending?.id === id) {
if (modelTimer !== undefined) clearTimeout(modelTimer);
modelPending = null;
modelError = { id, message: `모델 변경 실패: ${m.message}` };
ui.notify(`${title}: 모델 변경에 실패했습니다.`, 'error');
}
applyInteraction(m);
}
}, { host: sessionHost });
sock = s;
return () => {
s.close();
if (sock === s) sock = undefined;
};
});

function send(msg: ClientMsg): boolean {
return sock?.send(msg) ?? false;
}

function applyInteraction(ev: Sep) {
if (ev.t === 'question.request' || ev.t === 'plan.request') {
if (!interactions.some((request) => request.id === ev.id)) interactions.push(ev);
} else if (ev.t === 'permission.resolved') {
interactions = interactions.filter((request) => request.tool_call_id !== ev.tool_call_id);
} else if (ev.t === 'turn.end' || (ev.t === 'session.state' && ev.status === 'error')) {
interactions = [];
}
}

function replyInteraction(request: InteractionRequest, msg: ClientMsg): boolean {
if (!send(msg)) return false;
// Keep the card until the agent confirms resolution; validation errors remain retryable.
return true;
}

let text = $state(untrack(() => initialDraft?.text ?? ''));
let attachments = $state<DraftAttachment[]>(untrack(() => initialDraft?.attachments ?? []));
$effect(() => { const draft = { text, attachments: [...attachments] }; if (onDraftChange) untrack(() => onDraftChange(draft)); });
let attachmentSeq = 0;
let attachmentReads = $state(0);
let attachmentError = $state('');
let sendError = $state('');
let sending = $state(false);
let sendTimer: ReturnType<typeof setTimeout> | undefined;
let scroller = $state<HTMLElement>();
let historyContent = $state<HTMLElement>();
let composer = $state<HTMLTextAreaElement>();
let previewItems = $derived(attachments.map(({ key, attachment, size }) => ({ key, attachment, size })));

const canSend = $derived(
view.connected && historyReady && !view.running && !sending && attachmentReads === 0 && (text.trim() !== '' || attachments.length > 0)
);

$effect(() => {
const content = historyContent;
const scrollport = scroller;
if (!content || !scrollport || typeof ResizeObserver === 'undefined') return;
const observer = new ResizeObserver(() => {
if (historyReady && historyScroll.shouldFollow()) scrollToLatest();
});
observer.observe(content);
observer.observe(scrollport);
return () => observer.disconnect();
});

function historyMetrics(): ScrollMetrics {
return {
scrollTop: scroller?.scrollTop ?? 0,
scrollHeight: scroller?.scrollHeight ?? 0,
clientHeight: scroller?.clientHeight ?? 0
};
}

function onHistoryScroll() {
historyScroll.update(historyMetrics());
if (historyReady && scroller && scroller.scrollTop <= 120) loadOlderHistory();
}

function loadOlderHistory() {
const cursor = historyCursor;
if (!historyReady || !historyHasMore || !cursor || historyLoading) return;
historyAnchor = historyMetrics();
historyLoading = true;
historyError = '';
if (!send({ t: 'history.load', cursor })) {
historyLoading = false;
historyAnchor = undefined;
historyError = '이전 대화 기록 요청을 보내지 못했습니다.';
}
}

function scrollToLatest() {
if (scroller) scroller.scrollTo({ top: scroller.scrollHeight });
}

async function finishHistoryReplay(token: number, target: HistoryScrollTarget) {
await tick();
if (token !== historyReplayGeneration) return;
historyReady = true;
await tick();
if (token !== historyReplayGeneration) return;
if (target.kind === 'bottom') scrollToLatest();
else scroller?.scrollTo({ top: target.top });
}

function followHistoryUpdate() {
const shouldFollow = historyReady ? historyScroll.shouldFollow() : historyReplay !== undefined && historyReplayFollow;
if (!shouldFollow) return;
const token = historyReplayGeneration;
void tick().then(() => {
const stillFollowing = historyReady ? historyScroll.shouldFollow() : historyReplay !== undefined && historyReplayFollow;
if (token === historyReplayGeneration && stillFollowing) scrollToLatest();
});
}

function isTranscriptUpdate(ev: Sep): boolean {
return ev.t === 'user.message' || ev.t === 'user.attachment' || ev.t === 'message.delta' ||
ev.t === 'reasoning.delta' || ev.t === 'tool' || ev.t === 'error' ||
(ev.t === 'turn.end' && Boolean(ev.error));
}

function grow() {
if (!composer) return;
composer.style.height = 'auto';
composer.style.height = `${Math.min(composer.scrollHeight, 240)}px`;
}

function insertComposerText(value: string) {
if (!composer) {
text += value;
return;
}
composer.setRangeText(value, composer.selectionStart, composer.selectionEnd, 'end');
text = composer.value;
grow();
}

async function addFiles(files: File[]) {
const start = attachmentSeq;
attachmentSeq += files.length;
attachmentReads++;
try {
const result = await readAttachmentFiles(files, [], start);
if (destroyed) return;
const merged = mergeDrafts(attachments, result.attachments);
attachments = merged.attachments;
attachmentError = [attachmentError, result.error, merged.error].filter(Boolean).join(' ');
} finally {
attachmentReads--;
}
}

async function attachFiles(e: Event) {
const input = e.currentTarget as HTMLInputElement;
attachmentError = '';
await addFiles(Array.from(input.files ?? []));
input.value = '';
}

async function pasteAttachments(e: ClipboardEvent) {
const clipboard = e.clipboardData;
if (!clipboard) return;
const pastedText = clipboard.getData('text/plain');
const files = clipboardFiles(clipboard);
const longText = isLongPaste(pastedText);
if (!longText && files.length === 0) return;
e.preventDefault();
attachmentError = '';
if (longText) {
try {
if (attachments.length >= MAX_ATTACHMENTS) throw new Error('최대 10개까지 첨부할 수 있습니다.');
attachmentSeq++;
const merged = mergeDrafts(attachments, [createTextDraft(nextPastedTextName(attachments), pastedText, attachmentSeq)]);
if (merged.error) throw new Error(merged.error);
attachments = merged.attachments;
} catch (cause) {
attachmentError = cause instanceof Error ? cause.message : '텍스트 파일을 첨부하지 못했습니다.';
insertComposerText(pastedText);
}
} else if (pastedText) {
insertComposerText(pastedText);
}
await addFiles(files);
}

function submit(e: SubmitEvent) {
e.preventDefault();
if (!canSend) return;
const images = attachments.flatMap(({ attachment }) =>
attachment.kind === 'image' ? [{ mime: attachment.mime, data: attachment.data }] : []
);
const text_files = attachments.flatMap(({ attachment }) =>
attachment.kind === 'text' ? [{ name: attachment.name, text: attachment.text }] : []
);
const message: ClientMsg = {
t: 'prompt',
text: text.trim(),
images,
...(text_files.length ? { text_files } : {})
};
if (!send(message)) {
sendError = '메시지를 보내지 못했습니다. 연결을 확인한 뒤 다시 시도해주세요.';
return;
}
sendError = '';
attachmentError = '';
sending = true;
sendTimer = setTimeout(() => { sending = false; sendError = '전송 확인이 지연됩니다. 대화 기록을 확인한 뒤 다시 시도하세요.'; }, 10000);
}

function onComposerKey(e: KeyboardEvent) {
if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) {
e.preventDefault();
(e.currentTarget as HTMLTextAreaElement).form?.requestSubmit();
}
}

function chooseModel(modelId: string) {
if (!view.connected || view.running || modelPending?.id === id || modelId === view.modelId) return;
if (!send({ t: 'model', model_id: modelId })) {
modelError = { id, message: '모델 변경 요청을 보내지 못했습니다. 연결을 확인한 뒤 다시 시도해주세요.' };
ui.notify(`${title}: 모델 변경 요청을 보내지 못했습니다.`, 'error');
return;
}
modelError = null;
modelPending = { id, requested: modelId };
const request = ++modelRequest;
if (modelTimer !== undefined) clearTimeout(modelTimer);
modelTimer = setTimeout(() => {
if (modelRequest === request && modelPending?.id === id) {
modelPending = null;
modelError = { id, message: '모델 변경을 확인하지 못했습니다. 다시 시도해주세요.' };
ui.notify(`${title}: 모델 변경을 확인하지 못했습니다.`, 'error');
}
modelTimer = undefined;
}, 5000);
}

function setMode(next: Mode) {
if (!view.connected || mode === undefined || modePending?.id === id || next === mode) return;
if (next === 'always-approve' && !confirm('모든 작업을 승인 없이 실행합니다. 계속할까요?')) return;
if (!send({ t: 'mode', mode: next })) {
modeError = { id, message: '권한 모드를 변경하지 못했습니다. 다시 시도해주세요.' };
ui.notify(`${title}: 권한 모드를 변경하지 못했습니다.`, 'error');
return;
}
modeError = null;
modePending = { id, mode: next };
const request = ++modeRequest;
if (modeTimer !== undefined) clearTimeout(modeTimer);
modeTimer = setTimeout(() => {
if (modeRequest === request && modePending?.id === id && modePending.mode === next) {
modePending = null;
modeError = { id, message: '권한 모드 변경을 확인하지 못했습니다. 다시 시도해주세요.' };
ui.notify(`${title}: 권한 모드 변경을 확인하지 못했습니다.`, 'error');
}
modeTimer = undefined;
}, 5000);
}

function permissionDetails(toolCallId: string, fallbackTitle: string) {
const tool = view.items.find((item): item is ToolItem => item.kind === 'tool' && item.id === toolCallId);
const input = tool?.input && typeof tool.input === 'object' ? tool.input as Record<string, unknown> : {};
const command = typeof tool?.input === 'string' ? tool.input : [input.command, input.cmd, input.script].find((value) => typeof value === 'string' && value.trim());
return { command: typeof command === 'string' ? command : fallbackTitle, cwd: row?.cwd ?? '', readOnly: tool?.toolKind === 'read', isCommand: tool?.toolKind === 'execute' };
}

let conversationElement = $state<HTMLDivElement>();
const contextId = $derived(`context-usage-${encodeURIComponent(sessionId)}`);
const contextUsage = $derived(view.usage && Number.isFinite(view.usage.used) && view.usage.used >= 0 && Number.isFinite(view.usage.size) && view.usage.size > 0 ? { ...view.usage, percent: Math.round(view.usage.used / view.usage.size * 100) } : null);
function positionComposerPopover(event: ToggleEvent) {
if (event.newState !== 'open') return;
const popup = event.currentTarget as HTMLElement;
const anchor = conversationElement?.querySelector<HTMLElement>(`[popovertarget="${CSS.escape(popup.id)}"]`);
if (!anchor) return;
const rect = anchor.getBoundingClientRect();
const width = Math.min(280, window.innerWidth - 24);
Object.assign(popup.style, { left: `${Math.max(12, Math.min(rect.right - width, window.innerWidth - width - 12))}px`, right: 'auto', top: 'auto', bottom: `${window.innerHeight - rect.top + 8}px`, width: `${width}px`, maxHeight: `${Math.max(0, rect.top - 20)}px` });
}
$effect(() => {
const element = conversationElement;
if (!element) return;
const closePopovers = (event: Event) => {
for (const popup of element.querySelectorAll<HTMLElement>('[data-composer-popover]:popover-open')) {
if (event.type === 'scroll' && event.target instanceof Node && popup.contains(event.target)) continue;
popup.hidePopover();
}
};
window.addEventListener('resize', closePopovers);
window.addEventListener('scroll', closePopovers, true);
return () => { window.removeEventListener('resize', closePopovers); window.removeEventListener('scroll', closePopovers, true); };
});

function jumpToTool(toolCallId: string) {
const target = document.getElementById(toolDomId(toolCallId));
if (!target) return;
if (target instanceof HTMLDetailsElement) target.open = true;
let parent = target.parentElement;
while (parent) {
if (parent instanceof HTMLDetailsElement && !parent.open) parent.open = true;
parent = parent.parentElement;
}
target.scrollIntoView({ behavior: 'smooth', block: 'center' });
target.focus({ preventScroll: true });
}

function toolDomId(toolCallId: string): string {
return `tool-${encodeURIComponent(sessionId)}-${encodeURIComponent(toolCallId)}`;
}

function fillStarter(prompt: string) {
text = prompt;
composer?.focus();
}


</script>
<div bind:this={conversationElement} class="conversation-group flex h-full min-h-0 min-w-0 flex-1 flex-col overflow-hidden" data-testid="session-composer" data-session-id={sessionId}>
<section class="conversation-group flex h-full min-h-0 flex-col overflow-hidden" aria-label="대화">
<section class="flex min-h-0 min-w-0 flex-1 flex-col" aria-label="작업 대화 및 타임라인">
<div bind:this={scroller} onscroll={onHistoryScroll} class="flex min-h-0 flex-1 flex-col overflow-y-auto">
<div bind:this={historyContent} class="mx-auto flex w-full max-w-3xl flex-col gap-4 px-4 py-6">
{#if hostError}
<p role="alert" class="rounded-xl bg-red-50 p-4 text-sm text-red-900 dark:bg-red-950 dark:text-red-100">{hostError}</p>
{:else if !view.connected}
<p role="status" class="text-sm text-neutral-500">서버에 연결하는 중입니다.</p>
{/if}
{#if metadataError}<p role="alert" class="text-sm text-red-700">{metadataError}</p>{/if}
{#if !historyReady && !historyError && !hostError}<HistorySkeleton variant="work" />{/if}
{#if historyError}<p role="alert" class="text-sm text-red-700">{historyError}</p>{/if}
{#if historyReady && historyHasMore}
<button type="button" disabled={historyLoading} class="self-center rounded-lg border border-neutral-300 px-3 py-2 text-xs hover:bg-neutral-100 disabled:opacity-50 dark:border-neutral-700 dark:hover:bg-neutral-800" onclick={loadOlderHistory}>{historyLoading ? '이전 기록 불러오는 중…' : '이전 대화 불러오기'}</button>
{/if}
{#if historyReady && !historyError && view.connected && view.items.length === 0}
<section class="flex flex-col gap-3 rounded-2xl border border-neutral-200 bg-white p-4 dark:border-neutral-800 dark:bg-neutral-900" aria-label="새 코딩 작업 안내">
<div><h2 class="text-base font-semibold">코딩 작업을 시작하세요</h2><p class="mt-1 text-sm text-neutral-600 dark:text-neutral-300">요청을 보내면 진행 상황과 도구 실행이 여기에 표시됩니다.</p></div>
<dl class="grid gap-2 text-xs text-neutral-700 dark:text-neutral-300 sm:grid-cols-3">
<dt class="sr-only">작업공간</dt><dd class="min-w-0 truncate rounded-lg bg-neutral-100 px-3 py-2 dark:bg-neutral-800" title={row?.cwd}>작업공간 · {workspaceDisplayName(row?.cwd)}</dd>
<dt class="sr-only">권한 모드</dt><dd class="rounded-lg bg-neutral-100 px-3 py-2 dark:bg-neutral-800">권한 모드 · {modeLabel}</dd>
<dt class="sr-only">모델</dt><dd class="min-w-0 truncate rounded-lg bg-neutral-100 px-3 py-2 dark:bg-neutral-800" title={view.modelId}>모델 · {view.modelId || row?.modelId || '확인 중'}</dd>
</dl>
<div class="flex flex-wrap gap-2">
{#each ['프로젝트 구조를 요약해 주세요', '이 저장소의 테스트를 확인해 주세요', '현재 작업공간의 상태를 설명해 주세요'] as suggestion (suggestion)}
<button type="button" class="min-h-9 rounded-lg border border-neutral-300 px-3 text-xs text-neutral-800 hover:bg-sky-50 focus-visible:outline-2 focus-visible:outline-sky-700 dark:border-neutral-700 dark:text-neutral-200 dark:hover:bg-sky-950" onclick={() => fillStarter(suggestion)}>{suggestion}</button>
{/each}
</div>
</section>
{/if}
{#each timeline as entry (entry.kind === 'item' ? `item-${entry.item.key}` : `tools-${entry.items[0].key}`)}
{#if entry.kind === 'tools'}
<div class="flex flex-col gap-0 rounded-xl border border-neutral-200 dark:border-neutral-800">{#each entry.items as tool (tool.key)}<ToolRow item={tool} id={toolDomId(tool.id)} onselect={(tool) => onOpenTool?.(tool)} grouped />{/each}</div>
{:else}
{@const item = entry.item}
{#if item.kind === 'attachment'}
<div class="ml-auto max-w-[85%]">
<AttachmentPreview items={[{ key: item.key, attachment: item.attachment }]} />
</div>
{:else if item.kind === 'tool'}
<ToolRow {item} id={toolDomId(item.id)} onselect={(tool) => onOpenTool?.(tool)} />
{:else if item.role === 'user'}
<div class="ml-auto max-w-[85%] rounded-3xl bg-neutral-200/70 px-4 py-2.5 text-[15px] leading-7 break-words whitespace-pre-wrap dark:bg-neutral-800">
{item.text}
</div>
{:else if item.role === 'assistant'}
<div class="min-w-0"><Markdown text={item.text} /></div>
{:else if item.role === 'reasoning'}
<details class="group text-sm text-neutral-500">
<summary class="flex min-h-9 cursor-pointer list-none items-center gap-1.5 select-none">
<Icon name="chevron" class="size-3.5 transition-transform group-open:rotate-90" />
추론 과정
</summary>
<p class="mt-1 border-l-2 border-neutral-300 pl-3 whitespace-pre-wrap dark:border-neutral-700">{item.text}</p>
</details>
{:else}
<p role="alert" class="rounded-2xl bg-red-50 px-4 py-3 text-sm text-red-900 dark:bg-red-950 dark:text-red-100">
오류: {item.text}
</p>
{/if}
{/if}
{/each}
<div id={pendingId} class="flex flex-col gap-3">
{#each interactions as request (request.id)}
<InteractionCard {request} connected={view.connected} reply={(msg) => replyInteraction(request, msg)} />
{/each}
{#each view.permissions as p (p.toolCallId)}
{@const actions = permissionActions(p.options)}
{@const onceOption = actions.once}
{@const rejectOption = actions.reject}
{@const details = permissionDetails(p.toolCallId, p.title)}
<section data-tool-call-id={p.toolCallId} aria-label="실행 승인" class="rounded-[14px] border border-[#ecd9b0] border-l-[3px] border-l-amber-600 bg-[#fffdf8] text-neutral-900 dark:border-neutral-700 dark:border-l-amber-500 dark:bg-neutral-900 dark:text-neutral-100">
<div class="flex items-start gap-3 px-4 pt-3.5 pb-2.5">
<span class="flex size-[30px] shrink-0 items-center justify-center rounded-[9px] bg-amber-100 text-amber-800 dark:bg-amber-950 dark:text-amber-300"><Icon name="shield" class="size-4" /></span>
<div class="min-w-0 flex-1"><h3 class="text-sm font-semibold">{details.isCommand ? '명령 실행 승인이 필요해요' : '도구 실행 승인이 필요해요'}</h3><p class="mt-0.5 text-xs leading-5 text-neutral-600 dark:text-neutral-300">{p.title !== details.command ? p.title : '요청 내용을 확인한 뒤 실행을 허용해 주세요.'}</p></div>
{#if details.readOnly}<span class="shrink-0 rounded-full bg-emerald-50 px-2 py-1 text-[11px] font-semibold text-emerald-800 dark:bg-emerald-950 dark:text-emerald-200">읽기 전용</span>{/if}
</div>
<div class="flex flex-col gap-1.5 px-4 pb-2.5">
{#if details.command}<pre class="max-h-36 overflow-auto whitespace-pre-wrap break-words rounded-[10px] bg-neutral-950 px-3 py-2.5 font-mono text-xs leading-5 text-neutral-100">{details.command}</pre>{/if}
<div class="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1 text-xs text-neutral-500">{#if details.cwd}<span class="min-w-0 truncate font-mono" title={details.cwd}>작업 위치 · {details.cwd}</span>{/if}<button type="button" class="min-h-6 shrink-0 text-sky-800 underline underline-offset-4 dark:text-sky-300" onclick={() => jumpToTool(p.toolCallId)}>관련 도구 호출로 이동</button></div>
</div>
<div class="flex flex-wrap items-center gap-2 rounded-b-[13px] border-t border-[#f3e6c8] bg-[#fdf7ea] px-4 py-2.5 dark:border-neutral-700 dark:bg-neutral-800">
{#if onceOption}<button type="button" disabled={!view.connected} class="inline-flex h-9 items-center gap-1.5 rounded-[10px] bg-neutral-900 px-4 text-[13px] font-semibold text-white hover:bg-neutral-700 focus-visible:outline-2 focus-visible:outline-sky-700 disabled:opacity-40 dark:bg-neutral-100 dark:text-neutral-950" onclick={() => send({ t: 'permission', id: p.id, option_id: onceOption.id })}><Icon name="check" class="size-3.5" />{permissionOptionLabel(onceOption)}</button>{/if}
{#if rejectOption}<button type="button" disabled={!view.connected} class="h-9 rounded-[10px] border border-[#dcd3bf] bg-white px-3.5 text-[13px] font-medium hover:bg-neutral-50 focus-visible:outline-2 focus-visible:outline-sky-700 disabled:opacity-40 dark:border-neutral-600 dark:bg-neutral-900" onclick={() => send({ t: 'permission', id: p.id, option_id: rejectOption.id })}>{permissionOptionLabel(rejectOption)}</button>{/if}
{#if actions.permanent.length || actions.other.length}
{@const optionsId = `permission-options-${encodeURIComponent(sessionId)}-${encodeURIComponent(p.toolCallId)}`}
<button type="button" popovertarget={optionsId} aria-haspopup="dialog" class="ml-auto inline-flex h-9 items-center gap-1 rounded-lg px-2 text-[13px] font-medium hover:bg-amber-100 dark:hover:bg-neutral-700">{actions.permanent.length ? '항상 적용' : '추가 선택'}<Icon name="chevron" class="size-3 -rotate-90" /></button>
<div id={optionsId} popover="auto" data-composer-popover onbeforetoggle={positionComposerPopover} role="dialog" aria-label="추가 승인 선택" class="fixed inset-auto m-0 overflow-auto rounded-xl border border-neutral-200 bg-white p-1.5 shadow-xl dark:border-neutral-700 dark:bg-neutral-900">
{#if actions.permanent.length}<p class="px-2.5 py-2 text-xs leading-5 text-neutral-600 dark:text-neutral-300">이 선택은 이후 승인 동작에 계속 적용됩니다.</p>{/if}
{#each [...actions.permanent, ...actions.other] as opt (opt.id)}<button type="button" disabled={!view.connected} class="block min-h-9 w-full rounded-lg px-2.5 py-2 text-left text-[13px] hover:bg-neutral-100 disabled:opacity-40 dark:hover:bg-neutral-800" onclick={(event) => { if (send({ t: 'permission', id: p.id, option_id: opt.id })) event.currentTarget.closest<HTMLElement>('[popover]')?.hidePopover(); }}>{permissionOptionLabel(opt)}</button>{/each}
</div>
{/if}
</div>
</section>
{/each}

</div>
</div>
</div>

<div class="relative mx-auto flex w-full max-w-3xl shrink-0 flex-col gap-2 bg-white px-4 pt-2 pb-3 dark:bg-neutral-950">
{#if view.running}<div role="status" class="flex min-h-9 items-center gap-2 rounded-lg bg-neutral-100 px-3 text-sm dark:bg-neutral-800"><span>{[runningCommandCount > 0 ? `명령 ${runningCommandCount}개 실행 중` : '', runningAgentCount > 0 ? `에이전트 ${runningAgentCount}개 실행 중` : ''].filter(Boolean).join(' · ') || '작업 실행 중'}</span><button type="button" class="ml-auto min-h-8 rounded-lg px-3 hover:bg-neutral-200 dark:hover:bg-neutral-700" aria-label="중단" onclick={cancelCurrent}>중단</button></div>{/if}
{#if modelError?.id === id}<p role="alert" class="text-sm text-red-600">{modelError.message}</p>{/if}
{#if modeError?.id === id}<p role="alert" class="text-sm text-red-600">{modeError.message}</p>{/if}
<form
class="flex flex-col gap-1.5 rounded-[22px] border border-[#dcdce0] bg-white pt-2.5 pb-1.5 pl-4 pr-2 shadow-[0_2px_10px_rgba(20,20,30,.06)] focus-within:ring-2 focus-within:ring-sky-500/20 dark:border-neutral-700 dark:bg-neutral-900"
onsubmit={submit}
>
{#if attachments.length > 0}
<AttachmentPreview
items={previewItems}
removable
onremove={(key) => (attachments = attachments.filter((item) => item.key !== key))}
/>
{/if}
<label for={composerId} class="sr-only">메시지</label>
<textarea
id={composerId}
disabled={sending}
bind:this={composer}
bind:value={text}
oninput={grow}
onpaste={pasteAttachments}
onkeydown={onComposerKey}
rows="2"
placeholder={!view.connected ? '서버 연결을 기다리는 중…' : view.running ? '작업 실행 중 · 메시지는 실행이 끝난 뒤 보낼 수 있습니다.' : '메시지를 입력하세요'}
class="max-h-60 min-h-12 w-full resize-none bg-transparent p-0 text-[15px] leading-6 outline-none placeholder:text-neutral-600 dark:placeholder:text-neutral-300"
></textarea>
{#if attachmentError}
<p role="alert" class="px-2 text-sm text-red-700 dark:text-red-300">{attachmentError}</p>
{/if}
{#if sendError}
<p role="alert" class="px-2 text-sm text-red-700 dark:text-red-300">{sendError}</p>
{/if}
<div class="flex min-w-0 flex-nowrap items-center gap-0.5">
<label class="flex h-[34px] shrink-0 cursor-pointer items-center justify-center rounded-lg px-1 hover:bg-neutral-100 dark:hover:bg-neutral-800" title="이미지 또는 텍스트 파일 첨부"><Icon name="image" class="size-4" /><span class="sr-only">이미지 또는 TXT/MD 파일 첨부</span><input type="file" accept="image/*,.txt,.md,text/plain,text/markdown" multiple class="sr-only" onchange={attachFiles} /></label>
<div class="min-w-0 max-w-[180px] basis-[180px] shrink"><ComposerRouting id={`composer-routing-${encodeURIComponent(sessionId)}`} host={sessionHost} value={view.modelId || row?.modelId || 'Auto'} onselect={chooseModel} disabled={!view.connected || view.running || modelPending?.id === id} /></div>
<div class="min-w-0 flex-1"></div>
{#if contextUsage}
<button type="button" popovertarget={contextId} aria-haspopup="dialog" aria-label={`컨텍스트 사용량 ${contextUsage.percent}%`} title={`컨텍스트 ${contextUsage.used.toLocaleString()} / ${contextUsage.size.toLocaleString()} 토큰`} class="inline-flex h-[34px] shrink-0 items-center gap-1 rounded-lg px-1 text-xs hover:bg-neutral-100 dark:hover:bg-neutral-800"><svg aria-hidden="true" class="size-4 -rotate-90" viewBox="0 0 20 20"><circle cx="10" cy="10" r="7" fill="none" stroke="#e3e3e7" stroke-width="3" /><circle cx="10" cy="10" r="7" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" pathLength="100" stroke-dasharray={`${Math.min(100, contextUsage.percent)} 100`} /></svg><span>{contextUsage.percent}%</span></button>
<div id={contextId} popover="auto" data-composer-popover onbeforetoggle={positionComposerPopover} role="dialog" aria-label="컨텍스트 사용량" class="fixed inset-auto m-0 overflow-auto rounded-[14px] border border-neutral-200 bg-white p-3.5 text-[13px] shadow-xl dark:border-neutral-700 dark:bg-neutral-900"><h3 class="font-semibold">컨텍스트 사용량 · {contextUsage.percent}%</h3><p class="mt-2 font-mono text-xs">{contextUsage.used.toLocaleString()} / {contextUsage.size.toLocaleString()} 토큰</p><p class="mt-2 text-xs leading-5 text-neutral-500">현재 세션에서 보고한 사용량입니다.</p></div>
{:else}<span role="img" aria-label="컨텍스트 사용량 알 수 없음" title="컨텍스트 사용량 정보 없음" class="flex h-[34px] w-[30px] shrink-0 items-center justify-center text-xs text-neutral-500">—</span>{/if}
<details class="relative shrink-0"><summary aria-label={`승인 정책: ${modeLabel}`} title={`승인 정책 · ${modeLabel}`} class="flex h-[34px] w-[34px] cursor-pointer list-none items-center justify-center rounded-lg hover:bg-neutral-100 dark:hover:bg-neutral-800"><Icon name="shield" class="size-3.5" /><span class="sr-only">승인 정책 · {modeLabel}</span></summary><div class="absolute bottom-full right-0 z-20 mb-2 w-56 max-w-[calc(100vw-48px)] rounded-[14px] border border-neutral-200 bg-white p-1.5 shadow-xl dark:border-neutral-700 dark:bg-neutral-900">
{#each MODES as candidate (candidate)}<button type="button" disabled={!view.connected || mode === undefined || modePending?.id === id || candidate === mode} class="flex min-h-10 w-full items-start gap-2 rounded-lg px-2.5 py-2 text-left text-[13px] hover:bg-neutral-100 disabled:opacity-40 dark:hover:bg-neutral-800" onclick={(event) => { const parent = event.currentTarget.closest('details'); if (parent) parent.open = false; setMode(candidate); }}><span class="mt-0.5 w-4 shrink-0">{#if candidate === mode}<Icon name="check" class="size-3.5" />{/if}</span><span>{MODE_LABEL[candidate]}<small class="mt-0.5 block text-xs leading-4 text-neutral-500">{candidate === 'ask' ? '도구 사용 전 확인' : candidate === 'auto' ? '자동 승인 정책 사용' : '확인 없이 항상 승인'}</small></span></button>{/each}
</div></details>
{#if !view.running}<button type="submit" disabled={!canSend} aria-label="보내기" class="flex size-[38px] shrink-0 items-center justify-center rounded-full bg-neutral-900 text-white hover:bg-neutral-700 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-neutral-900 disabled:opacity-30 dark:bg-white dark:text-neutral-950"><Icon name="send" class="size-4" /></button>{/if}
</div>
</form>
<p class="text-center text-[11px] leading-4 text-neutral-500">작업은 연결된 호스트의 파일을 직접 수정할 수 있습니다.</p>
</div>
</section>
</section>

</div>
