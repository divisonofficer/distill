<script lang="ts">
import { page } from '$app/state';
import { afterNavigate } from '$app/navigation';
import { onDestroy, tick } from 'svelte';
import { getJson } from '$lib/api';
import { sessionOrigin } from '$lib/fleet';
import { board } from '$lib/board.svelte';

type Quota = { status: 'available' | 'unavailable' | 'unsupported'; remaining_percent: number | null };
type Account = { id: string; provider: string; label: string; connected: boolean; email: string | null; quota: Quota | null };
let open = $state(false);
let loading = $state(false);
let error = $state(false);
let accounts = $state<Account[]>([]);
let request = 0;
let root = $state<HTMLDivElement>();
let trigger = $state<HTMLButtonElement>();
let panel = $state<HTMLDivElement>();
const host = $derived.by(() => {
 try { return page.url.pathname.startsWith('/s/') ? sessionOrigin(page.url.search) ?? page.url.origin : board.origin; }
 catch { return null; }
});
let repositionCleanup: (() => void) | undefined;
function stopTracking() { repositionCleanup?.(); repositionCleanup = undefined; }
function positionPanel() {
 const rect = trigger?.getBoundingClientRect();
 if (!rect || !panel?.matches(':popover-open')) return;
 const width = Math.min(320, Math.max(0, window.innerWidth - 24));
 const left = Math.max(12, Math.min(rect.left, window.innerWidth - width - 12));
 const height = Math.max(0, rect.top - 20);
 panel.style.left = `${left}px`;
 panel.style.top = 'auto';
 panel.style.bottom = `${window.innerHeight - rect.top + 8}px`;
 panel.style.width = `${width}px`;
 panel.style.maxHeight = `${height}px`;
}
function trackPosition() {
 stopTracking();
 const onScroll = (event: Event) => {
  if (panel?.contains(event.target as Node)) return;
  positionPanel();
 };
 const onResize = () => close();
 window.addEventListener('resize', onResize);
 window.addEventListener('scroll', onScroll, true);
 repositionCleanup = () => { window.removeEventListener('resize', onResize); window.removeEventListener('scroll', onScroll, true); };
}
function close(restoreFocus = false) {
 open = false;
 stopTracking();
 panel?.hidePopover();
 if (restoreFocus) void tick().then(() => trigger?.focus());
}
onDestroy(stopTracking);
function validAccounts(value: unknown): Account[] | null {
 if (!Array.isArray(value)) return null;
 const ids = new Set<string>();
 const result: Account[] = [];
 for (const item of value) {
  if (!item || typeof item !== 'object') return null;
  const row = item as Record<string, unknown>;
  if (typeof row.id !== 'string' || !row.id || ids.has(row.id) || typeof row.provider !== 'string' || typeof row.label !== 'string' || typeof row.connected !== 'boolean' || (row.email != null && typeof row.email !== 'string')) return null;
  ids.add(row.id);
  let quota: Quota | null = null;
  if (row.quota != null) {
   if (typeof row.quota !== 'object') return null;
   const raw = row.quota as Record<string, unknown>;
   if (!['available', 'unavailable', 'unsupported'].includes(String(raw.status))) return null;
   quota = { status: raw.status as Quota['status'], remaining_percent: typeof raw.remaining_percent === 'number' ? raw.remaining_percent : null };
  }
  result.push({ id: row.id, provider: row.provider, label: row.label, connected: row.connected, email: typeof row.email === 'string' ? row.email : null, quota });
 }
 return result;
}
async function refresh() {
 const current = ++request;
 const target = host;
 accounts = []; error = false;
 if (!target) { loading = false; error = true; return; }
 loading = true;
 try {
  const result = validAccounts(await getJson<unknown>('/api/accounts', target));
  if (current !== request || target !== host) return;
  if (!result) throw new Error('Invalid account response');
  accounts = result;
 } catch { if (current === request) error = true; }
 finally { if (current === request) loading = false; }
}
async function toggle() {
 if (open) { close(); return; }
 open = true;
 await tick();
 panel?.showPopover();
 positionPanel();
 panel?.focus();
 trackPosition();
}
$effect(() => { const currentHost = host; if (open) void refresh(); else { request++; accounts = []; loading = false; } void currentHost; });
afterNavigate(() => { close(); request++; accounts = []; });
function outside(event: MouseEvent) { if (open && root && !event.composedPath().includes(root)) close(); }
function keydown(event: KeyboardEvent) { if (event.key === 'Escape' && open) { event.preventDefault(); close(true); } }
</script>
<svelte:window onkeydown={keydown} onclick={outside} />
<div bind:this={root} data-account-summary class="relative min-w-0 flex-1">
<button bind:this={trigger} type="button" aria-label="계정 및 사용량" aria-expanded={open} aria-haspopup="dialog" aria-controls="account-summary-panel" title="계정 및 사용량" class="flex w-full min-w-0 items-center gap-2 rounded-lg focus-visible:outline-2 focus-visible:outline-neutral-500" onclick={toggle}>
<img src="/icon-192.png?v=ponix-character-33fbef92da78" alt="" class="size-8 shrink-0" /><span class="min-w-0 flex-1 truncate text-left text-sm font-medium">Postudio</span>
</button>
{#if open}
<div bind:this={panel} id="account-summary-panel" role="dialog" aria-label="계정 및 사용량" tabindex="-1" popover="manual" class="fixed z-[100] m-0 overflow-y-auto rounded-xl border border-neutral-200 bg-white p-3 shadow-xl dark:border-neutral-700 dark:bg-neutral-950">
<div class="mb-2 flex items-center justify-between"><h2 class="text-sm font-semibold">계정 및 사용량</h2><div class="flex items-center gap-2"><button type="button" class="rounded px-2 py-1 text-xs underline" onclick={() => void refresh()}>새로고침</button><button type="button" aria-label="닫기" class="rounded px-2 py-1 text-sm" onclick={() => close(true)}>닫기</button></div></div>
{#if loading}<p role="status" class="py-2 text-sm">계정 정보를 불러오는 중입니다.</p>{:else if error}<p role="alert" class="py-2 text-sm text-red-700 dark:text-red-300">계정 정보를 불러오지 못했습니다. <button type="button" class="underline" onclick={() => void refresh()}>다시 시도</button></p>{:else if accounts.length === 0}<p class="py-2 text-sm">등록된 계정이 없습니다.</p>{:else}
<ul class="flex flex-col gap-2">{#each accounts as account (account.id)}<li class="rounded-lg border border-neutral-200 p-3 text-sm dark:border-neutral-800"><p class="break-words font-medium">{account.label} <span class="font-normal text-neutral-500">({account.provider})</span></p><p class="break-all font-mono text-xs text-neutral-500">{account.id}</p><p><span class={account.connected ? 'text-green-700 dark:text-green-400' : 'text-neutral-500'}>{account.connected ? '연결됨' : '연결 안 됨'}</span>{account.email ? ` · ${account.email}` : ''}</p>{#if account.quota?.status === 'unsupported'}<p class="mt-2 text-xs text-neutral-500">사용량: 지원되지 않음</p>{:else if account.quota?.status === 'unavailable'}<p class="mt-2 text-xs text-neutral-500">사용량: 확인할 수 없음</p>{:else if account.quota?.status === 'available' && typeof account.quota.remaining_percent === 'number' && Number.isFinite(account.quota.remaining_percent) && account.quota.remaining_percent >= 0 && account.quota.remaining_percent <= 100}<div class="mt-2"><div class="flex justify-between text-xs"><span>사용량 {Math.round(100 - account.quota.remaining_percent)}%</span><span>잔여량 {Math.round(account.quota.remaining_percent)}%</span></div><progress class="mt-1 h-2 w-full accent-blue-600" max="100" value={100 - account.quota.remaining_percent} aria-label="사용량"></progress></div>{:else}<p class="mt-2 text-xs text-neutral-500">사용량 정보 없음</p>{/if}</li>{/each}</ul>
{/if}
</div>
{/if}
</div>
