<script lang="ts">
import Icon from './Icon.svelte';
import type { ToolItem } from '$lib/session.svelte';
import type { ToolContent } from '$lib/types';
import type { Tone } from '$lib/format';

let { item, id, onselect }: { item: ToolItem; id?: string; onselect?: (item: ToolItem) => void } = $props();

type ToolIcon = 'document' | 'terminal' | 'edit' | 'server';
type OperationKind = 'read' | 'command' | 'edit' | 'other';

const statusLabels: Record<string, { label: string; tone: Tone }> = {
pending: { label: '대기', tone: 'idle' },
in_progress: { label: '실행 중', tone: 'run' },
completed: { label: '완료', tone: 'idle' },
failed: { label: '실패', tone: 'err' }
};

const operation = $derived.by(() => {
const input = item.input && typeof item.input === 'object' && !Array.isArray(item.input)
? item.input as Record<string, unknown>
: {};
const kind = (item.toolKind ?? '').toLowerCase();
const name = (item.name ?? '').toLowerCase();
const command = typeof item.input === 'string' ? item.input : firstString(input, ['command', 'cmd', 'script']);
const path = firstString(input, ['path', 'file_path', 'filePath', 'filepath', 'filename', 'target']);
if (kind === 'read' || /(^|[^a-z])read(?:_|$)|readfile|file_reader/.test(name)) {
return { kind: 'read' as const, label: '파일 읽기', icon: 'document' as const, detail: path ?? item.title ?? item.name };
}
if (kind === 'execute' || /(^|[_:/-])(bash|shell|execute|exec|terminal)([_:/-]|$)|(^|_)run_command([_:/-]|$)/.test(name)) {
return { kind: 'command' as const, label: '명령 실행', icon: 'terminal' as const, detail: command ?? item.title ?? item.name };
}
if (kind === 'edit' || kind === 'write' || /(^|[_:/-])(edit|write|patch|replace)([_:/-]|$)|search_replace|apply_patch/.test(name)) {
return { kind: 'edit' as const, label: '파일 수정', icon: 'edit' as const, detail: path ?? item.title ?? item.name };
}
if (kind === 'delete') return { kind: 'other' as const, label: '파일 삭제', icon: 'edit' as const, detail: path ?? item.title ?? item.name };
if (kind === 'move') return { kind: 'other' as const, label: '파일 이동', icon: 'edit' as const, detail: path ?? item.title ?? item.name };
return { kind: 'other' as const, label: item.title ?? item.name ?? '도구 실행', icon: 'server' as const, detail: undefined };
});

const texts = $derived((item.content ?? []).filter((content) => content.type === 'text'));
const images = $derived(
(item.content ?? []).flatMap((content) =>
content.type === 'image' && /^image\/(png|jpeg|gif|webp)$/.test(content.mime) ? [`data:${content.mime};base64,${content.data}`] : []
)
);
const commandExitCode = $derived.by(() => {
if (operation.kind !== 'command') return null;
const output = texts.map((content) => content.text).join('\n');
const match = output.match(/(?:^|\n)\s*(?:exit:\s*(-?\d+)|exit code:\s*(-?\d+))(?:\s|$)/i);
if (!match) return null;
return Number(match[1] ?? match[2]);
});
const commandFailed = $derived(item.status === 'completed' && commandExitCode !== null && commandExitCode !== 0 && commandExitCode !== -1);
const status = $derived.by(() => {
if (commandFailed) return { label: `명령 실패 · 종료 코드 ${commandExitCode}`, tone: 'err' as Tone };
return item.status ? (statusLabels[item.status] ?? { label: item.status, tone: 'idle' as Tone }) : null;
});
const changeSummary = $derived.by(() => {
const diffs = (item.content ?? []).filter((content): content is Extract<ToolContent, { type: 'diff' }> => content.type === 'diff');
if (diffs.length === 0 || diffs.some((content) => content.old === undefined || content.new === undefined)) return null;
return countDiffChanges(diffs.map((content) => ({ old: content.old!, new: content.new })));
});

function firstString(input: Record<string, unknown>, keys: string[]): string | undefined {
for (const key of keys) {
if (typeof input[key] === 'string' && input[key].length > 0) return input[key] as string;
}
return undefined;
}

