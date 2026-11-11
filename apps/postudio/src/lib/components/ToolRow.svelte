<script lang="ts">
import Badge from './Badge.svelte';
import Icon from './Icon.svelte';
import type { ToolItem } from '$lib/session.svelte';
import type { Tone } from '$lib/format';

let { item }: { item: ToolItem } = $props();

const statusLabel: Record<string, { label: string; tone: Tone }> = {
pending: { label: '대기', tone: 'idle' },
in_progress: { label: '실행 중', tone: 'run' },
completed: { label: '완료', tone: 'idle' },
failed: { label: '실패', tone: 'err' }
};

const status = $derived(item.status ? (statusLabel[item.status] ?? { label: item.status, tone: 'idle' as Tone }) : null);
const images = $derived(
(item.content ?? []).flatMap((c) =>
c.type === 'image' && /^image\/(png|jpeg|gif|webp)$/.test(c.mime) ? [`data:${c.mime};base64,${c.data}`] : []
)
);
</script>

<details class="group rounded-xl text-sm text-neutral-600 dark:text-neutral-400">
<summary class="flex min-h-9 cursor-pointer list-none items-center gap-2 rounded-lg px-2 select-none hover:bg-neutral-200/60 dark:hover:bg-neutral-800/60">
<Icon name="chevron" class="size-3.5 shrink-0 transition-transform group-open:rotate-90" />
<span class="min-w-0 flex-1 truncate font-mono text-[13px]">{item.title ?? item.name ?? item.id}</span>
{#if status}<Badge label={status.label} tone={status.tone} />{/if}
</summary>
<div class="mt-1 ml-5 flex flex-col gap-2 border-l-2 border-neutral-200 pl-3 dark:border-neutral-800">
{#if item.input !== undefined}
<div>
<p class="mb-1 text-xs text-neutral-500">입력</p>
<pre class="overflow-x-auto rounded-lg bg-neutral-100 p-2 text-xs dark:bg-neutral-900">{JSON.stringify(item.input, null, 2)}</pre>
</div>
{/if}
{#each item.content ?? [] as c, i (i)}
{#if c.type === 'text'}
<pre class="overflow-x-auto rounded-lg bg-neutral-100 p-2 text-xs whitespace-pre-wrap dark:bg-neutral-900">{c.text}</pre>
{:else if c.type === 'diff'}
<div class="text-xs">
<p class="mb-1 font-mono text-neutral-500">{c.path}</p>
<div class="overflow-x-auto rounded-lg bg-neutral-100 font-mono dark:bg-neutral-900">
{#if c.old !== undefined}
<pre class="bg-red-100 p-2 whitespace-pre-wrap text-red-900 dark:bg-red-950 dark:text-red-100">- {c.old}</pre>
{/if}
<pre class="bg-green-100 p-2 whitespace-pre-wrap text-green-900 dark:bg-green-950 dark:text-green-100">+ {c.new}</pre>
</div>
</div>
{/if}
{/each}
{#each images as src, i (i)}
<img {src} alt="도구 결과 이미지 {i + 1}" class="h-auto w-auto max-w-full self-start rounded-lg" />
{/each}
</div>
</details>
