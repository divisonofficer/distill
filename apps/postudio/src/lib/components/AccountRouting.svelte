<script module lang="ts">
export type RoutingStrategy = 'ordered_first' | 'highest_remaining';
export type RoutingOption = {
	id: string;
	account_id: string;
	main_model: string;
	worker_model?: string;
};
export type RoutingConfig = {
	enabled: boolean;
	strategy: RoutingStrategy;
	options: RoutingOption[];
};

export function parseRoutingConfig(value: unknown): RoutingConfig {
	if (!value || typeof value !== 'object') throw new Error('Invalid routing configuration.');
	const config = value as Record<string, unknown>;
	if (typeof config.enabled !== 'boolean' || !['ordered_first', 'highest_remaining'].includes(String(config.strategy)) || !Array.isArray(config.options)) {
		throw new Error('Invalid routing configuration.');
	}
	const options = config.options.map((value): RoutingOption => {
		if (!value || typeof value !== 'object') throw new Error('Invalid routing option.');
		const option = value as Record<string, unknown>;
		if (typeof option.id !== 'string' || typeof option.account_id !== 'string' || typeof option.main_model !== 'string' || (option.worker_model != null && typeof option.worker_model !== 'string')) {
			throw new Error('Invalid routing option.');
		}
		return {
			id: option.id,
			account_id: option.account_id,
			main_model: option.main_model,
			...(option.worker_model == null ? {} : { worker_model: option.worker_model })
		};
	});
	return {
		enabled: config.enabled,
		strategy: config.strategy as RoutingStrategy,
		options
	};
}

export function routingPayload(config: RoutingConfig): RoutingConfig {
	return {
		enabled: config.enabled,
		strategy: config.strategy,
		options: config.options.map((option) => ({
			id: option.id,
			account_id: option.account_id,
			main_model: option.main_model,
			...(option.worker_model == null ? {} : { worker_model: option.worker_model })
		}))
	};
}
</script>

<script lang="ts">
import { onMount } from 'svelte';
import { getJson, postJson } from '$lib/api';
import ModelSelect from '$lib/components/ModelSelect.svelte';
import { ui } from '$lib/ui.svelte';

type Account = { id: string; label: string; provider: string };
let { accounts }: { accounts: Account[] } = $props();

let saved = $state<RoutingConfig | null>(null);
let draft = $state<RoutingConfig | null>(null);
let loading = $state(false);
let saving = $state(false);
let loadError = $state('');
let saveError = $state('');
let savedMessage = $state('');
let dirty = $derived(saved !== null && draft !== null && JSON.stringify(routingPayload(saved)) !== JSON.stringify(routingPayload(draft)));
let validDraft = $derived(draft !== null && (!draft.enabled || draft.options.length > 0) && draft.options.every((option) => option.account_id.trim() !== '' && option.main_model.trim() !== '' && (option.worker_model === undefined || option.worker_model.trim() !== '')));

onMount(() => {
	void loadRouting();
});

async function loadRouting() {
	if (loading || saving || dirty) return;
	loading = true;
	loadError = '';
	saveError = '';
	savedMessage = '';
	try {
		const config = parseRoutingConfig(await getJson<unknown>('/api/accounts/routing'));
		saved = config;
		draft = parseRoutingConfig(config);
	} catch (cause) {
		loadError = cause instanceof Error ? cause.message : '자동 라우팅 설정을 불러오지 못했습니다.';
	} finally {
		loading = false;
	}
}

function updateOption(id: string, update: Partial<RoutingOption>) {
	if (!draft) return;
	draft = { ...draft, options: draft.options.map((option) => option.id === id ? { ...option, ...update } : option) };
	saveError = '';
	savedMessage = '';
}

function addOption() {
	if (!draft || accounts.length === 0) return;
	const id = newOptionId(draft.options);
	draft = {
		...draft,
		options: [...draft.options, { id, account_id: accounts[0].id, main_model: '' }]
	};
	saveError = '';
	savedMessage = '';
}

