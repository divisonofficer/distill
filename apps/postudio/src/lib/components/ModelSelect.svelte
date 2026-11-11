<script lang="ts">
import { onMount } from 'svelte';
import { getJson } from '$lib/api';
import type { ModelCatalog } from '$lib/types';

let {
	id,
	label = '모델',
	value = $bindable(''),
	initialFromCatalog = false,
	disabled = false,
	commitOnSelect = true,
	onselect
}: {
	id: string;
	label?: string;
	value?: string;
	initialFromCatalog?: boolean;
	disabled?: boolean;
	commitOnSelect?: boolean;
	onselect?: (modelId: string) => void;
} = $props();

let catalog = $state<ModelCatalog | null>(null);
let loading = $state(true);
let error = $state('');
let hasCatalogModel = $derived(catalog?.availableModels.some((model) => model.modelId === value) ?? false);
let selectDisabled = $derived(
	disabled || loading || error !== '' || (catalog !== null && catalog.availableModels.length === 0)
);

onMount(() => {
	let active = true;
	void getJson<ModelCatalog>('/api/models')
		.then((response) => {
			if (!active) return;
			catalog = response;
			if (initialFromCatalog && !value && response.currentModelId) value = response.currentModelId;
		})
		.catch((cause: unknown) => {
			if (!active) return;
			error = cause instanceof Error ? cause.message : '모델 목록을 불러오지 못했습니다.';
		})
		.finally(() => {
			if (active) loading = false;
		});
	return () => {
		active = false;
	};
});

function choose(event: Event) {
	const select = event.currentTarget as HTMLSelectElement;
	const modelId = select.value;
	if (commitOnSelect) value = modelId;
	else select.value = value;
	onselect?.(modelId);
}
</script>

<div class="flex min-w-0 flex-col gap-1.5">
	<label for={id} class="text-sm font-medium">{label}</label>
	<select
		{id}
		value={value}
		onchange={choose}
		disabled={selectDisabled}
		class="min-h-11 w-full rounded-xl border border-neutral-300 bg-white px-3 text-sm outline-none focus:border-neutral-500 disabled:cursor-not-allowed disabled:opacity-60 dark:border-neutral-700 dark:bg-neutral-900 dark:focus:border-neutral-500"
	>
		<option value="" disabled>모델 선택</option>
		{#if value && !hasCatalogModel}
			<option value={value} disabled>{value}{catalog ? ' (목록에 없음)' : ''}</option>
		{/if}
		{#each catalog?.availableModels ?? [] as model (model.modelId)}
			<option value={model.modelId}>{model.name}</option>
		{/each}
	</select>
	{#if loading}
		<p role="status" class="text-xs text-neutral-500">모델 목록 불러오는 중</p>
	{:else if error}
		<p role="alert" class="text-xs text-red-700 dark:text-red-300">모델 목록을 불러오지 못했습니다. {error}</p>
	{:else if catalog?.availableModels.length === 0}
		<p role="status" class="text-xs text-neutral-500">사용 가능한 모델이 없습니다.</p>
	{/if}
</div>
