<script lang="ts">
	import '../app.css';
	import { board } from '$lib/board.svelte';
	import type { Snippet } from 'svelte';

	let { children }: { children: Snippet } = $props();

	board.start();
</script>

<svelte:head>
	<title>Postudio</title>
</svelte:head>

<div class="mx-auto flex h-dvh w-full max-w-2xl flex-col">
	<header class="flex items-center gap-3 border-b border-neutral-200 px-4 py-2 dark:border-neutral-800">
		<a href="/" class="flex min-h-11 items-center text-lg font-semibold">Postudio</a>
		<nav class="ml-auto flex items-center gap-1 text-sm">
			<a href="/" class="flex min-h-11 items-center rounded px-3 hover:bg-neutral-200 dark:hover:bg-neutral-800">세션</a>
			<a href="/settings" class="flex min-h-11 items-center rounded px-3 hover:bg-neutral-200 dark:hover:bg-neutral-800">설정</a>
		</nav>
	</header>
	<main class="flex min-h-0 flex-1 flex-col">
		{@render children()}
	</main>
</div>
