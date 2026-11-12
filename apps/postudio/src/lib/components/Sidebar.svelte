<script lang="ts">
import { afterNavigate, goto } from '$app/navigation';
import { page } from '$app/state';
import { onMount } from 'svelte';
import { getJson, postJson } from '$lib/api';
import Badge from '$lib/components/Badge.svelte';
import Icon from '$lib/components/Icon.svelte';
import AccountSummary from '$lib/components/AccountSummary.svelte';
import { activityBadge } from '$lib/format';
import { sessionHref, sessionOrigin } from '$lib/fleet';
import { board, sortRows } from '$lib/board.svelte';
import { groupWorkspaceRows, isTemporaryWorkspace, visibleWorkspaceRows, type WorkspaceSession } from '$lib/workspaces';
import { chatDisplayTitle, chatPreviewTitles, chatTitleOverrides, parseChatList, parseCreatedChat, type ChatSummary } from '$lib/chats';
import { ui } from '$lib/ui.svelte';
import type { BoardRow } from '$lib/types';

const chatMode = $derived(page.url.pathname.startsWith('/c'));
const rows = $derived(sortRows(board.rows));
const activeHost = $derived.by(() => {
try { return page.url.pathname.startsWith('/s/') ? sessionOrigin(page.url.search) ?? page.url.origin : board.origin; }
catch { return ''; }
});
let pinnedRow = $state.raw<BoardRow | undefined>();
let pinnedHostResponding = $state(false);
const selectedRow = $derived(rows.find((row) => row.sessionId === page.params.id && rowOrigin(row) === activeHost) ?? (pinnedRow && pinnedRow.sessionId === page.params.id && pinnedRow.apiOrigin === activeHost ? pinnedRow : undefined));
const selectedWorkspaceName = $derived(selectedRow?.cwd?.split(/[\\/]/).filter(Boolean).at(-1) || (page.url.pathname === '/new' ? '새 작업' : page.url.pathname.startsWith('/s/') ? '작업공간' : 'Postudio'));
const selectedHostConnected = $derived(activeHost === board.origin ? board.connected : !!pinnedRow && pinnedRow.apiOrigin === activeHost && pinnedRow.sessionId === page.params.id && pinnedHostResponding);
$effect(() => {
const sid = page.url.pathname.startsWith('/s/') ? page.params.id : undefined;
const host = activeHost;
pinnedRow = undefined; pinnedHostResponding = false;
if (!sid || !host || host === board.origin) return;
let current = true;
const refresh = async () => {
try {
const result = await getJson<{ sessions: BoardRow[] }>('/api/sessions', host);
if (!current) return;
const selected = result.sessions.find((item) => item.sessionId === sid);
pinnedRow = selected ? { ...selected, apiOrigin: host } : undefined;
pinnedHostResponding = true;
} catch { if (current) pinnedHostResponding = false; }
};
void refresh();
const timer = setInterval(() => void refresh(), 2000);
return () => { current = false; clearInterval(timer); };
});
let showTemporary = $state(false);
const temporaryRows = $derived(groupWorkspaceRows(rows)
.filter((group) => isTemporaryWorkspace(group.cwd ?? ''))
.flatMap((group) => group.rows));
const visibleRows = $derived(visibleWorkspaceRows(rows, showTemporary, page.params.id, activeHost, board.origin));
const groups = $derived(groupWorkspaceRows(visibleRows));
let openGroups = $state<Record<string, boolean>>({});
let chats = $state.raw<ChatSummary[]>([]);
let chatListLoading = $state(true);
let chatListLoaded = $state(false);
let chatListError = $state('');
let createError = $state('');
let creatingChat = $state(false);
let chatRequestId = 0;

const activeGroupId = $derived(groups.find((group) => group.rows.some((row) => page.params.id === row.sessionId && rowOrigin(row) === activeHost))?.id);
$effect(() => {
const activeId = page.params.id;
const groupId = activeGroupId;
if (activeId && groupId) openGroups[groupId] = true;
});

