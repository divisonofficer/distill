<script lang="ts">
import { goto } from '$app/navigation';
import { page } from '$app/state';
import { connect, type Socket } from '$lib/api';
import Badge from '$lib/components/Badge.svelte';
import Icon from '$lib/components/Icon.svelte';
import InteractionCard from '$lib/components/InteractionCard.svelte';
import Markdown from '$lib/components/Markdown.svelte';
import ToolRow from '$lib/components/ToolRow.svelte';
import { board } from '$lib/board.svelte';
import { modeFromRow } from '$lib/types';
import { MODES, MODE_LABEL } from '$lib/prefs';
import { sessionTitle, statusBadge } from '$lib/format';
import { SessionView } from '$lib/session.svelte';
import { ui } from '$lib/ui.svelte';
import type { ClientMsg, InteractionRequest, Mode, Sep } from '$lib/types';

const id = $derived(page.params.id ?? '');
const view = new SessionView();
let interactions = $state<InteractionRequest[]>([]);
const row = $derived(board.find(id));
const title = $derived(view.title || (row ? sessionTitle(row) : id));
const badge = $derived(statusBadge(view.status));

const mode = $derived<Mode | undefined>(modeFromRow(row));
const modeLabel = $derived(mode === undefined ? '확인 중' : MODE_LABEL[mode]);
let modePending = $state<{ id: string; mode: Mode } | null>(null);
let modeError = $state<{ id: string; message: string } | null>(null);
let modeTimer: ReturnType<typeof setTimeout> | undefined;
let modeRequest = 0;

$effect(() => {
const pending = modePending;
if (pending?.id === id && modeFromRow(row) === pending.mode) {
modeRequest++;
if (modeTimer !== undefined) clearTimeout(modeTimer);
modeTimer = undefined;
modePending = null;
modeError = null;
}
});

// One socket per session view: the effect closes the previous socket before opening the next.
// Each open resets the transcript so replayed events never stack on old ones.
let sock: Socket | undefined;
$effect(() => {
const s = connect<Sep>(`/ws/session/${id}`, {
onOpen: () => {
interactions = [];
view.reset();
view.setConnected(true);
},
onClose: () => {
interactions = [];
view.setConnected(false);
},
onMessage: (m) => {
view.apply(m);
applyInteraction(m);
}
});
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

let text = $state('');
let images = $state<{ key: number; mime: string; data: string; url: string }[]>([]);
let imageSeq = 0;
let sendError = $state('');
let scroller = $state<HTMLElement>();
let composer = $state<HTMLTextAreaElement>();

const canSend = $derived(view.connected && !view.running && (text.trim() !== '' || images.length > 0));

$effect(() => {
void view.revision;
scroller?.scrollTo({ top: scroller.scrollHeight });
});

function grow() {
if (!composer) return;
composer.style.height = 'auto';
composer.style.height = `${Math.min(composer.scrollHeight, 240)}px`;
}

function submit(e: SubmitEvent) {
e.preventDefault();
if (!canSend) return;
if (!send({
t: 'prompt',
text: text.trim(),
images: images.map(({ mime, data }) => ({ mime, data }))
})) {
sendError = '메시지를 보내지 못했습니다. 연결을 확인한 뒤 다시 시도해주세요.';
return;
}
sendError = '';
text = '';
images = [];
grow();
}

function onComposerKey(e: KeyboardEvent) {
if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) {
e.preventDefault();
(e.currentTarget as HTMLTextAreaElement).form?.requestSubmit();
}
}

function readDataUrl(file: File): Promise<string> {
return new Promise((resolve, reject) => {
const r = new FileReader();
r.onload = () => resolve(String(r.result));
r.onerror = () => reject(r.error);
r.readAsDataURL(file);
});
}

async function attach(e: Event) {
const input = e.currentTarget as HTMLInputElement;
for (const f of Array.from(input.files ?? [])) {
if (!f.type.startsWith('image/')) continue;
const url = await readDataUrl(f);
const comma = url.indexOf(',');
const mime = url.slice(5, url.indexOf(';'));
images.push({ key: ++imageSeq, mime, data: url.slice(comma + 1), url });
}
input.value = '';
}

function cycleMode() {
if (!view.connected || mode === undefined || modePending?.id === id) return;
const next = MODES[(MODES.indexOf(mode) + 1) % MODES.length];
if (next === 'always-approve' && !confirm('모든 작업을 승인 없이 실행합니다. 계속할까요?')) return;
if (!send({ t: 'mode', mode: next })) {
modeError = { id, message: '권한 모드를 변경하지 못했습니다. 다시 시도해주세요.' };
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
}
modeTimer = undefined;
}, 5000);
}

