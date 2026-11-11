<script lang="ts">
	import { goto } from '$app/navigation';
	import { postJson } from '$lib/api';
	import { loadDefaultMode, loadRecent, MODES, MODE_LABEL, rememberCwd } from '$lib/prefs';
	import type { Mode, NewSession } from '$lib/types';

	let cwd = $state('');
	let worktree = $state(true);
	let wtName = $state('');
	let mode = $state<Mode>(loadDefaultMode());
	let prompt = $state('');
	let error = $state('');
	let busy = $state(false);
	let recent = $state<string[]>(loadRecent());

	async function submit(e: SubmitEvent) {
		e.preventDefault();
		error = '';
		const dir = cwd.trim();
		if (!dir) {
			error = '작업 경로를 입력하세요.';
			return;
		}
		const body: NewSession = { cwd: dir, mode };
		if (worktree) body.worktree = { name: wtName.trim() };
		if (prompt.trim()) body.prompt = prompt.trim();
		busy = true;
		try {
			const res = await postJson<{ sessionId: string }>('/api/sessions', body);
			rememberCwd(dir);
			await goto(`/s/${encodeURIComponent(res.sessionId)}`);
		} catch {
			error = '세션을 만들지 못했습니다. 연결을 확인하세요.';
		} finally {
			busy = false;
		}
	}
</script>

<svelte:head>
	<title>새 작업 · Postudio</title>
</svelte:head>

<form class="flex flex-col gap-4 p-4" onsubmit={submit}>
	<h1 class="text-base font-semibold">새 작업</h1>

	<div class="flex flex-col gap-2">
		<label for="cwd" class="text-sm font-medium">작업 경로</label>
		<input
			id="cwd"
			bind:value={cwd}
			list="recent-cwds"
			autocomplete="off"
			autocapitalize="off"
			spellcheck="false"
			placeholder="/home/user/project"
			class="min-h-11 rounded-lg border border-neutral-300 bg-white px-3 font-mono text-sm dark:border-neutral-700 dark:bg-neutral-900"
		/>
		<datalist id="recent-cwds">
			{#each recent as r (r)}
				<option value={r}></option>
			{/each}
		</datalist>
		{#if recent.length > 0}
			<div class="flex flex-wrap gap-2">
				{#each recent as r (r)}
					<button
						type="button"
						class="min-h-11 rounded-full border border-neutral-300 px-3 font-mono text-xs dark:border-neutral-700"
						onclick={() => (cwd = r)}
					>
						{r}
					</button>
				{/each}
			</div>
		{/if}
	</div>

	<div class="flex flex-col gap-2">
		<label class="flex min-h-11 items-center gap-3 text-sm font-medium">
			<input type="checkbox" bind:checked={worktree} class="size-5" />
			worktree로 격리
		</label>
		{#if worktree}
			<label for="wt-name" class="text-sm font-medium">worktree 이름</label>
			<input
				id="wt-name"
				bind:value={wtName}
				autocomplete="off"
				placeholder="비우면 자동 이름"
				class="min-h-11 rounded-lg border border-neutral-300 bg-white px-3 text-sm dark:border-neutral-700 dark:bg-neutral-900"
			/>
		{/if}
	</div>

	<div class="flex flex-col gap-2">
		<label for="mode" class="text-sm font-medium">권한 모드</label>
		<select
			id="mode"
			bind:value={mode}
			class="min-h-11 rounded-lg border border-neutral-300 bg-white px-3 text-sm dark:border-neutral-700 dark:bg-neutral-900"
		>
			{#each MODES as m (m)}
				<option value={m}>{MODE_LABEL[m]}</option>
			{/each}
		</select>
	</div>

	<div class="flex flex-col gap-2">
		<label for="prompt" class="text-sm font-medium">프롬프트</label>
		<textarea
			id="prompt"
			bind:value={prompt}
			rows="4"
			class="rounded-lg border border-neutral-300 bg-white p-2 text-base dark:border-neutral-700 dark:bg-neutral-900"
		></textarea>
	</div>

	{#if error}
		<p role="alert" class="text-sm text-red-700 dark:text-red-300">{error}</p>
	{/if}

	<button
		type="submit"
		disabled={busy}
		class="min-h-11 rounded-lg bg-blue-700 px-4 text-sm font-medium text-white disabled:opacity-40"
	>
		시작
	</button>
</form>
