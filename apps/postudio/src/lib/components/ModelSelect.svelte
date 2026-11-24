<script lang="ts">
import { onDestroy, tick, untrack } from 'svelte';
import { getJson } from '$lib/api';
import Icon from '$lib/components/Icon.svelte';
import { authenticatedModels, defaultModelGroups, groupModels, isAutoModel, modelProvider, moveMenuIndex, moveModelIndex } from '$lib/models';
import type { Model, ModelCatalog } from '$lib/types';

let {
id,
label = '모델',
compact = false,
value = $bindable(''),
initialFromCatalog = false,
disabled = false,
commitOnSelect = true,
onselect,
host,
modelOptions
}: {
id: string;
host?: string;
modelOptions?: Model[];
label?: string;
compact?: boolean;
value?: string;
initialFromCatalog?: boolean;
disabled?: boolean;
commitOnSelect?: boolean;
onselect?: (modelId: string) => void;
} = $props();

let catalog = $state<ModelCatalog | null>(null);
let accounts = $state<unknown>(null);
let loading = $state(true);
let error = $state('');
let open = $state(false);
let query = $state('');
let activeRootIndex = $state(-1);
let activeModelIndex = $state(-1);
let submenuProvider = $state<string | null>(null);
let opensUp = $state(false);
let popupHeight = $state(120);
let popupWidth = $state(288);
let popupLeft = $state(0);
let popupTop = $state(0);
let popup = $state<HTMLDivElement>();
let root = $state<HTMLDivElement>();
let repositionCleanup: (() => void) | undefined;
let input = $state<HTMLInputElement>();
let selectionFocusPending = $state(false);
let catalogModels = $derived(modelOptions ?? authenticatedModels(catalog?.availableModels ?? [], accounts));
let menuGroups = $derived(defaultModelGroups(catalogModels));
let autoModel = $derived(catalogModels.find(isAutoModel));
let rootEntries = $derived([
...(autoModel ? [{ kind: 'auto' as const, model: autoModel }] : []),
...menuGroups.map((group) => ({ kind: 'provider' as const, provider: group.provider }))
]);
let searchGroups = $derived(groupModels(catalogModels, query));
let searchModels = $derived(searchGroups.flatMap((group) => group.models));
let submenuModels = $derived(menuGroups.find((group) => group.provider === submenuProvider)?.models ?? []);
let selectedModel = $derived(catalogModels.find((model) => model.modelId === value));
let selectedProvider = $derived(selectedModel ? modelProvider(selectedModel.modelId) : null);
let selectDisabled = $derived(disabled || loading || error !== '' || (modelOptions !== undefined && !modelOptions.length) || (catalog !== null && catalog.availableModels.length === 0));
let activeDescendant = $derived.by(() => {
if (!open) return undefined;
if (query.trim()) return activeModelIndex >= 0 && activeModelIndex < searchModels.length ? `${id}-option-${activeModelIndex}` : undefined;
if (submenuProvider) return activeModelIndex >= 0 && activeModelIndex < submenuModels.length ? `${id}-option-${activeModelIndex}` : undefined;
return activeRootIndex >= 0 && activeRootIndex < rootEntries.length ? `${id}-root-option-${activeRootIndex}` : undefined;
});
const listId = $derived(`${id}-listbox`);

function providerInitial(provider: string): string {
return Array.from(provider)[0]?.toLocaleUpperCase() ?? '•';
}

function providerBadgeClass(provider: string): string {
const colors: Record<string, string> = {
Claude: 'bg-orange-50 text-orange-700 dark:bg-orange-950 dark:text-orange-300',
ChatGPT: 'bg-emerald-50 text-emerald-700 dark:bg-emerald-950 dark:text-emerald-300',
'xAI': 'bg-neutral-900 text-white dark:bg-neutral-100 dark:text-neutral-900',
Google: 'bg-blue-50 text-blue-700 dark:bg-blue-950 dark:text-blue-300'
};
return colors[provider] ?? 'bg-neutral-100 text-neutral-600 dark:bg-neutral-800 dark:text-neutral-300';
}

