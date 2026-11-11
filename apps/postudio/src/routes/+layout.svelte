<script lang="ts">
import '../app.css';
import { page } from '$app/state';
import { board } from '$lib/board.svelte';
import Icon from '$lib/components/Icon.svelte';
import Sidebar from '$lib/components/Sidebar.svelte';
import { ui } from '$lib/ui.svelte';
import type { Snippet } from 'svelte';

let { children }: { children: Snippet } = $props();

board.start();

$effect(() => {
void page.url.pathname;
ui.drawerOpen = false;
});
</script>

<svelte:head>
<title>Postudio</title>
</svelte:head>

<svelte:window onkeydown={(e) => e.key === 'Escape' && (ui.drawerOpen = false)} />

<div class="flex h-dvh w-full overflow-hidden">
{#if ui.drawerOpen}
<button
type="button"
class="fixed inset-0 z-30 bg-black/40 lg:hidden"
aria-label="메뉴 닫기"
onclick={() => (ui.drawerOpen = false)}
></button>
{/if}

<aside
class={[
'fixed inset-y-0 left-0 z-40 flex w-72 flex-col bg-neutral-100 pt-[env(safe-area-inset-top)] transition-transform duration-200 dark:bg-neutral-900',
'lg:static lg:z-auto lg:shrink-0 lg:pt-0',
ui.drawerOpen ? 'translate-x-0' : '-translate-x-full',
ui.sidebarHidden ? 'lg:hidden' : 'lg:flex lg:translate-x-0'
]}
>
<Sidebar />
</aside>

<div class="flex min-w-0 flex-1 flex-col">
{#if page.route.id !== '/s/[id]'}
<header class="flex h-12 shrink-0 items-center gap-1 px-2 pt-[env(safe-area-inset-top)]">
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
<a href="/" class="ml-1 min-h-10 content-center text-sm font-semibold tracking-tight">Postudio</a>
</header>
{/if}
<main class="flex min-h-0 flex-1 flex-col">
{@render children()}
</main>
</div>
</div>
