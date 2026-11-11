<script lang="ts">
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { connect, type Socket } from '$lib/api';
	import Badge from '$lib/components/Badge.svelte';
	import Markdown from '$lib/components/Markdown.svelte';
	import ToolRow from '$lib/components/ToolRow.svelte';
	import { board } from '$lib/board.svelte';
	import { MODES, MODE_LABEL } from '$lib/prefs';
	import { sessionTitle, statusBadge } from '$lib/format';
	import { SessionView } from '$lib/session.svelte';
	import type { ClientMsg, Mode, Sep } from '$lib/types';

	const id = $derived(page.params.id ?? '');
	const view = new SessionView();
	const row = $derived(board.find(id));
	const title = $derived(view.title || (row ? sessionTitle(row) : id));
	const badge = $derived(statusBadge(view.status));

	let modeOverride = $state<Mode | null>(null);
	const mode = $derived<Mode>(modeOverride ?? (row?.yolo ? 'always-approve' : 'ask'));

	let sock: Socket | undefined;
	$effect(() => {
		const s = connect<Sep>(`/ws/session/${id}`, {
			onOpen: () => {
				view.reset();
				view.setConnected(true);
			},
			onClose: () => view.setConnected(false),
			onMessage: (m) => view.apply(m)
		});
		sock = s;
		return () => s.close();
	});

	function send(msg: ClientMsg) {
		sock?.send(msg);
	}

	let text = $state('');
	let images = $state<{ key: number; mime: string; data: string; url: string }[]>([]);
	let imageSeq = 0;
	let scroller = $state<HTMLElement>();

	const canSend = $derived(view.connected && !view.running && (text.trim() !== '' || images.length > 0));

	$effect(() => {
		void view.revision;
		scroller?.scrollTo({ top: scroller.scrollHeight });
	});

	function submit(e: SubmitEvent) {
		e.preventDefault();
		if (!canSend) return;
		send({
			t: 'prompt',
			text: text.trim(),
			images: images.map(({ mime, data }) => ({ mime, data }))
		});
		text = '';
		images = [];
	}

	function readDataUrl(file: File): Promise<string> {
		return new Promise((resolve, reject) => {
			const r = new FileReader();
			r.onload = () => resolve(String(r.result));
			r.onerror = () => reject(r.error);
			r.readAsDataURL(file);
		});
	}

	async function attach(e: Event) {
		const input = e.currentTarget as HTMLInputElement;
		for (const f of Array.from(input.files ?? [])) {
			if (!f.type.startsWith('image/')) continue;
			const url = await readDataUrl(f);
			const comma = url.indexOf(',');
			const mime = url.slice(5, url.indexOf(';'));
			images.push({ key: ++imageSeq, mime, data: url.slice(comma + 1), url });
		}
		input.value = '';
	}

	function cycleMode() {
		const next = MODES[(MODES.indexOf(mode) + 1) % MODES.length];
		if (next === 'always-approve' && !confirm('모든 작업을 승인 없이 실행합니다. 계속할까요?')) return;
		modeOverride = next;
		send({ t: 'mode', mode: next });
	}

	function optionLabel(kind: string, name: string): string {
		switch (kind) {
			case 'allow_once':
				return '허용';
			case 'allow_always':
				return '항상 허용';
			case 'reject_once':
				return '거부';
			default:
				return name;
		}
	}
</script>

<svelte:head>
	<title>{title} · Postudio</title>
</svelte:head>

