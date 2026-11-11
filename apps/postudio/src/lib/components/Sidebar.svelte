<script lang="ts">
import { page } from '$app/state';
import Badge from '$lib/components/Badge.svelte';
import Icon from '$lib/components/Icon.svelte';
import { activityBadge, basename, sessionTitle } from '$lib/format';
import { board, sortRows } from '$lib/board.svelte';
import { ui } from '$lib/ui.svelte';

const rows = $derived(sortRows(board.rows));

function close() {
	ui.drawerOpen = false;
}
</script>

<div class="flex items-center gap-2 px-3 pt-3 pb-2">
	<a
		href="/new"
		onclick={close}
		class="flex min-h-10 flex-1 items-center gap-2 rounded-xl px-3 text-sm font-medium hover:bg-neutral-200/70 dark:hover:bg-neutral-800"
	>
		<Icon name="new" class="size-4" />
		새 작업
	</a>
	<button
		type="button"
		class="flex size-10 items-center justify-center rounded-xl hover:bg-neutral-200/70 lg:hidden dark:hover:bg-neutral-800"
		aria-label="메뉴 닫기"
		onclick={close}
	>
		<Icon name="close" />
	</button>
	<button
		type="button"
		class="hidden size-10 items-center justify-center rounded-xl hover:bg-neutral-200/70 lg:flex dark:hover:bg-neutral-800"
		aria-label="사이드바 접기"
		title="사이드바 접기"
		onclick={() => ui.toggleSidebar()}
	>
		<Icon name="sidebar" />
	</button>
</div>

<div class="flex min-h-0 flex-1 flex-col gap-1 overflow-y-auto px-2 pb-3">
	<p class="px-2 pt-2 pb-1 text-xs font-medium text-neutral-500">{board.host || '세션'}</p>
	{#if !board.connected}
		<p role="status" class="px-2 text-xs text-neutral-500">서버에 연결하는 중입니다.</p>
	{:else if rows.length === 0}
		<p class="px-2 text-xs text-neutral-500">세션이 없습니다.</p>
	{/if}
	{#each rows as row (row.sessionId)}
		{@const badge = activityBadge(row.activity)}
		{@const active = page.params.id === row.sessionId}
		<a
			href="/s/{row.sessionId}"
			onclick={close}
			aria-current={active ? 'page' : undefined}
			class={[
				'flex flex-col gap-1 rounded-xl px-3 py-2 hover:bg-neutral-200/70 dark:hover:bg-neutral-800',
				active && 'bg-neutral-200 dark:bg-neutral-800'
			]}
		>
			<span class="flex items-center gap-2">
				<span class="min-w-0 flex-1 truncate text-sm">{sessionTitle(row)}</span>
				{#if row.isWorktree}
					<span class="rounded px-1 text-[10px] text-neutral-500 ring-1 ring-neutral-300 dark:ring-neutral-700" title="worktree">WT</span>
				{/if}
			</span>
			<span class="flex items-center gap-2">
				<Badge label={badge.label} tone={badge.tone} />
				<span class="min-w-0 flex-1 truncate font-mono text-[11px] text-neutral-500" title={row.cwd}>{basename(row.cwd)}</span>
			</span>
		</a>
	{/each}
</div>

<div class="border-t border-neutral-200 p-2 dark:border-neutral-800">
	<a
		href="/settings"
		onclick={close}
		class="flex min-h-10 items-center gap-2 rounded-xl px-3 text-sm hover:bg-neutral-200/70 dark:hover:bg-neutral-800"
	>
		<Icon name="sliders" class="size-4" />
		설정
	</a>
</div>