async function refreshChatList() {
const request = ++chatRequestId;
if (!chatListLoaded) chatListLoading = true;
try {
const next = parseChatList(await getJson<unknown>('/api/chats'));
if (request !== chatRequestId) return;
chats = next;
chatListLoaded = true;
chatListError = '';
} catch (cause) {
if (request === chatRequestId) chatListError = cause instanceof Error ? cause.message : '알 수 없는 오류';
} finally {
if (request === chatRequestId) chatListLoading = false;
}
}

async function createChat() {
if (creatingChat) return;
creatingChat = true;
createError = '';
try {
const chat = parseCreatedChat(await postJson<unknown>('/api/chats', {}));
await goto(`/c/${encodeURIComponent(chat.chatId)}`);
} catch (cause) {
createError = cause instanceof Error ? cause.message : '알 수 없는 오류';
} finally {
creatingChat = false;
}
}

afterNavigate(() => void refreshChatList());
onMount(() => {
const timer = window.setInterval(() => void refreshChatList(), 15000);
return () => window.clearInterval(timer);
});

function setGroupOpen(id: string, event: Event) {
openGroups[id] = (event.currentTarget as HTMLDetailsElement).open;
}

function close() {
ui.drawerOpen = false;
}

function rowOrigin(row: typeof board.rows[number]): string {
return row.apiOrigin ?? board.origin;
}

function containsActiveSession(item: WorkspaceSession): boolean {
return (item.row.sessionId === page.params.id && rowOrigin(item.row) === activeHost)
|| item.children.some(containsActiveSession);
}

function portsHref(): string {
const origin = board.origin;
return origin && origin !== (typeof location === 'undefined' ? '' : location.origin) ? `/ports?host=${encodeURIComponent(origin)}` : '/ports';
}
</script>