$effect(() => {
const destination = host;
if (modelOptions !== undefined) { loading = false; return; }
selectionFocusPending = false;
untrack(closeMenu);
let active = true;
catalog = null;
accounts = null;
loading = true;
error = '';
void Promise.all([getJson<ModelCatalog>('/api/models', destination), getJson<unknown>('/api/accounts', destination)])
.then(([response, accountResponse]) => {
if (!active) return;
catalog = response;
accounts = accountResponse;
if (initialFromCatalog && !value && response.currentModelId && authenticatedModels(response.availableModels, accountResponse).some((model) => model.modelId === response.currentModelId)) value = response.currentModelId;
})
.catch((cause: unknown) => {
if (!active) return;
error = cause instanceof Error ? cause.message : '모델 목록을 불러오지 못했습니다.';
})
.finally(() => { if (active) loading = false; });
return () => { active = false; };
});

function stopTrackingPopup() {
if (!repositionCleanup) return;
repositionCleanup();
repositionCleanup = undefined;
}

function trackPopupPosition() {
stopTrackingPopup();
const reposition = () => positionPopup();
const dismissOutside = (event: PointerEvent) => {
if (open && !root?.contains(event.target as Node | null)) closeMenu();
};
window.addEventListener('resize', reposition);
window.addEventListener('scroll', reposition, true);
window.addEventListener('pointerdown', dismissOutside, true);
repositionCleanup = () => {
window.removeEventListener('resize', reposition);
window.removeEventListener('scroll', reposition, true);
window.removeEventListener('pointerdown', dismissOutside, true);
};
}

onDestroy(stopTrackingPopup);

function positionPopup() {
const rect = root?.getBoundingClientRect();
if (!rect) return;
const viewportWidth = window.innerWidth;
const availableWidth = Math.max(0, viewportWidth - 24);
popupWidth = Math.min(320, availableWidth);
popupLeft = Math.max(12, Math.min(rect.left, viewportWidth - popupWidth - 12));
const rootHeight = rootEntries.length * 56 + 16;
const searchHeight = searchModels.length * 40 + searchGroups.length * 28 + 16;
const submenuHeight = submenuModels.length * 44 + 64;
const compactSearchHeight = compact ? 48 : 0;
const wantedHeight = Math.min(380, Math.max(96, query.trim() ? searchHeight : submenuProvider ? submenuHeight : rootHeight) + compactSearchHeight);
const above = Math.max(0, rect.top - 12);
const below = Math.max(0, window.innerHeight - rect.bottom - 12);
opensUp = compact
? above >= Math.min(wantedHeight, 180) || above > below
: below < wantedHeight + 8 && above > below;
const availableHeight = opensUp ? above : below;
popupHeight = Math.min(wantedHeight, Math.max(0, availableHeight - 8));
popupTop = opensUp ? rect.top - popupHeight - 8 : rect.bottom + 8;
if (popup?.matches(':popover-open')) {
popup.style.left = `${popupLeft}px`;
popup.style.top = `${popupTop}px`;
popup.style.width = `${popupWidth}px`;
popup.style.height = `${popupHeight}px`;
}
}

async function openMenu() {
if (selectDisabled) return;
query = '';
submenuProvider = null;
open = true;
await tick();
const selectedProvider = modelProvider(value);
activeRootIndex = rootEntries.findIndex((entry) => entry.kind === 'auto'
? entry.model.modelId === value
: entry.provider === selectedProvider);
if (activeRootIndex < 0) activeRootIndex = 0;
activeModelIndex = -1;
positionPopup();
popup?.showPopover();
if (compact) {
await tick();
input?.focus();
}
trackPopupPosition();
}