<div class="flex min-h-0 flex-1 flex-col">
	<div class="flex items-center gap-2 border-b border-neutral-200 px-3 py-2 dark:border-neutral-800">
		<button
			type="button"
			class="min-h-11 min-w-11 rounded px-2 hover:bg-neutral-200 dark:hover:bg-neutral-800"
			aria-label="세션 목록으로"
			onclick={() => goto('/')}
		>
			←
		</button>
		<h1 class="min-w-0 flex-1 truncate text-sm font-semibold">{title}</h1>
		<Badge label={badge.label} tone={badge.tone} />
		{#if view.running}
			<button
				type="button"
				class="min-h-11 rounded-lg border border-red-500 px-3 text-sm text-red-700 hover:bg-red-50 dark:text-red-300 dark:hover:bg-red-950"
				onclick={() => send({ t: 'cancel' })}
			>
				중단
			</button>
		{/if}
	</div>

	{#if view.usage}
		<div class="flex items-center gap-2 px-3 py-1 text-xs text-neutral-600 dark:text-neutral-400">
			<span>사용량</span>
			<div class="h-2 flex-1 overflow-hidden rounded bg-neutral-200 dark:bg-neutral-800" role="presentation">
				<div
					class="h-full bg-blue-600"
					style:width="{Math.min(100, (view.usage.used / Math.max(1, view.usage.size)) * 100)}%"
				></div>
			</div>
			<span class="font-mono">{view.usage.used.toLocaleString()} / {view.usage.size.toLocaleString()}</span>
		</div>
	{/if}

	<div bind:this={scroller} class="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto p-3">
		{#if !view.connected}
			<p role="status" class="text-sm text-neutral-500">서버에 연결하는 중입니다.</p>
		{/if}
		{#each view.items as item (item.key)}
			{#if item.kind === 'tool'}
				<ToolRow {item} />
			{:else if item.role === 'user'}
				<div class="ml-auto max-w-[85%] rounded-2xl bg-blue-700 px-3 py-2 text-sm whitespace-pre-wrap text-white">
					{item.text}
				</div>
			{:else if item.role === 'assistant'}
				<div class="max-w-full"><Markdown text={item.text} /></div>
			{:else if item.role === 'reasoning'}
				<details class="text-xs text-neutral-500">
					<summary class="flex min-h-11 cursor-pointer items-center">추론 과정</summary>
					<p class="whitespace-pre-wrap">{item.text}</p>
				</details>
			{:else}
				<p role="alert" class="rounded-lg bg-red-50 p-3 text-sm text-red-900 dark:bg-red-950 dark:text-red-100">
					오류: {item.text}
				</p>
			{/if}
		{/each}
	</div>

	{#each view.permissions as p (p.toolCallId)}
		<div class="mx-3 mb-2 rounded-lg border border-amber-500 bg-amber-50 p-3 text-sm dark:bg-amber-950">
			<p class="mb-2 font-medium">승인 필요: {p.title}</p>
			<div class="flex flex-wrap gap-2">
				{#each p.options as opt (opt.id)}
					<button
						type="button"
						class="min-h-11 flex-1 rounded-lg border border-neutral-400 bg-white px-3 text-sm font-medium hover:bg-neutral-100 dark:bg-neutral-900 dark:hover:bg-neutral-800"
						onclick={() => send({ t: 'permission', id: p.id, option_id: opt.id })}
					>
						{optionLabel(opt.kind, opt.name)}
					</button>
				{/each}
			</div>
		</div>
	{/each}

	<form class="flex flex-col gap-2 border-t border-neutral-200 p-3 dark:border-neutral-800" onsubmit={submit}>
		{#if images.length > 0}
			<ul class="flex flex-wrap gap-2">
				{#each images as img (img.key)}
					<li class="relative">
						<img src={img.url} alt="첨부 이미지 미리보기" class="h-16 w-16 rounded border border-neutral-300 object-cover" />
						<button
							type="button"
							class="absolute -top-2 -right-2 flex size-11 items-center justify-center text-lg"
							aria-label="첨부 이미지 제거"
							onclick={() => (images = images.filter((i) => i.key !== img.key))}
						>
							<span class="flex size-6 items-center justify-center rounded-full bg-neutral-800 text-white">×</span>
						</button>
					</li>
				{/each}
			</ul>
		{/if}
		<label for="composer" class="sr-only">메시지</label>
		<textarea
			id="composer"
			bind:value={text}
			rows="2"
			placeholder="메시지 입력"
			class="min-h-11 resize-y rounded-lg border border-neutral-300 bg-white p-2 text-base dark:border-neutral-700 dark:bg-neutral-900"
		></textarea>
		<div class="flex items-center gap-2">
			<label class="flex min-h-11 cursor-pointer items-center rounded-lg border border-neutral-300 px-3 text-sm dark:border-neutral-700">
				<span>이미지</span>
				<input type="file" accept="image/*" class="sr-only" onchange={attach} />
			</label>
			<button
				type="button"
				class="min-h-11 rounded-lg border border-neutral-300 px-3 text-sm dark:border-neutral-700"
				aria-label="권한 모드: {MODE_LABEL[mode]}. 눌러서 변경"
				onclick={cycleMode}
			>
				{MODE_LABEL[mode]}
			</button>
			<button
				type="submit"
				disabled={!canSend}
				class="ml-auto min-h-11 rounded-lg bg-blue-700 px-5 text-sm font-medium text-white disabled:opacity-40"
			>
				보내기
			</button>
		</div>
	</form>
</div>