{#snippet sessionItem(item: WorkspaceSession)}
{@const row = item.row}
{@const active = page.params.id === row.sessionId && rowOrigin(row) === activeHost}
{@const badge = activityBadge(row.activity)}
<div class="flex flex-col gap-0.5">
<a href={sessionHref(row.sessionId, rowOrigin(row))} onclick={close} aria-current={active ? 'page' : undefined} class={['flex min-h-9 flex-col justify-center gap-1 rounded-lg px-3 py-1.5 text-neutral-800 hover:bg-neutral-200/70 dark:text-neutral-200 dark:hover:bg-neutral-800', active && 'bg-neutral-200 dark:bg-neutral-800']}>
<span class="flex items-center gap-2"><span class="min-w-0 flex-1 truncate text-sm">{row.title || '제목 없는 작업'}</span>{#if row.isWorktree}<span class="rounded px-1 text-[10px] text-neutral-600 ring-1 ring-neutral-300 dark:text-neutral-300 dark:ring-neutral-700" title="worktree">WT</span>{/if}</span>
<span class="flex items-center gap-2"><span title={row.activity === 'dormant' ? '저장된 작업이며 현재 실행 프로세스가 없습니다.' : row.activity === 'idle' ? '상주 프로세스가 있으나 실행 중인 턴이 없습니다.' : row.activity === 'needs_input' ? '권한·질문·계획 응답 대기' : undefined}><Badge label={badge.label} tone={badge.tone} /></span>{#if Number.isFinite(row.lastChangeUnixMs) && row.lastChangeUnixMs > 0}<time class="ml-auto text-[11px] text-neutral-600 dark:text-neutral-300" datetime={new Date(row.lastChangeUnixMs).toISOString()} title={new Date(row.lastChangeUnixMs).toLocaleString('ko-KR')}>{new Date(row.lastChangeUnixMs).toLocaleString('ko-KR', { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' })}</time>{/if}</span>
</a>
{#if item.children.length}
<details open={item.children.some(containsActiveSession)} class="ml-3 rounded-lg px-2 text-xs text-neutral-600 dark:text-neutral-300">
<summary class="min-h-8 cursor-pointer content-center">하위 에이전트 · {item.children.length}</summary>
<div class="flex flex-col gap-0.5 border-l border-neutral-200 pl-1 dark:border-neutral-700">
{#each item.children as child (JSON.stringify([rowOrigin(child.row), child.row.sessionId]))}{@render sessionItem(child)}{/each}
</div>
</details>
{/if}
</div>
{/snippet}

<svelte:window onfocus={refreshChatList} />

<header class="flex min-h-[60px] shrink-0 items-center justify-between gap-2 px-3">
{#if chatMode}<span class="text-base font-bold">AI 채팅</span>{:else}<div class="min-w-0 flex-1"><div class="flex min-w-0 items-center gap-2 text-sm font-semibold"><svg class="size-4 shrink-0" aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><path d="M3 6h7l2 2h9v11H3z" /></svg><span class="truncate" title={selectedRow?.cwd ?? selectedWorkspaceName}>{selectedWorkspaceName}</span>{#if selectedRow?.activity === 'working'}<span class="size-3 shrink-0 animate-spin rounded-full border-2 border-neutral-400 border-t-neutral-800 motion-reduce:animate-none" role="img" aria-label="작업 실행 중"></span>{/if}</div><div class="mt-1 truncate pl-6 text-[11.5px] text-neutral-500 dark:text-neutral-400" title={selectedRow ? rowOrigin(selectedRow) : activeHost}>{selectedRow ? rowOrigin(selectedRow) : activeHost || '호스트 확인 필요'} · {selectedHostConnected ? '연결됨' : '연결 중'}</div></div>{/if}
{#if !chatMode && page.url.pathname.startsWith('/s/') && selectedRow && activeHost}
<a href="/s/{encodeURIComponent(selectedRow.sessionId)}/artifacts{page.url.search}" onclick={close} class="shrink-0 rounded-lg px-2 py-1.5 text-xs text-neutral-600 hover:bg-neutral-200/70 dark:text-neutral-300 dark:hover:bg-neutral-800">아티팩트</a>
{/if}
<button type="button" class="hidden size-9 shrink-0 items-center justify-center rounded-lg text-neutral-600 hover:bg-neutral-200/70 focus-visible:outline-2 focus-visible:outline-neutral-500 lg:flex dark:hover:bg-neutral-800" aria-label="사이드바 접기" title="사이드바 접기" onclick={() => ui.toggleSidebar()}><Icon name="sidebar" class="size-[18px]" /></button>
</header>

<nav aria-label="작업 모드" class="mx-3 flex h-10 shrink-0 gap-[2px] rounded-[11px] bg-[#e8e8eb] p-[3px] dark:bg-neutral-800">
<a href="/c" onclick={close} aria-current={chatMode ? 'page' : undefined} class={['flex flex-1 items-center justify-center gap-2 rounded-lg text-[13.5px] focus-visible:outline-2 focus-visible:outline-neutral-500', chatMode ? 'bg-white font-semibold text-neutral-900 shadow-sm dark:bg-neutral-950 dark:text-white' : 'font-medium text-neutral-600 dark:text-neutral-300']}><Icon name="chat" class="size-4" />채팅</a>
<a href="/new" onclick={close} aria-current={!chatMode ? 'page' : undefined} class={['flex flex-1 items-center justify-center gap-2 rounded-lg text-[13.5px] focus-visible:outline-2 focus-visible:outline-neutral-500', !chatMode ? 'bg-white font-semibold text-neutral-900 shadow-sm dark:bg-neutral-950 dark:text-white' : 'font-medium text-neutral-600 dark:text-neutral-300']}><Icon name="terminal" class="size-4" />작업</a>
</nav>

<div class="flex shrink-0 items-center gap-1 px-3 pt-2">
<button type="button" disabled={chatMode && creatingChat} class="flex h-10 flex-1 items-center gap-2 rounded-lg px-3 text-left text-sm font-medium hover:bg-neutral-200/70 focus-visible:outline-2 focus-visible:outline-neutral-500 disabled:opacity-50 dark:hover:bg-neutral-800" onclick={() => chatMode ? createChat() : goto('/new')}><Icon name="new" class="size-4" />{chatMode ? (creatingChat ? '채팅 만드는 중…' : '새 채팅') : '새 작업'}</button>
<button type="button" class="flex size-9 items-center justify-center rounded-lg hover:bg-neutral-200/70 focus-visible:outline-2 focus-visible:outline-neutral-500 lg:hidden dark:hover:bg-neutral-800" aria-label="메뉴 닫기" onclick={close}><Icon name="close" /></button>
</div>

{#if chatMode}
<nav aria-label="채팅 목록" class="flex shrink-0 flex-col px-3 pt-1">
<a href="/c" onclick={close} aria-current={page.route.id === '/c' || page.route.id === '/c/[id]' ? 'page' : undefined} class="flex h-9 items-center gap-2 rounded-lg px-3 text-sm text-neutral-700 hover:bg-neutral-200/70 focus-visible:outline-2 focus-visible:outline-neutral-500 dark:text-neutral-300 dark:hover:bg-neutral-800"><Icon name="list" class="size-4" />채팅 목록</a>
</nav>
{/if}

<section aria-label="연구 도구" class="flex shrink-0 flex-col px-3 pt-2">
<h2 class="px-2 py-1 text-xs font-semibold text-neutral-500">연구 도구</h2>
{#each [{label:'캘린더',icon:'calendar'}, {label:'논문 서지 관리',icon:'book'}, {label:'LaTeX 논문 에디터',icon:'document'}, {label:'회의 PPT 만들기',icon:'presentation'}] as tool}
<button type="button" disabled title="아직 사용할 수 없습니다" aria-describedby="research-tools-unavailable" class="flex h-9 cursor-not-allowed items-center gap-2 rounded-lg px-3 text-left text-sm text-neutral-500 opacity-70"><Icon name={tool.icon as 'calendar' | 'book' | 'document' | 'presentation'} class="size-4 shrink-0" />{tool.label}</button>
{/each}
<span id="research-tools-unavailable" class="sr-only">연구 도구는 아직 사용할 수 없습니다.</span>
</section>

<div class="flex min-h-0 flex-1 flex-col gap-1 overflow-y-auto px-3 pb-2 pt-2">
{#if chatMode}
<section aria-labelledby="chat-list-heading" class="flex flex-col gap-0.5">
<div class="flex h-9 items-center justify-between gap-2 px-2">
<h2 id="chat-list-heading" class="text-xs font-semibold text-neutral-600 dark:text-neutral-300">최근 채팅</h2>
<a href="/c" onclick={close} class="rounded px-1 py-1 text-xs text-neutral-600 hover:bg-neutral-200/70 focus-visible:outline-2 focus-visible:outline-neutral-500 dark:text-neutral-300 dark:hover:bg-neutral-800">모두 보기</a>
</div>
{#if chatListLoading && !chatListLoaded}<p role="status" class="px-3 py-1 text-xs text-neutral-600 dark:text-neutral-300">채팅 목록을 불러오는 중입니다.</p>{:else if chatListLoaded && chats.length === 0}<p class="px-3 py-1 text-xs text-neutral-600 dark:text-neutral-300">채팅이 없습니다.</p>{/if}
{#if chatListError}<div class="mx-2 rounded-lg bg-red-50 px-3 py-2 text-xs text-red-800 dark:bg-red-950 dark:text-red-200"><p role="alert">채팅 목록을 불러오지 못했습니다.</p><button type="button" class="min-h-8 underline underline-offset-4" onclick={() => void refreshChatList()}>다시 시도</button></div>{/if}
{#if createError}<p role="alert" class="mx-2 rounded-lg bg-red-50 px-3 py-2 text-xs text-red-800 dark:bg-red-950 dark:text-red-200">채팅을 만들지 못했습니다. {createError}</p>{/if}
{#each chats as chat (chat.chatId)}<a href="/c/{encodeURIComponent(chat.chatId)}" onclick={close} aria-current={page.route.id === '/c/[id]' && page.params.id === chat.chatId ? 'page' : undefined} class={['flex h-9 items-center gap-2 rounded-lg px-3 text-sm text-neutral-700 hover:bg-neutral-200/70 focus-visible:outline-2 focus-visible:outline-neutral-500 dark:text-neutral-300 dark:hover:bg-neutral-800', page.route.id === '/c/[id]' && page.params.id === chat.chatId && 'bg-neutral-200 dark:bg-neutral-800']}><Icon name="chat" class="size-4 shrink-0 text-neutral-500" /><span class="min-w-0 flex-1 truncate">{chatDisplayTitle(chat, [], $chatPreviewTitles, $chatTitleOverrides)}</span></a>{/each}
</section>
{:else}
<section aria-label="원격 연결" class="mb-3 flex flex-col gap-2 px-2 pt-2">
<h2 class="text-xs font-semibold text-neutral-500">원격 연결</h2>
<div class="flex min-w-0 items-center gap-2 text-xs"><Icon name="server" class="size-4 shrink-0" /><span class="min-w-0 flex-1 truncate" title={board.origin}>활성 호스트: {board.origin || '확인 중'}</span><span class="shrink-0" class:text-green-700={board.connected}>{board.connected ? '연결됨' : '연결 중'}</span></div>
</section>
<section aria-labelledby="coding-workspace-heading" class="flex flex-col gap-0.5">
<div class="flex h-9 items-center justify-between gap-2 px-2"><h2 id="coding-workspace-heading" class="text-xs font-semibold text-neutral-600 dark:text-neutral-300">워크스페이스</h2><a href={portsHref()} onclick={close} class="rounded px-1 py-1 text-xs text-neutral-600 hover:bg-neutral-200/70 focus-visible:outline-2 focus-visible:outline-neutral-500 dark:text-neutral-300 dark:hover:bg-neutral-800">포트</a></div>
{#if !board.connected}<p role="status" class="px-2 text-xs text-neutral-600 dark:text-neutral-300">서버에 연결하는 중입니다.</p>{:else if rows.length === 0}<p class="px-2 text-xs text-neutral-600 dark:text-neutral-300">코딩 작업이 없습니다.</p>{/if}
{#if temporaryRows.length}<button type="button" class="min-h-9 rounded-lg px-3 text-left text-xs text-neutral-600 hover:bg-neutral-200/70 dark:text-neutral-300" aria-expanded={showTemporary} onclick={() => (showTemporary = !showTemporary)}>{showTemporary ? '임시 작업 숨기기' : '임시 작업 보기'} · {temporaryRows.length}</button>{/if}
{#each groups as group (group.id)}<details class="group/workspace" open={openGroups[group.id] ?? !isTemporaryWorkspace(group.cwd ?? '')} ontoggle={(event) => setGroupOpen(group.id, event)}>
<summary class="flex min-h-9 cursor-pointer select-none items-center gap-2 rounded-lg px-2 text-xs font-medium text-neutral-700 hover:bg-neutral-200/70 dark:text-neutral-300 dark:hover:bg-neutral-800" title={group.title} aria-label={group.title && !isTemporaryWorkspace(group.title) ? `${group.label}. 전체 경로 ${group.title}` : group.label}><Icon name="chevron" class="size-3 shrink-0 transition-transform group-open/workspace:rotate-90" /><span class="min-w-0 flex-1 truncate">{group.label}</span>{#if group.secondary}<span class="max-w-24 truncate font-mono text-[10px] text-neutral-600 dark:text-neutral-300" title={group.title}>{group.secondary}</span>{/if}</summary>
{#if group.cwd && isTemporaryWorkspace(group.cwd)}<details class="mx-2 rounded-lg px-2 text-xs text-neutral-600 dark:text-neutral-300"><summary class="min-h-8 cursor-pointer content-center underline underline-offset-4">임시 작업공간의 실제 경로</summary><code class="block overflow-x-auto py-1 font-mono">{group.cwd}</code></details>{/if}
{#each group.sessions as item (JSON.stringify([rowOrigin(item.row), item.row.sessionId]))}{@render sessionItem(item)}{/each}</details>{/each}
</section>
{/if}
</div>

<footer class="flex shrink-0 items-center gap-2 border-t border-neutral-200 px-3 py-2 dark:border-neutral-800">
<AccountSummary />
<a href="/settings" onclick={close} aria-label="설정" title="설정" class="flex size-9 items-center justify-center rounded-lg text-neutral-600 hover:bg-neutral-200/70 focus-visible:outline-2 focus-visible:outline-neutral-500 dark:text-neutral-300 dark:hover:bg-neutral-800"><Icon name="settings" class="size-4" /></a>
</footer>