function optionLabel(kind: string, name: string): string {
switch (kind) {
case 'allow_once':
return '허용';
case 'allow_always':
return '항상 허용';
case 'reject_once':
return '거부';
default:
return name;
}
}
</script>

<svelte:head>
<title>{title} · Postudio</title>
</svelte:head>

<div class="flex min-h-0 flex-1 flex-col">
<div class="flex h-12 shrink-0 items-center gap-1 border-b border-neutral-200 px-2 pt-[env(safe-area-inset-top)] dark:border-neutral-800">
<button
type="button"
class="flex size-10 items-center justify-center rounded-xl hover:bg-neutral-200/70 lg:hidden dark:hover:bg-neutral-800"
aria-label="메뉴 열기"
onclick={() => (ui.drawerOpen = true)}
>
<Icon name="menu" />
</button>
{#if ui.sidebarHidden}
<button
type="button"
class="hidden size-10 items-center justify-center rounded-xl hover:bg-neutral-200/70 lg:flex dark:hover:bg-neutral-800"
aria-label="사이드바 펼치기"
title="사이드바 펼치기"
onclick={() => ui.toggleSidebar()}
>
<Icon name="sidebar" />
</button>
{/if}
<h1 class="min-w-0 flex-1 truncate px-1 text-sm font-medium">{title}</h1>
<Badge label={badge.label} tone={badge.tone} />
{#if view.running}
<button
type="button"
class="ml-1 min-h-10 rounded-xl px-3 text-sm text-red-700 hover:bg-red-50 dark:text-red-300 dark:hover:bg-red-950"
onclick={() => send({ t: 'cancel' })}
>
중단
</button>
{/if}
</div>

{#if view.usage}
<div class="mx-auto flex w-full max-w-3xl items-center gap-2 px-4 pt-2 text-xs text-neutral-500">
<span>사용량</span>
<div class="h-1.5 flex-1 overflow-hidden rounded-full bg-neutral-200 dark:bg-neutral-800" role="presentation">
<div
class="h-full bg-neutral-600 dark:bg-neutral-300"
style:width="{Math.min(100, (view.usage.used / Math.max(1, view.usage.size)) * 100)}%"
></div>
</div>
<span class="font-mono">{view.usage.used.toLocaleString()} / {view.usage.size.toLocaleString()}</span>
</div>
{/if}

<div bind:this={scroller} class="flex min-h-0 flex-1 flex-col overflow-y-auto">
<div class="mx-auto flex w-full max-w-3xl flex-col gap-4 px-4 py-6">
{#if !view.connected}
<p role="status" class="text-sm text-neutral-500">서버에 연결하는 중입니다.</p>
{/if}
{#each view.items as item (item.key)}
{#if item.kind === 'tool'}
<ToolRow {item} />
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
{/each}
</div>
</div>

<div class="mx-auto flex w-full max-w-3xl flex-col gap-2 px-4 pb-4">
{#each interactions as request (request.id)}
<InteractionCard {request} connected={view.connected} reply={(msg) => replyInteraction(request, msg)} />
{/each}
{#each view.permissions as p (p.toolCallId)}
<div class="rounded-2xl border border-amber-400 bg-amber-50 p-4 shadow-lg dark:border-amber-600 dark:bg-amber-950">
<p class="mb-3 flex items-start gap-2 text-sm font-medium">
<Icon name="shield" class="mt-0.5 size-4 shrink-0 text-amber-700 dark:text-amber-300" />
<span class="min-w-0 break-words">승인 필요: {p.title}</span>
</p>
<div class="flex flex-wrap gap-2">
{#each p.options as opt (opt.id)}
<button
type="button"
disabled={!view.connected}
class="min-h-10 flex-1 rounded-xl border border-neutral-300 bg-white px-4 text-sm font-medium hover:bg-neutral-100 disabled:opacity-40 dark:border-neutral-600 dark:bg-neutral-900 dark:hover:bg-neutral-800"
onclick={() => send({ t: 'permission', id: p.id, option_id: opt.id })}
>
{optionLabel(opt.kind, opt.name)}
</button>
{/each}
</div>
</div>
{/each}

{#if modeError?.id === id}<p role="alert" class="text-sm text-red-600">{modeError.message}</p>{/if}
<form
class="flex flex-col gap-2 rounded-3xl border border-neutral-200 bg-white p-2 shadow-lg dark:border-neutral-700 dark:bg-neutral-900"
onsubmit={submit}
>
{#if images.length > 0}
<ul class="flex flex-wrap gap-2 px-1 pt-1">
{#each images as img (img.key)}
<li class="relative">
<img src={img.url} alt="첨부 이미지 미리보기" class="size-16 rounded-xl object-cover" />
<button
type="button"
class="absolute -top-2 -right-2 flex size-7 items-center justify-center rounded-full bg-neutral-800 text-white"
aria-label="첨부 이미지 제거"
onclick={() => (images = images.filter((i) => i.key !== img.key))}
>
<Icon name="close" class="size-3.5" />
</button>
</li>
{/each}
</ul>
{/if}
<label for="composer" class="sr-only">메시지</label>
<textarea
id="composer"
bind:this={composer}
bind:value={text}
oninput={grow}
onkeydown={onComposerKey}
rows="1"
placeholder="무엇을 도와드릴까요?"
class="max-h-60 min-h-10 w-full resize-none bg-transparent px-2 py-1.5 text-[15px] leading-6 outline-none placeholder:text-neutral-400"
></textarea>
{#if sendError}
<p role="alert" class="px-2 text-sm text-red-700 dark:text-red-300">{sendError}</p>
{/if}
<div class="flex items-center gap-1">
<label class="flex size-10 cursor-pointer items-center justify-center rounded-xl hover:bg-neutral-100 dark:hover:bg-neutral-800" title="이미지 첨부">
<Icon name="image" />
<span class="sr-only">이미지 첨부</span>
<input type="file" accept="image/*" multiple class="sr-only" onchange={attach} />
</label>
<button
type="button"
disabled={!view.connected || mode === undefined || modePending?.id === id}
class="min-h-10 rounded-xl px-3 text-sm hover:bg-neutral-100 disabled:opacity-40 dark:hover:bg-neutral-800"
aria-label="권한 모드: {modeLabel}. 눌러서 변경"
onclick={cycleMode}
>
{modeLabel}
</button>
{#if view.running}
<button
type="button"
class="ml-auto flex size-10 items-center justify-center rounded-full bg-neutral-900 text-white dark:bg-neutral-100 dark:text-neutral-900"
aria-label="중단"
onclick={() => send({ t: 'cancel' })}
>
<Icon name="stop" class="size-4" />
</button>
{:else}
<button
type="submit"
disabled={!canSend}
aria-label="보내기"
class="ml-auto flex size-10 items-center justify-center rounded-full bg-neutral-900 text-white disabled:opacity-30 dark:bg-neutral-100 dark:text-neutral-900"
>
<Icon name="send" class="size-4" />
</button>
{/if}
</div>
</form>
</div>
</div>
