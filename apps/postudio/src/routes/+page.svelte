<script lang="ts">
	import { goto } from '$app/navigation';
	import Badge from '$lib/components/Badge.svelte';
	import { activityBadge, basename, sessionTitle } from '$lib/format';
	import { board, sortRows } from '$lib/board.svelte';

	const rows = $derived(sortRows(board.rows));
</script>

<svelte:head>
	<title>세션 · Postudio</title>
</svelte:head>

<section class="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto p-4">
	<div class="flex items-center gap-3">
		<h1 class="min-w-0 flex-1 truncate text-base font-semibold">{board.host || '호스트'}</h1>
		<button
			type="button"
			class="min-h-11 rounded-lg bg-blue-700 px-4 text-sm font-medium text-white hover:bg-blue-800"
			onclick={() => goto('/new')}
		>
			새 작업
		</button>
	</div>

	{#if !board.connected}
		<p role="status" class="text-sm text-neutral-500">서버에 연결하는 중입니다.</p>
	{/if}

	{#if board.connected && rows.length === 0}
		<p class="text-sm text-neutral-500">세션이 없습니다. 새 작업을 시작하세요.</p>
	{/if}

	<ul class="flex flex-col gap-2">
		{#each rows as row (row.sessionId)}
			{@const badge = activityBadge(row.activity)}
			<li>
				<a
					href="/s/{row.sessionId}"
					class="flex min-h-11 flex-col gap-1 rounded-lg border border-neutral-300 bg-white p-3 hover:border-blue-500 dark:border-neutral-700 dark:bg-neutral-900"
				>
					<span class="flex items-center gap-2">
						<span class="min-w-0 flex-1 truncate font-medium">{sessionTitle(row)}</span>
						{#if row.isWorktree}
							<span class="rounded border border-neutral-400 px-1 text-xs text-neutral-600 dark:text-neutral-300" title="worktree">WT</span>
						{/if}
						<Badge label={badge.label} tone={badge.tone} />
					</span>
					<span class="truncate font-mono text-xs text-neutral-500" title={row.cwd}>{basename(row.cwd)} · {row.cwd}</span>
				</a>
			</li>
		{/each}
	</ul>
</section>
