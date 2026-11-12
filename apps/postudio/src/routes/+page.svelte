<script lang="ts">
import { board } from '$lib/board.svelte';
import Icon from '$lib/components/Icon.svelte';
import { ui } from '$lib/ui.svelte';
</script>

<svelte:head>
<title>Postudio</title>
</svelte:head>

<section class="mx-auto flex min-h-0 w-full max-w-3xl flex-1 flex-col justify-center gap-6 px-4 py-8 md:px-6">
<header class="text-center">
<h1 class="text-3xl font-semibold tracking-tight">Postudio</h1>
<p class="mt-2 text-sm text-neutral-600 dark:text-neutral-300">채팅을 저장하거나 코딩 작업을 시작하세요.</p>
</header>
<div class="grid gap-3 sm:grid-cols-2">
<a
 href="/c"
 class="flex min-h-32 flex-col justify-between rounded-2xl border border-neutral-300 bg-white p-5 shadow-sm hover:bg-neutral-100 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-neutral-500 dark:border-neutral-700 dark:bg-neutral-900 dark:hover:bg-neutral-800"
>
<span class="flex items-center gap-2 font-medium"><Icon name="chat" class="size-5" />채팅</span>
<span class="text-sm text-neutral-600 dark:text-neutral-300">서버에 저장된 대화를 보고 새 채팅을 만듭니다.</span>
</a>
<a
 href="/new"
 class="flex min-h-32 flex-col justify-between rounded-2xl border border-neutral-300 bg-white p-5 shadow-sm hover:bg-neutral-100 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-neutral-500 dark:border-neutral-700 dark:bg-neutral-900 dark:hover:bg-neutral-800"
>
<span class="flex items-center gap-2 font-medium"><Icon name="new" class="size-5" />새 코딩 작업</span>
<span class="text-sm text-neutral-600 dark:text-neutral-300">작업공간에서 코딩 세션을 시작합니다.</span>
</a>
</div>
<button
 type="button"
 class="min-h-11 self-center rounded-xl px-4 text-sm text-neutral-600 dark:text-neutral-300 underline-offset-4 hover:bg-neutral-100 hover:underline focus-visible:outline-2 focus-visible:outline-neutral-500 lg:hidden dark:hover:bg-neutral-800"
 onclick={() => (ui.drawerOpen = true)}
>
사이드바 열기
</button>
</section>
