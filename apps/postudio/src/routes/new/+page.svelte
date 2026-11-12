<script lang="ts">
import { goto } from '$app/navigation';
import { onMount } from 'svelte';
import { postJson } from '$lib/api';
import { board } from '$lib/board.svelte';
import { sessionHref } from '$lib/fleet';
import { workspacesForHost } from '$lib/workspaces';
import AttachmentPreview from '$lib/components/AttachmentPreview.svelte';
import Icon from '$lib/components/Icon.svelte';
import ModelSelect from '$lib/components/ModelSelect.svelte';
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
import { loadDefaultMode, loadRecent, MODES, MODE_LABEL, rememberCwd } from '$lib/prefs';
import type { Mode, NewSession } from '$lib/types';

let ownerHost = $state('');
let touched = false;
let currentHost = $derived(board.origin || (typeof location === 'undefined' ? '' : location.origin));
onMount(() => { ownerHost = currentHost; });
$effect(() => { if (!touched && currentHost && ownerHost !== currentHost) { ownerHost = currentHost; modelId = ''; } });
function switchHost() {
 ownerHost = currentHost;
 cwd = ''; worktree = false; wtName = ''; modelId = ''; workspaceOpen = false;
 error = '';
}
let cwd = $state('');
let worktree = $state(false);
let wtName = $state('');
let mode = $state<Mode>(loadDefaultMode());
let modelId = $state('');
let prompt = $state('');
let promptElement = $state<HTMLTextAreaElement>();
let attachments = $state<DraftAttachment[]>([]);
let attachmentSeq = 0;
let attachmentReads = $state(0);
let attachmentError = $state('');
let error = $state('');
let busy = $state(false);
let previewItems = $derived(attachments.map(({ key, attachment, size }) => ({ key, attachment, size })));
let recent = $state<string[]>(loadRecent());
let workspaceOpen = $state(false);
let workspaces = $derived(workspacesForHost(board.rows, recent, ownerHost, currentHost));
let selectedTemporaryWorkspace = $derived(workspaces.temporary.find((workspace) => workspace.cwd === cwd));

function selectWorkspace(path: string) {
touched = true;
cwd = path;
worktree = true;
workspaceOpen = false;
}

function updateCustomCwd(event: Event) {
const input = event.currentTarget as HTMLInputElement;
touched = true;
if (!cwd.trim() && input.value.trim()) worktree = true;
cwd = input.value;
}

function insertPromptText(value: string) {
if (!promptElement) {
prompt += value;
return;
}
promptElement.setRangeText(value, promptElement.selectionStart, promptElement.selectionEnd, 'end');
prompt = promptElement.value;
}

