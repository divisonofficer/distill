<script lang="ts">
import { goto } from '$app/navigation';
import { page } from '$app/state';
import { getJson } from '$lib/api';
import { sessionHref, sessionOrigin } from '$lib/fleet';
import { basename, sessionTitle } from '$lib/format';
import WorkbenchPane from '$lib/components/WorkbenchPane.svelte';
import Icon from '$lib/components/Icon.svelte';
import { ui } from '$lib/ui.svelte';
import type { BoardRow } from '$lib/types';

const id = $derived(page.params.id ?? '');
const destination = $derived.by(() => {
 try { return { host: sessionOrigin(page.url.search) ?? page.url.origin, error: '' }; }
 catch { return { host: page.url.origin, error: '호스트 주소가 설정 목록에 없거나 유효하지 않습니다. 설정에서 호스트를 확인하세요.' }; }
});
const apiHost = $derived(destination.host);
const hostError = $derived(destination.error);
let roster = $state.raw<BoardRow[]>([]);
let ready = $state(false);
let metadataError = $state('');
let generation = 0;
const row = $derived(roster.find((item) => item.sessionId === id));
const workspaceId = $derived(row?.cwd || `session:${id}`);
const title = $derived(row ? sessionTitle(row) : id);
const conversations = $derived.by(() => {
 const items = row?.cwd ? roster.filter((item) => item.cwd === row.cwd) : roster.filter((item) => item.sessionId === id);
 const result = items.map((item) => ({ sessionId: item.sessionId, title: sessionTitle(item) }));
 return result.some((item) => item.sessionId === id) ? result : [...result, { sessionId: id, title }];
});

$effect(() => {
 const host = apiHost, blocked = !!hostError, token = ++generation;
 roster = []; ready = blocked; metadataError = '';
 if (blocked) return;
 const refresh = async () => {
  try {
   const result = await getJson<{ sessions: BoardRow[] }>('/api/sessions', host);
   if (token !== generation) return;
   roster = result.sessions; metadataError = '';
  } catch {
   if (token === generation) metadataError = '작업공간 정보를 불러오지 못했습니다. 연결을 확인하세요.';
  } finally { if (token === generation) ready = true; }
 };
 void refresh();
 const timer = setInterval(() => void refresh(), 2000);
 return () => { clearInterval(timer); if (token === generation) generation++; };
});

$effect(() => {
 const route = id, host = apiHost, blocked = !!hostError;
 const sessionRow = roster.find((item) => item.sessionId === route);
 const context = !blocked ? { sessionId: route, host, workspaceName: basename(sessionRow?.cwd ?? '') || (sessionRow ? sessionTitle(sessionRow) : route) } : null;
 ui.workContext = context;
 return () => { if (context && ui.workContext === context) ui.workContext = null; };
});

function selectConversation(sessionId: string) {
 if (!destination.error && sessionId !== id) void goto(sessionHref(sessionId, destination.host), { noScroll: true, keepFocus: true });
}
</script>

<svelte:head><title>{title} · Postudio</title></svelte:head>

<div class="flex min-h-0 min-w-0 flex-1 flex-col">
 <h1 class="sr-only">{row?.cwd ? basename(row.cwd) : title}</h1>
 {#if destination.error}<p role="alert" class="shrink-0 px-4 py-2 text-sm text-red-700">{destination.error}</p>{/if}
 {#if metadataError}<p role="alert" class="shrink-0 px-4 py-2 text-sm text-red-700">{metadataError}</p>{/if}
 {#if ready}
  <WorkbenchPane sessionId={id} host={destination.host} {workspaceId} {conversations} sessionRows={roster} blocked={!!destination.error} onConversationSelect={selectConversation} />
 {:else}<div class="flex min-h-0 flex-1 flex-col"><nav class="flex h-10 shrink-0 items-center border-b border-neutral-200 px-2" aria-label="탐색"><button class="flex size-9 items-center justify-center rounded-xl hover:bg-neutral-200/70 lg:hidden" type="button" aria-label="메뉴 열기" onclick={() => (ui.drawerOpen = true)}><Icon name="menu" /></button>{#if ui.sidebarHidden}<button class="hidden size-9 items-center justify-center rounded-xl hover:bg-neutral-200/70 lg:flex" type="button" aria-label="사이드바 펼치기" title="사이드바 펼치기" onclick={() => ui.toggleSidebar()}><Icon name="sidebar" /></button>{/if}</nav><p role="status" class="p-6 text-sm text-neutral-500">작업공간을 불러오는 중입니다.</p></div>{/if}
</div>