async function openProvider(provider: string, rootIndex: number) {
submenuProvider = provider;
activeRootIndex = rootIndex;
const models = menuGroups.find((group) => group.provider === provider)?.models ?? [];
activeModelIndex = Math.max(0, models.findIndex((model) => model.modelId === value));
await tick();
positionPopup();
if (compact) input?.focus();
}

async function returnToRoot() {
const provider = submenuProvider;
submenuProvider = null;
query = '';
activeRootIndex = rootEntries.findIndex((entry) => entry.kind === 'provider' && entry.provider === provider);
activeModelIndex = -1;
await tick();
positionPopup();
if (compact) input?.focus();
}

function closeMenu() {
open = false;
stopTrackingPopup();
popup?.hidePopover();
}

function choose(modelId: string) {
if (commitOnSelect) value = modelId;
closeMenu();
query = '';
submenuProvider = null;
if (compact) selectionFocusPending = true;
onselect?.(modelId);
if (!compact) input?.focus();
}

$effect(() => {
if (!selectionFocusPending || selectDisabled) return;
selectionFocusPending = false;
void tick().then(() => {
if (document.activeElement === document.body || root?.contains(document.activeElement)) {
(root?.querySelector('button[aria-haspopup]') as HTMLButtonElement | null)?.focus();
}
});
});

async function onInput(event: Event) {
const target = event.currentTarget as HTMLInputElement;
const text = target.value;
if (!open) await openMenu();
query = text;
submenuProvider = null;
activeRootIndex = -1;
activeModelIndex = 0;
await tick();
positionPopup();
}

function onTriggerKeydown(event: KeyboardEvent) {
if (event.isComposing || event.keyCode === 229) return;
if (['Enter', ' ', 'ArrowDown', 'ArrowUp'].includes(event.key)) {
event.preventDefault();
if (open) closeMenu();
else void openMenu();
} else if (event.key === 'Escape' && open) {
event.preventDefault();
closeMenu();
}
}

