<script lang="ts">
import '../app.css';
import { page } from '$app/state';
import { board } from '$lib/board.svelte';
import Icon from '$lib/components/Icon.svelte';
import Sidebar from '$lib/components/Sidebar.svelte';
import Snackbar from '$lib/components/Snackbar.svelte';
import { sessionOrigin } from '$lib/fleet';
import { ui } from '$lib/ui.svelte';
import { onMount, type Snippet } from 'svelte';

let { children }: { children: Snippet } = $props();

let desktop = $state(false);
let sidebar: HTMLElement;
let content: HTMLElement;
let returnFocus: HTMLElement | null = null;
const mode = $derived(page.url.pathname.startsWith('/c') ? 'chat' : 'work');
const sidebarVisible = $derived(desktop ? !ui.sidebarHidden : ui.drawerOpen);
const workContext = $derived.by(() => {
if (page.route.id !== '/s/[id]' || !ui.workContext || ui.workContext.sessionId !== page.params.id) return null;
try { return (sessionOrigin(page.url.search) ?? page.url.origin) === ui.workContext.host ? ui.workContext : null; }
catch { return null; }
});
let previousMode: string;

onMount(() => {
const media = window.matchMedia('(min-width: 1024px)');
const update = syncViewport;
update();
media.addEventListener('change', update);
return () => media.removeEventListener('change', update);
});

$effect(() => {
const next = mode;
if (previousMode && previousMode !== next && content) {
content.getAnimations().forEach((animation) => animation.cancel());
if (!window.matchMedia('(prefers-reduced-motion: reduce)').matches) {
content.animate([{ opacity: 0.65, transform: 'translateY(4px)' }, { opacity: 1, transform: 'translateY(0)' }], { duration: 180, easing: 'ease-out' });
}
}
previousMode = next;
});

$effect(() => {
if (!sidebarVisible && sidebar?.contains(document.activeElement)) {
document.querySelector<HTMLElement>(desktop ? '[aria-label="사이드바 펼치기"]' : '[aria-label="메뉴 열기"]')?.focus();
}
});

$effect(() => {
if (ui.drawerOpen && !desktop) {
returnFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
sidebar?.querySelector<HTMLElement>('a, button')?.focus();
} else if (returnFocus) {
if (sidebar?.contains(document.activeElement)) returnFocus.focus();
returnFocus = null;
}
});

function syncViewport() {
const next = window.matchMedia('(min-width: 1024px)').matches;
if (next !== desktop) {
desktop = next;
ui.drawerOpen = false;
}
}

function closeDrawer(event: KeyboardEvent) {
if (event.key === 'Escape') ui.drawerOpen = false;
if (event.key !== 'Tab' || desktop || !ui.drawerOpen) return;
const items = Array.from(sidebar.querySelectorAll<HTMLElement>('a[href], button:not(:disabled), summary')).filter((item) => item.getClientRects().length);
const first = items[0];
const last = items.at(-1);
if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus(); }
else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus(); }
}

board.start();

$effect(() => {
void page.url.pathname;
ui.drawerOpen = false;
});
</script>

<svelte:head>
<title>Postudio</title>
</svelte:head>

<svelte:window onkeydown={closeDrawer} onresize={syncViewport} />

<div class="flex h-dvh w-full overflow-hidden">
<button
 type="button"
 class="drawer-backdrop fixed inset-0 z-30 bg-black/40 lg:hidden"
 class:open={ui.drawerOpen}
 inert={!ui.drawerOpen || desktop}
 tabindex="-1"
 aria-label="메뉴 닫기"
 onclick={() => {
returnFocus?.focus();
ui.drawerOpen = false;
}}
></button>
<aside bind:this={sidebar} class="shell-sidebar fixed inset-y-0 left-0 z-40 bg-[#f7f7f7] lg:static lg:z-auto lg:shrink-0"
 class:drawer-open={ui.drawerOpen} class:collapsed={ui.sidebarHidden} inert={!sidebarVisible}>
<div class="sidebar-inner flex h-full w-[272px] flex-col pt-[env(safe-area-inset-top)] lg:pt-0">
<Sidebar />
</div>
</aside>

<div class="flex min-w-0 flex-1 flex-col">
{#if (page.route.id === '/s/[id]' && (!desktop || ui.sidebarHidden)) || (page.route.id !== '/s/[id]' && page.route.id !== '/c/[id]')}
<header class="flex h-9 shrink-0 items-center gap-1 border-b border-neutral-200 px-2 pt-[env(safe-area-inset-top)]">
<button
type="button"
class="flex size-10 items-center justify-center rounded-xl hover:bg-neutral-200/70 lg:hidden dark:hover:bg-neutral-800"
aria-label="메뉴 열기"
onclick={() => (ui.drawerOpen = true)}
>
<Icon name="menu" />
</button>
<button
type="button"
class="hidden size-10 items-center justify-center rounded-xl hover:bg-neutral-200/70 lg:flex dark:hover:bg-neutral-800"
aria-label={ui.sidebarHidden ? "사이드바 펼치기" : "사이드바 접기"}
title={ui.sidebarHidden ? "사이드바 펼치기" : "사이드바 접기"}
onclick={() => ui.toggleSidebar()}
>
<Icon name="sidebar" />
</button>
{#if workContext}
<span class="min-w-0 flex-1 truncate text-xs font-medium">{workContext.workspaceName} · {new URL(workContext.host).host}</span>
<a href="/s/{encodeURIComponent(workContext.sessionId)}/artifacts{page.url.search}" class="shrink-0 rounded px-2 py-1 text-xs">아티팩트</a>
{:else}<a href="/" aria-label="Postudio 홈" class="ml-1 min-h-10 content-center text-sm font-semibold tracking-tight">{page.url.pathname === '/settings' ? '설정' : page.url.pathname === '/new' ? '새 작업' : 'Postudio'}</a>{/if}
</header>
{/if}
<main bind:this={content} class="flex min-h-0 flex-1 flex-col">
{@render children()}
</main>
</div>
</div>
<Snackbar />

<style>
.shell-sidebar { width: 272px; transform: translateX(-100%); transition: transform 200ms ease, width 200ms ease; }
.shell-sidebar.drawer-open { transform: translateX(0); }
.drawer-backdrop { opacity: 0; visibility: hidden; pointer-events: none; transition: opacity 200ms ease, visibility 200ms; }
.drawer-backdrop.open { opacity: 1; visibility: visible; pointer-events: auto; }
@media (min-width: 1024px) {
.shell-sidebar { transform: none; overflow: hidden; }
.shell-sidebar.collapsed { width: 0; }
.sidebar-inner { transition: transform 200ms ease; }
.collapsed .sidebar-inner { transform: translateX(-272px); }
}
@media (prefers-reduced-motion: reduce) {
.shell-sidebar, .sidebar-inner, .drawer-backdrop { transition: none; }
}
</style>