function newOptionId(options: RoutingOption[]): string {
	let id = typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function'
		? crypto.randomUUID()
		: `routing-${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
	let suffix = 1;
	while (options.some((option) => option.id === id)) id = `${id}-${suffix++}`;
	return id;
}

function removeOption(id: string) {
	if (!draft) return;
	draft = { ...draft, options: draft.options.filter((option) => option.id !== id) };
	saveError = '';
	savedMessage = '';
}

function moveOption(id: string, direction: -1 | 1) {
	if (!draft) return;
	const index = draft.options.findIndex((option) => option.id === id);
	const target = index + direction;
	if (index < 0 || target < 0 || target >= draft.options.length) return;
	const options = [...draft.options];
	[options[index], options[target]] = [options[target], options[index]];
	draft = { ...draft, options };
	saveError = '';
	savedMessage = '';
}

function toggleWorker(id: string, enabled: boolean) {
	const option = draft?.options.find((item) => item.id === id);
	if (!option) return;
	if (enabled) updateOption(id, { worker_model: option.main_model });
	else {
		const { worker_model: _workerModel, ...withoutWorker } = option;
		if (!draft) return;
		draft = { ...draft, options: draft.options.map((item) => item.id === id ? withoutWorker : item) };
		saveError = '';
		savedMessage = '';
	}
}

async function saveRouting() {
	if (!draft || !dirty || !validDraft || saving) return;
	saving = true;
	saveError = '';
	savedMessage = '';
	try {
		const canonical = parseRoutingConfig(await postJson<unknown>('/api/accounts/routing', routingPayload(draft)));
		saved = canonical;
		draft = parseRoutingConfig(canonical);
		savedMessage = '자동 라우팅 설정을 저장했습니다.';
		ui.notify('라우팅 설정을 저장했습니다.');
	} catch (cause) {
		saveError = cause instanceof Error ? cause.message : '자동 라우팅 설정을 저장하지 못했습니다.';
		ui.notify('라우팅 설정을 저장하지 못했습니다.', 'error');
	} finally {
		saving = false;
	}
}
</script>

<section id="routing-h" class="settings-card flex flex-col gap-4 rounded-2xl border border-neutral-200 bg-white p-4 dark:border-neutral-800 dark:bg-neutral-950 sm:p-5" aria-labelledby="routing-title">
	<div class="flex flex-wrap items-end justify-between gap-3">
		<div><h2 id="routing-title" class="text-[22px] font-semibold tracking-tight">모델 라우팅</h2><p class="mt-1 text-[13.5px] leading-relaxed text-neutral-600 dark:text-neutral-400">어떤 계정과 모델로 요청을 보낼지 정해요. 위에서부터 순서대로 시도해요.</p></div>
		{#if draft}
			<button type="button" disabled={loading || saving || dirty} class="min-h-10 rounded-xl border border-neutral-300 px-3 text-sm disabled:opacity-40 dark:border-neutral-700" onclick={loadRouting}>새로고침</button>
		{/if}
	</div>
	<p class="text-sm text-neutral-600 dark:text-neutral-400">계정 라우팅은 모델 선택이 Auto일 때 적용됩니다. 고정 모델 선택은 자동으로 덮어쓰지 않습니다. 변경 사항은 저장을 눌렀을 때만 호스트 설정에 반영됩니다.</p>
	{#if loading}
		<p role="status" class="text-sm text-neutral-500">라우팅 설정을 불러오는 중입니다.</p>
	{/if}
	{#if loadError}
		<p role="alert" class="text-sm text-red-700 dark:text-red-300">자동 라우팅 설정을 불러오지 못했습니다. {loadError}</p>
		<button type="button" disabled={loading || saving} class="min-h-10 self-start rounded-lg border border-neutral-300 px-3 text-sm disabled:opacity-40 dark:border-neutral-700" onclick={loadRouting}>다시 시도</button>
	{/if}
	{#if draft}
		<div class="flex flex-col gap-4 rounded-2xl border border-neutral-200 bg-white p-4 sm:p-5 dark:border-neutral-800 dark:bg-neutral-950">
			<label class="flex min-h-10 items-center gap-3 text-sm font-medium">
				<input type="checkbox" checked={draft.enabled} disabled={saving} onchange={(event) => { draft = { ...draft!, enabled: event.currentTarget.checked }; saveError = ''; savedMessage = ''; }} />
				자동 라우팅 사용
			</label>
			<label class="flex max-w-md flex-col gap-1.5 text-sm">
				<span>라우팅 전략</span>
				<select value={draft.strategy} disabled={saving} onchange={(event) => { draft = { ...draft!, strategy: event.currentTarget.value as RoutingStrategy }; saveError = ''; savedMessage = ''; }} class="min-h-11 rounded-xl border border-neutral-300 bg-white px-3 dark:border-neutral-700 dark:bg-neutral-900">
					<option value="ordered_first">순서대로 시도</option>
					<option value="highest_remaining">남은 사용량이 높은 계정 우선</option>
				</select>
			</label>
			<div class="flex flex-col gap-3" aria-label="라우팅 옵션">
				{#if draft.options.length === 0}
					<p class="text-sm text-neutral-500">라우팅 옵션이 없습니다. 옵션을 추가해 주세요.</p>
				{/if}
				{#each draft.options as option, index (option.id)}
					<fieldset class="grid min-w-0 gap-3 rounded-lg border border-neutral-200 p-3 dark:border-neutral-700 sm:grid-cols-2">
						<legend class="sr-only">라우팅 옵션 {index + 1}</legend>
						<label class="flex min-w-0 flex-col gap-1.5 text-sm">
							<span>계정</span>
							<select value={option.account_id} disabled={saving} onchange={(event) => updateOption(option.id, { account_id: event.currentTarget.value })} class="min-h-11 min-w-0 rounded-xl border border-neutral-300 bg-white px-3 dark:border-neutral-700 dark:bg-neutral-900">
								{#if !accounts.some((account) => account.id === option.account_id)}
									<option value={option.account_id} disabled>등록되지 않은 계정 ({option.account_id || '없음'})</option>
								{/if}
								{#each accounts as account (account.id)}
									<option value={account.id}>{account.label} ({account.provider})</option>
								{/each}
							</select>
						</label>
						<div class="min-w-0">
							<ModelSelect id={`routing-main-${option.id}`} bind:value={option.main_model} label="Main 모델" disabled={saving} />
						</div>
						<div class="flex min-w-0 flex-col gap-2 sm:col-span-2">
							<label class="flex min-h-10 items-center gap-2 text-sm">
								<input type="checkbox" checked={option.worker_model !== undefined} disabled={saving || !option.main_model} onchange={(event) => toggleWorker(option.id, event.currentTarget.checked)} />
								별도 Worker 모델 사용 (선택하지 않으면 Main 모델 사용)
							</label>
							{#if option.worker_model !== undefined}
								<ModelSelect id={`routing-worker-${option.id}`} bind:value={option.worker_model} label="Worker 모델" disabled={saving} />
							{/if}
						</div>
						<div class="flex flex-wrap items-center gap-2 sm:col-span-2">
							<button type="button" disabled={saving || index === 0} aria-label={`옵션 ${index + 1} 위로 이동`} class="min-h-10 min-w-10 rounded-lg border border-neutral-300 px-3 text-sm disabled:opacity-40 dark:border-neutral-700" onclick={() => moveOption(option.id, -1)}>위로</button>
							<button type="button" disabled={saving || index === draft.options.length - 1} aria-label={`옵션 ${index + 1} 아래로 이동`} class="min-h-10 min-w-10 rounded-lg border border-neutral-300 px-3 text-sm disabled:opacity-40 dark:border-neutral-700" onclick={() => moveOption(option.id, 1)}>아래로</button>
							<button type="button" disabled={saving} class="min-h-10 rounded-lg border border-red-300 px-3 text-sm text-red-700 dark:border-red-900 dark:text-red-300" onclick={() => removeOption(option.id)}>옵션 제거</button>
						</div>
					</fieldset>
				{/each}
			</div>
			<div class="flex flex-wrap items-center gap-2">
				<button type="button" disabled={accounts.length === 0 || saving} class="min-h-10 rounded-lg border border-neutral-300 px-3 text-sm disabled:opacity-40 dark:border-neutral-700" onclick={addOption}>옵션 추가</button>
				<button type="button" disabled={!dirty || !validDraft || saving} class="min-h-10 rounded-lg bg-neutral-900 px-4 text-sm font-medium text-white disabled:opacity-40 dark:bg-neutral-100 dark:text-neutral-900" onclick={saveRouting}>{saving ? '저장 중…' : '저장'}</button>
				{#if dirty}<span class="text-sm text-amber-700 dark:text-amber-300">저장하지 않은 변경 사항</span>{/if}
			</div>
			{#if draft.options.some((option) => !option.account_id.trim() || !option.main_model.trim() || (option.worker_model !== undefined && !option.worker_model.trim()))}
				<p class="text-sm text-amber-700 dark:text-amber-300">각 옵션에 계정과 Main 모델을 선택해 주세요.</p>
			{/if}
			{#if saveError}<p role="alert" class="break-words text-sm text-red-700 dark:text-red-300">저장하지 못했습니다. 초안은 유지됩니다. {saveError}</p>{/if}
			{#if savedMessage}<p role="status" class="text-sm text-green-700 dark:text-green-300">{savedMessage}</p>{/if}
		</div>
	{/if}
</section>
