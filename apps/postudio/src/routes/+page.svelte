<script lang="ts">
import { goto } from '$app/navigation';
import { board } from '$lib/board.svelte';
import Icon from '$lib/components/Icon.svelte';
import { ui } from '$lib/ui.svelte';
</script>

<svelte:head>
<title>Postudio</title>
</svelte:head>

<section class="flex min-h-0 flex-1 flex-col items-center justify-center gap-6 p-6">
<div class="flex flex-col items-center gap-2 text-center">
<h1 class="text-3xl font-semibold tracking-tight">무엇을 만들까요?</h1>
<p class="text-sm text-neutral-500">{board.host ? `${board.host}에서 작업을 시작합니다.` : '새 작업을 시작하세요.'}</p>
</div>
<button
type="button"
class="flex min-h-11 items-center gap-2 rounded-2xl border border-neutral-300 bg-white px-5 text-sm font-medium shadow-sm hover:bg-neutral-100 dark:border-neutral-700 dark:bg-neutral-900 dark:hover:bg-neutral-800"
onclick={() => goto('/new')}
>
<Icon name="new" class="size-4" />
새 작업
</button>
<button
type="button"
class="text-sm text-neutral-500 underline-offset-4 hover:underline lg:hidden"
onclick={() => (ui.drawerOpen = true)}
>
세션 목록 보기
</button>
</section>