async function addFiles(files: File[]) {
const start = attachmentSeq;
attachmentSeq += files.length;
attachmentReads++;
try {
const result = await readAttachmentFiles(files, [], start);
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
const text = clipboard.getData('text/plain');
const files = clipboardFiles(clipboard);
const longText = isLongPaste(text);
if (!longText && files.length === 0) return;
e.preventDefault();
attachmentError = '';
if (longText) {
try {
if (attachments.length >= MAX_ATTACHMENTS) throw new Error('최대 10개까지 첨부할 수 있습니다.');
attachmentSeq++;
const merged = mergeDrafts(attachments, [createTextDraft(nextPastedTextName(attachments), text, attachmentSeq)]);
if (merged.error) throw new Error(merged.error);
attachments = merged.attachments;
} catch (cause) {
attachmentError = cause instanceof Error ? cause.message : '텍스트 파일을 첨부하지 못했습니다.';
insertPromptText(text);
}
} else if (text) {
insertPromptText(text);
}
await addFiles(files);
}

async function submit(e: SubmitEvent) {
e.preventDefault();
error = '';
if (busy || attachmentReads) return;
const dir = cwd.trim();
if (!dir) {
error = '작업 경로를 입력하세요.';
return;
}
if (worktree && !/^[A-Za-z0-9_-]{1,64}$/.test(wtName.trim())) {
error = 'worktree 이름은 영문자, 숫자, 하이픈, 밑줄로 1~64자 입력하세요.';
return;
}
const body: NewSession = { cwd: dir, mode };
if (modelId) body.modelId = modelId;
if (worktree) body.worktree = { name: wtName.trim() };
if (prompt.trim()) body.prompt = prompt.trim();
const images = attachments.flatMap(({ attachment }) =>
attachment.kind === 'image' ? [{ mime: attachment.mime, data: attachment.data }] : []
);
const text_files = attachments.flatMap(({ attachment }) =>
attachment.kind === 'text' ? [{ name: attachment.name, text: attachment.text }] : []
);
if (images.length) body.images = images;
if (text_files.length) body.text_files = text_files;
const origin = ownerHost || currentHost;
if (origin !== currentHost) {
 error = '연결 호스트가 변경되어 작업 생성을 중단했습니다. 실행 호스트로 다시 연결하거나 현재 호스트로 전환한 뒤 작업공간과 모델을 다시 선택하세요.';
 return;
}

busy = true;
try {
const res = await postJson<{ sessionId: string }>('/api/sessions', body, origin || undefined);
rememberCwd(dir);
await goto(sessionHref(res.sessionId, origin));
} catch (cause) {
const detail = cause instanceof Error ? cause.message : '알 수 없는 오류';
error = `세션을 만들지 못했습니다. ${detail}`;
} finally {
busy = false;
}
}
</script>

<svelte:head>
<title>새 작업 · Postudio</title>
</svelte:head>

<form class="flex min-h-0 flex-1 flex-col" onsubmit={submit}>
<section aria-label="새 작업 설정" class="min-h-0 flex-1 overflow-y-auto">
<div class="mx-auto flex w-full max-w-[808px] flex-col gap-7 px-4 py-8 md:px-6 md:py-10">
<header class="flex flex-col gap-1.5">
<h1 class="text-[26px] font-semibold tracking-tight">어디에서 작업을 시작할까요?</h1>
<p class="text-sm leading-6 text-neutral-600 dark:text-neutral-300">에이전트가 코드를 읽고 명령을 실행할 환경을 고르세요.</p>
</header>
<section role="group" aria-label="호스트 선택" class="flex flex-col gap-2.5">
<h2 class="text-sm font-semibold text-neutral-600 dark:text-neutral-300">호스트</h2>
<div class="flex min-h-20 flex-wrap items-center justify-between gap-3 rounded-2xl border border-neutral-300 bg-white p-4 dark:border-neutral-700 dark:bg-neutral-900">
<div class="flex min-w-0 flex-col gap-1">
<span class="truncate text-sm font-semibold" title={ownerHost}>{ownerHost || '호스트 확인 중'}</span>
<span class="flex items-center gap-2 text-xs text-neutral-600 dark:text-neutral-300"><span aria-hidden="true" class="size-2 rounded-full" class:bg-emerald-500={board.connected} class:bg-neutral-400={!board.connected}></span>{board.connected ? '연결됨' : '연결 중'}</span>
</div>
{#if ownerHost !== currentHost}<button type="button" class="min-h-9 shrink-0 rounded-xl border border-neutral-300 px-3 text-xs font-medium hover:bg-neutral-100 focus-visible:outline-2 focus-visible:outline-neutral-500 dark:border-neutral-700 dark:hover:bg-neutral-800" onclick={switchHost}>현재 호스트로 전환</button>{/if}
</div>
</section>
<section role="group" aria-label="워크스페이스 선택" class="flex flex-col gap-2.5">
<h2 class="text-sm font-semibold text-neutral-600 dark:text-neutral-300">워크스페이스</h2>
{#if workspaces.normal.length || selectedTemporaryWorkspace}
<div class="flex flex-wrap gap-2">
{#each workspaces.normal as workspace (workspace.cwd)}
<button type="button" aria-pressed={cwd === workspace.cwd} title={workspace.cwd} class="min-h-9 max-w-full rounded-full border px-3 text-left text-sm hover:bg-neutral-100 focus-visible:outline-2 focus-visible:outline-neutral-500 dark:hover:bg-neutral-800" class:border-neutral-900={cwd === workspace.cwd} class:bg-neutral-100={cwd === workspace.cwd} class:dark:border-neutral-100={cwd === workspace.cwd} onclick={() => selectWorkspace(workspace.cwd)}><span class="block truncate">{workspace.label}</span>{#if workspace.secondary}<span class="block max-w-56 truncate text-xs text-neutral-500">{workspace.secondary}</span>{/if}</button>
{/each}
{#if selectedTemporaryWorkspace}<button type="button" aria-pressed="true" class="min-h-9 max-w-full rounded-full border border-neutral-900 bg-neutral-100 px-3 text-left text-sm focus-visible:outline-2 focus-visible:outline-neutral-500 dark:border-neutral-100 dark:hover:bg-neutral-800" onclick={() => selectWorkspace(selectedTemporaryWorkspace.cwd)}>임시 작업공간 선택됨</button>{/if}
</div>
{:else if !workspaces.temporary.length}<p class="text-sm text-neutral-500">최근 작업공간이 없습니다.</p>{/if}
<details class="rounded-2xl border border-neutral-200 bg-white px-4 dark:border-neutral-800 dark:bg-neutral-900">
<summary class="flex min-h-11 cursor-pointer list-none items-center justify-between text-sm font-medium [&::-webkit-details-marker]:hidden">경로 및 격리 설정<span aria-hidden="true" class="text-neutral-500">⌄</span></summary>
<div class="flex flex-col gap-3 border-t border-neutral-200 py-3 dark:border-neutral-800">
<label for="custom-cwd" class="text-xs font-medium text-neutral-600 dark:text-neutral-300">사용자 지정 절대 경로</label>
<input id="custom-cwd" aria-label="사용자 지정 절대 경로" type="text" value={cwd} oninput={updateCustomCwd} autocomplete="off" autocapitalize="off" spellcheck="false" placeholder="호스트의 절대 경로" onkeydown={(event) => event.key === 'Enter' && event.preventDefault()} class="min-h-10 w-full min-w-0 rounded-xl border border-neutral-300 bg-white px-3 font-mono text-sm outline-none focus:border-neutral-500 dark:border-neutral-700 dark:bg-neutral-950 dark:focus:border-neutral-500" />
<p class="-mt-2 text-xs text-neutral-500">경로는 연결된 호스트에서 사용합니다.</p>
<label class="flex min-h-9 items-center gap-2 text-sm text-neutral-700 dark:text-neutral-300"><input type="checkbox" bind:checked={worktree} disabled={!cwd.trim()} aria-describedby="worktree-help" class="size-4 accent-neutral-900 disabled:cursor-not-allowed disabled:opacity-50 dark:accent-neutral-100" />worktree로 격리</label>
<p id="worktree-help" class="-mt-2 text-xs text-neutral-500 dark:text-neutral-400">저장소 경로에서 새 worktree를 만들고 그 안에서 시작합니다.</p>
{#if worktree}<label for="wt-name" class="text-xs font-medium text-neutral-600 dark:text-neutral-300">worktree 이름</label><input id="wt-name" aria-label="worktree 이름" bind:value={wtName} required maxlength="64" pattern={'[A-Za-z0-9_\\-]+'} oninvalid={(event) => event.currentTarget.closest('details')?.setAttribute('open', '')} autocomplete="off" placeholder="예: feature-update" class="min-h-10 w-full min-w-0 rounded-xl border border-neutral-300 bg-white px-3 text-sm dark:border-neutral-700 dark:bg-neutral-950" />{/if}
</div>
</details>
{#if workspaces.temporary.length}
<details open={workspaceOpen} ontoggle={(event) => (workspaceOpen = (event.currentTarget as HTMLDetailsElement).open)} class="rounded-xl border border-neutral-200 bg-white px-3 dark:border-neutral-800 dark:bg-neutral-900">
<summary class="flex min-h-10 cursor-pointer list-none items-center justify-between text-sm font-medium [&::-webkit-details-marker]:hidden">임시 작업공간 {workspaces.temporary.length}개<span aria-hidden="true" class="text-neutral-500">⌄</span></summary>
<div class="flex flex-wrap gap-2 border-t border-neutral-200 py-3 dark:border-neutral-800">
{#each workspaces.temporary as workspace (workspace.cwd)}
<button type="button" aria-pressed={cwd === workspace.cwd} title={workspace.cwd} class="min-h-9 max-w-full rounded-full border px-3 text-left text-sm hover:bg-neutral-100 focus-visible:outline-2 focus-visible:outline-neutral-500 dark:hover:bg-neutral-800" class:border-neutral-900={cwd === workspace.cwd} class:bg-neutral-100={cwd === workspace.cwd} class:dark:border-neutral-100={cwd === workspace.cwd} onclick={() => selectWorkspace(workspace.cwd)}><span class="block truncate">{workspace.label}</span>{#if workspace.secondary}<span class="block max-w-56 truncate text-xs text-neutral-500">{workspace.secondary}</span>{/if}</button>
{/each}
</div>
</details>
{/if}
</section>
</div>
</section>
<div class="shrink-0 px-4 pb-[max(0.875rem,env(safe-area-inset-bottom))] md:px-6">
<div class="mx-auto flex w-full max-w-[808px] flex-col gap-2 rounded-[22px] border border-neutral-300 bg-white p-3 pl-[18px] shadow-sm dark:border-neutral-700 dark:bg-neutral-900">
<label for="prompt" class="sr-only">작업 지시 입력</label>
<textarea id="prompt" bind:this={promptElement} bind:value={prompt} oninput={() => { touched = true; }} onpaste={pasteAttachments} rows="2" placeholder="무엇을 시킬까요? 예: 테스트 실패 원인 찾아서 고쳐줘" class="block max-h-40 min-h-12 w-full resize-y bg-transparent p-0 text-[15px] leading-6 outline-none placeholder:text-neutral-400"></textarea>
{#if attachments.length > 0}<AttachmentPreview items={previewItems} removable onremove={(key) => (attachments = attachments.filter((item) => item.key !== key))} />{/if}
{#if attachmentError}<p role="alert" class="text-sm text-red-700 dark:text-red-300">{attachmentError}</p>{/if}
<div class="flex flex-wrap items-center gap-2">
<div class="min-w-0 max-w-72 flex-[1_1_12rem]"><ModelSelect id="new-model" bind:value={modelId} initialFromCatalog compact host={ownerHost || undefined} onselect={() => { touched = true; }} /></div>
<label class="flex size-9 shrink-0 cursor-pointer items-center justify-center rounded-xl hover:bg-neutral-100 dark:hover:bg-neutral-800" title="이미지 또는 텍스트 파일 첨부"><Icon name="image" /><span class="sr-only">이미지 또는 TXT/MD 파일 첨부</span><input type="file" accept="image/*,.txt,.md,text/plain,text/markdown" multiple class="sr-only" onchange={attachFiles} /></label>
<select id="mode" aria-label="권한 모드" bind:value={mode} onchange={() => { touched = true; }} class="min-h-9 min-w-0 max-w-32 flex-[0.8_1_7rem] truncate rounded-xl border border-neutral-300 bg-transparent px-2 text-xs outline-none focus-visible:ring-2 focus-visible:ring-neutral-500 dark:border-neutral-700">{#each MODES as m (m)}<option value={m}>{MODE_LABEL[m]}</option>{/each}</select>
<div class="flex min-w-0 flex-1 flex-col text-xs text-neutral-500"><span class="truncate" title={ownerHost}>{ownerHost || '호스트 확인 중'}</span><span>{board.connected ? '서버 연결됨' : '서버 연결 중'}</span></div>
<button type="submit" disabled={busy || attachmentReads > 0} class="flex h-10 w-10 shrink-0 items-center justify-center gap-2 rounded-xl bg-neutral-900 text-sm font-medium text-white disabled:opacity-40 dark:bg-neutral-100 dark:text-neutral-900" aria-label="작업 시작"><Icon name="send" class="size-4" /><span class="sr-only">시작</span></button>
</div>
{#if error}<p role="alert" class="text-sm text-red-700 dark:text-red-300">{error}</p>{/if}
</div>
</div>
</form>