export function countDiffChanges(diffs: { old: string; new: string }[]) {
let added = 0;
let removed = 0;
// ponytail: Cap comparisons at one million; use Myers diff for larger changes.
let budget = 1_000_000;
for (const diff of diffs) {
const oldLines = diff.old === '' ? [] : diff.old.replace(/\r\n/g, '\n').split('\n');
const newLines = diff.new === '' ? [] : diff.new.replace(/\r\n/g, '\n').split('\n');
if (oldLines.at(-1) === '' && diff.old.endsWith('\n')) oldLines.pop();
if (newLines.at(-1) === '' && diff.new.endsWith('\n')) newLines.pop();
const rows = oldLines.length + 1;
const columns = newLines.length + 1;
const comparisons = (rows - 1) * (columns - 1);
if (comparisons > budget) return null;
budget -= comparisons;
let previous = new Uint32Array(columns);
for (let row = 1; row < rows; row++) {
const current = new Uint32Array(columns);
for (let column = 1; column < columns; column++) {
current[column] = oldLines[row - 1] === newLines[column - 1]
? previous[column - 1] + 1
: Math.max(previous[column], current[column - 1]);
}
previous = current;
}
const lcs = previous[columns - 1];
removed += oldLines.length - lcs;
added += newLines.length - lcs;
}
return { added, removed };
}
</script>

<details {id} tabindex="-1" open={item.status === 'failed' || item.status === 'in_progress' || commandFailed} class="group overflow-hidden rounded-xl border border-neutral-200 bg-white text-sm text-neutral-700 dark:border-neutral-800 dark:bg-neutral-900 dark:text-neutral-300">
<summary onclick={() => onselect?.(item)} class="flex min-h-10 cursor-pointer list-none items-center gap-2 px-3 select-none hover:bg-neutral-100 focus-visible:outline-2 focus-visible:outline-sky-700 dark:hover:bg-neutral-800/80">
{#if item.status === 'failed' || commandFailed}<span aria-label="실패" class="size-2 shrink-0 rounded-full bg-red-500"></span>{:else if item.status === 'in_progress'}<span aria-label="실행 중" class="size-2 shrink-0 animate-pulse rounded-full bg-sky-500"></span>{:else if item.status === 'completed'}<span aria-label="완료" class="shrink-0 text-emerald-600 dark:text-emerald-400">✓</span>{:else if item.status === 'pending'}<span aria-label="대기" class="size-2 shrink-0 rounded-full bg-neutral-400"></span>{/if}
<Icon name={operation.icon as ToolIcon} class="size-4 shrink-0 text-neutral-500" />
<span class="max-w-[40%] shrink-0 truncate font-medium" title={operation.label}>{operation.label}</span>
{#if operation.detail}<code class="min-w-0 flex-1 truncate text-left text-xs text-neutral-600 dark:text-neutral-400" title={operation.detail}>{operation.detail}</code>{:else}<span class="min-w-0 flex-1"></span>{/if}
{#if changeSummary}<span class="shrink-0 text-xs text-neutral-500"><span class="text-emerald-700 dark:text-emerald-400">+{changeSummary.added}</span> / <span class="text-red-700 dark:text-red-400">−{changeSummary.removed}</span></span>{/if}
{#if commandExitCode !== null}<span class="shrink-0 text-xs text-neutral-500">exit {commandExitCode}</span>{:else if status}<span class="shrink-0 text-xs text-neutral-500">{status.label}</span>{/if}
</summary>
<div class="flex flex-col gap-2 border-t border-neutral-200 bg-neutral-950 p-2 dark:border-neutral-800">
{#if item.status === 'failed' && texts.length === 0}<p role="alert" class="font-mono text-xs text-red-300">도구 실행 실패</p>{/if}
{#each item.content ?? [] as content, index (index)}
{#if content.type === 'text'}
<pre class="overflow-x-auto font-mono text-xs leading-5 whitespace-pre-wrap text-neutral-100">{content.text}</pre>
{:else if content.type === 'diff'}
<div class="text-xs">
<p class="mb-1 break-all font-mono text-neutral-400">{content.path}</p>
<div class="overflow-x-auto rounded bg-neutral-900 font-mono">
{#if content.old !== undefined}<pre class="bg-red-950/70 p-2 whitespace-pre-wrap text-red-200">- {content.old}</pre>{/if}
<pre class="bg-emerald-950/70 p-2 whitespace-pre-wrap text-emerald-200">+ {content.new}</pre>
</div>
</div>
{/if}
{/each}
{#each images as src, index (index)}
<img {src} alt="도구 결과 이미지 {index + 1}" class="h-auto w-auto max-w-full self-start rounded-lg" />
{/each}
{#if item.input !== undefined}<details class="text-xs text-neutral-400"><summary class="min-h-8 cursor-pointer content-center">입력 데이터</summary><pre class="overflow-x-auto whitespace-pre-wrap font-mono text-neutral-100">{JSON.stringify(item.input, null, 2)}</pre></details>{/if}
<details class="text-xs text-neutral-400"><summary class="min-h-8 cursor-pointer content-center">호출 정보</summary><code class="font-mono text-neutral-100">{item.id}</code></details>
</div>
</details>
