<script lang="ts">
import { tick } from 'svelte';
import { isPreviewImageMime, textSize } from '$lib/attachments';
import type { UserAttachment } from '$lib/types';

export interface PreviewItem {
	key: number | string;
	attachment: UserAttachment;
	size?: number;
}

let {
	items,
	removable = false,
	onremove
}: {
	items: PreviewItem[];
	removable?: boolean;
	onremove?: (key: PreviewItem['key']) => void;
} = $props();

let selected = $state<UserAttachment | null>(null);
let dialog = $state<HTMLDialogElement>();
let returnFocus: HTMLElement | null = null;

$effect(() => {
	if (selected && dialog && !dialog.open) dialog.showModal();
});

export function openPreview(attachment: UserAttachment, trigger: HTMLButtonElement) {
	returnFocus = trigger;
	selected = attachment;
}

function closePreview() {
	if (dialog?.open) dialog.close();
	else restoreFocus();
}

async function restoreFocus() {
const trigger = returnFocus;
returnFocus = null;
selected = null;
await tick();
trigger?.focus();
}

function imageUrl(attachment: Extract<UserAttachment, { kind: 'image' }>): string {
	return `data:${attachment.mime};base64,${attachment.data}`;
}

function sizeLabel(size: number): string {
	if (size < 1024) return `${size} B`;
	if (size < 1024 * 1024) return `${(size / 1024).toFixed(1)} KB`;
	return `${(size / (1024 * 1024)).toFixed(1)} MB`;
}

function filename(attachment: UserAttachment): string {
	return attachment.kind === 'image' ? attachment.name || '이미지' : attachment.name;
}

function handleDialogClick(event: MouseEvent) {
	if (event.target === dialog) closePreview();
}
</script>

{#if items.length > 0}
<ul aria-label="첨부 파일" class="flex gap-2 overflow-x-auto px-1 pb-2 pt-1">
	{#each items as item (item.key)}
		{@const attachment = item.attachment}
		{@const name = filename(attachment)}
		<li class="relative min-w-0 max-w-full shrink-0">
			{#if attachment.kind === 'image' && isPreviewImageMime(attachment.mime)}
				<button
					type="button"
					class="block rounded-xl focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-neutral-500"
					aria-label="이미지 미리보기: {name}"
					onclick={(event) => openPreview(attachment, event.currentTarget)}
				>
					<img src={imageUrl(attachment)} alt={name} class="size-16 rounded-xl object-cover" />
				</button>
			{:else if attachment.kind === 'text'}
				<button
					type="button"
					class="flex h-14 w-[232px] max-w-[calc(100vw-5rem)] items-center gap-2 overflow-hidden rounded-xl border border-neutral-200 bg-white py-2 pl-2 pr-8 text-left text-xs hover:bg-neutral-50 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-neutral-500 dark:border-neutral-700 dark:bg-neutral-950 dark:hover:bg-neutral-900"
					aria-label="텍스트 파일 미리보기: {attachment.name}"
					onclick={(event) => openPreview(attachment, event.currentTarget)}
				>
					<span aria-hidden="true" class="flex size-[38px] shrink-0 items-center justify-center rounded-lg bg-[#3a3a3e] text-[10px] font-bold text-white">{attachment.name.toLowerCase().endsWith('.md') ? 'MD' : 'TXT'}</span>
					<span class="min-w-0"><span class="block truncate text-[13px] font-semibold">{attachment.name}</span>
					<span class="mt-0.5 block text-[11.5px] text-neutral-500">{attachment.name.toLowerCase().endsWith('.md') ? 'MD' : 'TXT'} · {sizeLabel(item.size ?? textSize(attachment.text))}</span></span>
				</button>
			{:else}
				<div class="min-h-16 w-52 max-w-[calc(100vw-5rem)] rounded-xl border border-neutral-300 bg-neutral-50 p-2 text-xs dark:border-neutral-700 dark:bg-neutral-950">
					<span class="block truncate font-medium">{name}</span>
					<span class="mt-2 block text-neutral-500">미리보기 콘텐츠를 사용할 수 없습니다.</span>
				</div>
			{/if}
			{#if removable && onremove}
				<button
					type="button"
					class="absolute right-1 top-1 flex size-7 items-center justify-center rounded-full bg-white text-neutral-600 hover:bg-neutral-100 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-neutral-500 dark:bg-neutral-900 dark:text-neutral-300"
					aria-label="첨부 {name} 제거"
					onclick={() => onremove?.(item.key)}
				>
					<span aria-hidden="true">×</span>
				</button>
			{/if}
		</li>
	{/each}
</ul>
{/if}

<dialog
	bind:this={dialog}
	aria-labelledby="attachment-preview-title"
	class="m-auto max-h-[90vh] max-w-[min(90vw,64rem)] rounded-2xl bg-white p-0 text-neutral-900 shadow-2xl backdrop:bg-black/70 dark:bg-neutral-900 dark:text-neutral-100"
	onclose={restoreFocus}
	onclick={handleDialogClick}
>
	{#if selected}
		<div class="flex max-h-[90vh] flex-col">
			<header class="flex items-center gap-3 border-b border-neutral-200 p-4 dark:border-neutral-700">
				<h2 id="attachment-preview-title" class="min-w-0 flex-1 truncate text-sm font-medium">{filename(selected)}</h2>
				<button
					type="button"
					class="flex size-10 shrink-0 items-center justify-center rounded-xl hover:bg-neutral-100 focus-visible:outline-2 focus-visible:outline-neutral-500 dark:hover:bg-neutral-800"
					aria-label="미리보기 닫기"
					onclick={closePreview}
				>
					<span aria-hidden="true">×</span>
				</button>
			</header>
			{#if selected.kind === 'image' && isPreviewImageMime(selected.mime)}
				<img src={imageUrl(selected)} alt={filename(selected)} class="max-h-[calc(90vh-5rem)] max-w-[90vw] object-contain p-4" />
			{:else if selected.kind === 'text'}
				<pre class="max-h-[calc(90vh-5rem)] max-w-[90vw] overflow-auto p-4 text-sm leading-6 whitespace-pre-wrap break-words">{selected.text}</pre>
			{/if}
		</div>
	{/if}
</dialog>