async function onKeydown(event: KeyboardEvent) {
if (event.isComposing || event.keyCode === 229) return;
const key = event.key;
if (key === 'Escape' && open) {
event.preventDefault();
closeMenu();
query = '';
submenuProvider = null;
activeRootIndex = -1;
activeModelIndex = -1;
if (compact) (root?.querySelector('button[aria-haspopup]') as HTMLButtonElement | null)?.focus();
return;
}
if (key === 'ArrowLeft' && open && submenuProvider && !query.trim()) {
event.preventDefault();
await returnToRoot();
return;
}
if (['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(key)) {
event.preventDefault();
if (!open) {
await openMenu();
return;
}
if (query.trim()) {
activeModelIndex = moveModelIndex(searchModels, searchModels[activeModelIndex]?.modelId ?? '', key) ?? -1;
} else if (submenuProvider) {
activeModelIndex = moveModelIndex(submenuModels, submenuModels[activeModelIndex]?.modelId ?? '', key) ?? -1;
} else {
activeRootIndex = moveMenuIndex(rootEntries.length, activeRootIndex, key) ?? -1;
}
return;
}
if (key === 'ArrowRight') {
if (!open) {
event.preventDefault();
await openMenu();
return;
}
if (!query.trim()) {
const entry = rootEntries[activeRootIndex];
if (entry?.kind === 'provider') {
event.preventDefault();
await openProvider(entry.provider, activeRootIndex);
}
}
return;
}
if (key === 'Enter') {
event.preventDefault();
if (!open) {
await openMenu();
return;
}
if (query.trim()) {
const model = searchModels[activeModelIndex];
if (model) choose(model.modelId);
return;
}
if (submenuProvider) {
const model = submenuModels[activeModelIndex];
if (model) choose(model.modelId);
return;
}
const entry = rootEntries[activeRootIndex];
if (entry?.kind === 'auto') choose(entry.model.modelId);
else if (entry?.kind === 'provider') await openProvider(entry.provider, activeRootIndex);
}
}

function onPopupKeydown(event: KeyboardEvent) {
if (event.target === input) return;
if (event.key === 'Escape') {
onKeydown(event);
} else if (event.key === 'ArrowLeft' && submenuProvider && !query.trim()) {
event.preventDefault();
void returnToRoot();
}
}

$effect(() => {
const optionId = activeDescendant;
if (optionId) void tick().then(() => {
const option = document.getElementById(optionId);
if (!option) return;
let pane = option.parentElement;
while (pane && pane !== root && !pane.classList.contains('overflow-y-auto')) pane = pane.parentElement;
if (!pane || pane === root) return;
const bounds = pane.getBoundingClientRect();
const item = option.getBoundingClientRect();
const heading = option.parentElement?.querySelector('p.sticky');
const inset = heading?.getBoundingClientRect().height ?? 0;
if (item.top < bounds.top + inset) pane.scrollTop -= bounds.top + inset - item.top;
else if (item.bottom > bounds.bottom) pane.scrollTop += item.bottom - bounds.bottom;
});
});

function onFocusOut(event: FocusEvent) {
if (!root?.contains(event.relatedTarget as Node | null)) closeMenu();
}
</script>

<div bind:this={root} class="relative flex min-w-0 flex-col gap-1.5" style:width={compact ? 'max-content' : undefined} style:max-width={compact ? '100%' : undefined} onfocusout={onFocusOut}>
<label for={id} class={compact ? 'sr-only' : 'text-sm font-medium'}>{label}</label>
{#if compact}
<button
{id}
type="button"
aria-label={`${label} · ${selectedModel?.name ?? (value && !loading ? '사용할 수 없는 모델' : '모델 선택')}`}
aria-haspopup="dialog"
aria-expanded={open}
disabled={selectDisabled}
onclick={() => { if (open) closeMenu(); else void openMenu(); }}
onkeydown={onTriggerKeydown}
class="flex h-[34px] max-w-[180px] min-w-0 items-center gap-2 rounded-lg px-2 text-sm outline-none hover:bg-neutral-100 focus-visible:ring-2 focus-visible:ring-sky-700 disabled:cursor-not-allowed disabled:opacity-60 dark:hover:bg-neutral-800"
><span class="min-w-0 truncate">{selectedModel?.name ?? (value && !loading ? '사용할 수 없는 모델' : '모델 선택')}</span><Icon name="chevron" class={`size-4 shrink-0 text-neutral-700 dark:text-neutral-300 ${open ? 'rotate-90' : ''}`} /></button>
{:else}
<input
{id}
bind:this={input}
role="combobox"
autocomplete="off"
aria-label={label}
aria-expanded={open}
aria-controls={open ? listId : undefined}
aria-autocomplete="list"
aria-activedescendant={activeDescendant}
value={open ? query : selectedModel?.name ?? (value && !loading ? '사용할 수 없는 모델' : '')}
disabled={selectDisabled}
onclick={() => { if (!open) void openMenu(); }}
oninput={onInput}
onkeydown={onKeydown}
class="min-h-11 min-w-0 w-full rounded-xl border border-neutral-300 bg-white px-3 pr-8 text-sm outline-none focus:border-sky-700 focus-visible:ring-2 focus-visible:ring-sky-700 disabled:cursor-not-allowed disabled:opacity-60 dark:border-neutral-700 dark:bg-neutral-900 dark:focus:border-sky-500"
/>
<Icon name="chevron" class={`pointer-events-none absolute right-3 top-8 size-4 text-neutral-700 dark:text-neutral-300 ${open ? 'rotate-90' : ''}`} />
{/if}
{#if open}
<div bind:this={popup} popover="manual" role={compact ? 'dialog' : undefined}
aria-label={compact ? `${label} 선택` : undefined}
onkeydown={onPopupKeydown}
class="fixed z-[100] m-0 flex flex-col overflow-hidden rounded-2xl border border-neutral-200 bg-white p-0 shadow-2xl backdrop:bg-transparent dark:border-neutral-700 dark:bg-neutral-900" style:height="{popupHeight}px" style:width="{popupWidth}px" style:left="{popupLeft}px" style:top="{popupTop}px">
{#if compact}
<input
bind:this={input}
role="combobox"
autocomplete="off"
aria-label="모델 검색"
aria-expanded="true"
aria-controls={listId}
aria-autocomplete="list"
aria-activedescendant={activeDescendant}
placeholder="모델 검색"
value={query}
oninput={onInput}
onkeydown={onKeydown}
class="m-2 mb-0 h-9 shrink-0 rounded-lg border border-neutral-200 bg-neutral-50 px-3 text-sm outline-none focus-visible:ring-2 focus-visible:ring-sky-700 dark:border-neutral-700 dark:bg-neutral-800 dark:text-white"
/>
{/if}
<div id={listId} role="listbox" aria-label={`${label} 목록`} class="min-h-0 flex-1 overflow-hidden">
{#key query.trim() ? 'search' : submenuProvider ?? 'providers'}
<div class="picker-stage h-full min-h-0">
{#if query.trim()}
<div role="presentation" class="min-h-0 h-full overflow-y-auto p-1.5">
{#each searchGroups as group (group.provider)}
<div role="group" aria-label={group.provider}>
<p class="sticky top-0 z-10 bg-white px-3 py-1.5 text-xs font-semibold text-neutral-500 dark:bg-neutral-900 dark:text-neutral-400">{group.provider}</p>
{#each group.models as model (model.modelId)}
{@const index = searchModels.findIndex((entry) => entry.modelId === model.modelId)}
<button
id={`${id}-option-${index}`}
type="button"
role="option"
aria-selected={value === model.modelId}
class={['flex min-h-10 w-full items-center justify-between gap-2 rounded-xl px-3 py-2 text-left text-sm focus-visible:outline-2 focus-visible:outline-sky-700', activeModelIndex === index || value === model.modelId ? 'bg-sky-50 text-sky-950 dark:bg-sky-950 dark:text-sky-100' : 'hover:bg-neutral-100 dark:hover:bg-neutral-800']}
onmousedown={(event) => event.preventDefault()}
onclick={() => choose(model.modelId)}
onpointermove={() => (activeModelIndex = index)}
>
<span class="min-w-0 truncate">{model.name}</span>
{#if model.modelId === value}<span aria-hidden="true" class="shrink-0 text-xs text-sky-700 dark:text-sky-300">선택됨</span>{/if}
</button>
{/each}
</div>
{/each}
{#if searchModels.length === 0}<p role="status" class="px-3 py-4 text-sm text-neutral-500 dark:text-neutral-400">일치하는 모델이 없습니다.</p>{/if}
</div>
{:else if submenuProvider}
<div class="flex h-full min-h-0 flex-col">
<div class="flex h-12 shrink-0 items-center gap-2 border-b border-neutral-100 px-2 dark:border-neutral-800">
<button type="button" aria-label="제공자 목록으로 돌아가기" title="뒤로" class="grid size-8 shrink-0 place-items-center rounded-lg text-neutral-600 hover:bg-neutral-100 focus-visible:outline-2 focus-visible:outline-sky-700 dark:text-neutral-300 dark:hover:bg-neutral-800" onmousedown={(event) => event.preventDefault()} onclick={() => void returnToRoot()}><Icon name="chevron" class="size-4 rotate-180" /></button>
<p class="min-w-0 truncate text-sm font-semibold text-neutral-900 dark:text-neutral-100">{submenuProvider}</p>
</div>
<div role="group" aria-label={`${submenuProvider} 모델`} class="min-h-0 flex-1 overflow-y-auto p-1.5">
{#each submenuModels as model, index (model.modelId)}
<button
id={`${id}-option-${index}`}
type="button"
role="option"
aria-selected={value === model.modelId}
class={['flex min-h-11 w-full items-center justify-between gap-2 rounded-xl px-3 py-2 text-left text-sm focus-visible:outline-2 focus-visible:outline-sky-700', activeModelIndex === index || value === model.modelId ? 'bg-sky-50 text-sky-950 dark:bg-sky-950 dark:text-sky-100' : 'hover:bg-neutral-100 dark:hover:bg-neutral-800']}
onmousedown={(event) => event.preventDefault()}
onclick={() => choose(model.modelId)}
onpointermove={() => (activeModelIndex = index)}
>
<span class="min-w-0 truncate">{model.name}</span>
{#if model.modelId === value}<span aria-hidden="true" class="shrink-0 text-xs text-sky-700 dark:text-sky-300">선택됨</span>{/if}
</button>
{/each}
</div>
</div>
{:else}
<div role="group" aria-label="제공자" class="min-h-0 h-full overflow-y-auto p-1.5">
{#each rootEntries as entry, index (`${entry.kind}-${entry.kind === 'auto' ? entry.model.modelId : entry.provider}`)}
<button
id={`${id}-root-option-${index}`}
type="button"
role="option"
aria-selected={entry.kind === 'auto' ? value === entry.model.modelId : selectedProvider === entry.provider}
class={['flex min-h-14 w-full items-center gap-3 rounded-xl px-3 py-2 text-left text-sm focus-visible:outline-2 focus-visible:outline-sky-700', activeRootIndex === index || entry.kind === 'provider' && selectedProvider === entry.provider || entry.kind === 'auto' && value === entry.model.modelId ? 'bg-sky-50 text-sky-950 dark:bg-sky-950 dark:text-sky-100' : 'hover:bg-neutral-100 dark:hover:bg-neutral-800']}
onmousedown={(event) => event.preventDefault()}
onclick={() => {
activeRootIndex = index;
if (entry.kind === 'auto') choose(entry.model.modelId);
else void openProvider(entry.provider, index);
}}
onpointermove={() => (activeRootIndex = index)}
>
{#if entry.kind === 'auto'}
<span aria-hidden="true" class="grid size-9 shrink-0 place-items-center rounded-xl bg-neutral-100 text-sm font-semibold text-neutral-600 dark:bg-neutral-800 dark:text-neutral-300">A</span>
<span class="min-w-0 flex-1 truncate">{entry.model.name}</span>
{#if entry.model.modelId === value}<span aria-hidden="true" class="shrink-0 text-xs text-sky-700 dark:text-sky-300">선택됨</span>{/if}
{:else}
<span aria-hidden="true" class={`grid size-9 shrink-0 place-items-center rounded-xl text-sm font-semibold ${providerBadgeClass(entry.provider)}`}>{providerInitial(entry.provider)}</span>
<span class="min-w-0 flex-1">
<span class="block truncate font-medium">{entry.provider}</span>
{#if selectedProvider === entry.provider && selectedModel}<span class="block truncate text-xs text-neutral-500 dark:text-neutral-400">{selectedModel.name}</span>{/if}
</span>
<Icon name="chevron" class="size-4 shrink-0 text-neutral-400" />
{/if}
</button>
{/each}
</div>
{/if}
</div>
{/key}
</div>
</div>
{/if}
{#if loading}
<p role="status" class="text-xs text-neutral-600 dark:text-neutral-300">모델 목록 불러오는 중</p>
{:else if error}
<p role="alert" class="text-xs text-red-700 dark:text-red-300">모델 목록을 불러오지 못했습니다. {error}</p>
{:else if catalog?.availableModels.length === 0}
<p role="status" class="text-xs text-neutral-600 dark:text-neutral-300">사용 가능한 모델이 없습니다.</p>
{/if}
</div>

<style>
.picker-stage {
animation: picker-stage-in 150ms cubic-bezier(0.2, 0.7, 0.2, 1) both;
}

@keyframes picker-stage-in {
from { opacity: 0; transform: translateY(4px); }
to { opacity: 1; transform: translateY(0); }
}

@media (prefers-reduced-motion: reduce) {
.picker-stage { animation: none; }
}
</style>
