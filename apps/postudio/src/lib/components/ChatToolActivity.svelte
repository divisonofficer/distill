<script lang="ts">
import type { SearchActivity, ChatSource } from '$lib/chat-activity';

let { calls, sources, live = false, writing = false, showSources = false, sourceScope = '', highlighted = 0 }: {
 calls: SearchActivity[]; sources: ChatSource[]; live?: boolean; writing?: boolean; showSources?: boolean; sourceScope?: string; highlighted?: number;
} = $props();
let expandedSources = $state(false);
$effect(() => { if (highlighted > 6) expandedSources = true; });
const allSourcesVisible = $derived(expandedSources || highlighted > 6);
const papersOnly = $derived(calls.length > 0 && calls.every((call) => ['alphaXiv 논문 검색', '논문 본문 읽기', '논문 원문 그림'].includes(call.label)));
const isFailed = (status: string) => status === '검색을 완료하지 못함' || status === '본문을 가져오지 못함' || status === '그림을 가져오지 못함';
const current = $derived(calls.at(-1));
const summary = $derived(calls.every((call) => call.label === 'alphaXiv 논문 검색') ? `논문 ${calls.length}번 검색` : `도구 ${calls.length}개 사용`);
const failed = $derived(calls.some((call) => isFailed(call.status)));
const uniqueCalls = $derived.by(() => {
 const seen = new Set<string>();
 return calls.map((call) => ({ ...call, hits: call.hits.filter((hit) => {
  if (call.label === '논문 본문 읽기') return true;
  if (seen.has(hit.url)) return false;
  seen.add(hit.url);
  return true;
 }) }));
});
</script>

{#snippet rows()}
 <div class="space-y-1 border-t border-neutral-200/80 px-2 py-1 dark:border-neutral-700">
 {#each uniqueCalls as call}
  {@const paper = call.label === '논문 본문 읽기' && call.hits[0] ? (sources.find((source) => source.url === call.hits[0].url) ?? call.hits[0]) : undefined}
  <div class={['rounded-lg px-2 py-1', isFailed(call.status) && 'bg-red-50 dark:bg-red-950/50']}>
   <div class="flex min-h-7 min-w-0 items-center gap-2 text-[13px]">
    {#if call.status === '실행 중…'}<span class="spinner" aria-hidden="true"></span>{:else if isFailed(call.status)}<span class="text-red-700 dark:text-red-300" aria-hidden="true">×</span>{:else}<span class="text-green-700 dark:text-green-400" aria-hidden="true">✓</span>{/if}
    <span class="min-w-0 flex-1 truncate font-semibold">{call.label}</span>
    <span class="shrink-0 text-xs text-neutral-600 dark:text-neutral-300">{isFailed(call.status) ? '실패' : call.status}</span>
   </div>
   {#if isFailed(call.status)}<p class="pl-5 text-xs text-red-700 dark:text-red-300">{call.label === '논문 본문 읽기' ? '추출된 본문을 가져오지 못했어요.' : call.label === '논문 원문 그림' ? 'arXiv HTML에서 원문 그림을 가져오지 못했어요.' : '검색 결과를 가져오지 못했어요.'}</p>{/if}
   {#if paper}<a href={paper.url} target="_blank" rel="noopener noreferrer" title={paper.title} class="ml-5 flex w-fit max-w-[calc(100%-1.25rem)] flex-col rounded-xl border border-neutral-200 bg-white px-2.5 py-1.5 text-xs dark:border-neutral-700 dark:bg-neutral-900"><span class="font-semibold">{paper.citation ?? 'arXiv · 서지 정보 미확인'}</span><span class="line-clamp-2 text-neutral-600 dark:text-neutral-300">{paper.title}</span></a>
   {:else if call.hits.length}<div class="flex min-w-0 flex-wrap gap-1 pb-1 pl-5">
    {#each call.hits.slice(0, 4) as hit (hit.url)}<a href={hit.url} target="_blank" rel="noopener noreferrer" title={hit.title} class="flex max-w-[min(280px,100%)] items-center gap-1 rounded-full border border-neutral-200 bg-white px-2 py-1 text-xs dark:border-neutral-700 dark:bg-neutral-900"><span class="truncate">{hit.title}</span>{#if hit.date}<span class="shrink-0 text-neutral-500">{hit.date.slice(5)}</span>{/if}</a>{/each}
    {#if call.hits.length > 4}<span class="self-center px-1 text-xs text-neutral-500">외 {call.hits.length - 4}건</span>{/if}
   </div>{/if}
  </div>
 {/each}
 </div>
{/snippet}

{#if showSources && sources.length}
 <section aria-label="참고한 자료" class="mt-3 space-y-2">
  <h4 class="text-xs font-semibold text-neutral-600 dark:text-neutral-300">{papersOnly ? `참고한 논문 ${sources.length}편` : `참고한 자료 ${sources.length}건`}</h4>
  <div class="grid grid-cols-[repeat(auto-fit,minmax(min(100%,220px),1fr))] gap-1.5">
  {#each (allSourcesVisible ? sources : sources.slice(0, 6)) as source, index (source.url)}
   <a id={`${sourceScope}-source-${index + 1}`} href={source.url} target="_blank" rel="noopener noreferrer" class={['flex min-w-0 gap-2 rounded-[10px] border px-2.5 py-2 hover:border-neutral-400', highlighted === index + 1 ? 'border-blue-600 ring-2 ring-blue-200 dark:ring-blue-800' : 'border-neutral-200 dark:border-neutral-700']}>
    <span class="font-mono text-xs font-bold text-blue-700 dark:text-blue-300">{index + 1}</span><span class="min-w-0"><span class="line-clamp-2 text-xs font-semibold">{source.title}</span><span class="text-[11px] text-neutral-500">{source.citation ?? `${new URL(source.url).hostname}${source.date ? ` · ${source.date}` : ''}`}</span></span>
   </a>
  {/each}
  </div>
  {#if sources.length > 6}<button type="button" aria-expanded={allSourcesVisible} class="min-h-8 text-xs font-semibold text-blue-700 dark:text-blue-300" onclick={() => (expandedSources = !allSourcesVisible)}>{allSourcesVisible ? '접기 ▴' : `나머지 ${sources.length - 6}${papersOnly ? '편' : '건'} 보기 ▾`}</button>{/if}
 </section>
{:else if !showSources && calls.length}
 {#if live}
  <div class="mb-2 rounded-xl border border-[#d7e2fa] bg-[#f8faff] dark:border-blue-900 dark:bg-blue-950/30">
   <div role="status" class="flex min-h-10 min-w-0 items-center gap-2 px-3 text-[13.5px] font-semibold"><span class="spinner" aria-hidden="true"></span><span class="min-w-0 grow truncate">{writing ? '답변 쓰는 중' : `${current?.label ?? '도구'} 실행 중`}</span><span class="font-mono text-xs text-blue-700 dark:text-blue-300">{calls.length}</span></div>
   {@render rows()}
  </div>
 {:else}
  <details class="mb-2 rounded-xl border border-neutral-200 bg-neutral-50 dark:border-neutral-800 dark:bg-neutral-900">
   <summary class="min-h-10 cursor-pointer px-3 py-2 text-[13.5px] font-semibold"><span class={failed ? 'text-red-700 dark:text-red-300' : 'text-green-700 dark:text-green-400'} aria-hidden="true">{failed ? '!' : '✓'}</span> {summary}<span class="ml-2 text-xs font-normal text-neutral-500">{sources.length ? `자료 ${sources.length}건` : ''}{failed ? ' · 일부 도구 실패' : ''}</span></summary>
   {@render rows()}
  </details>
 {/if}
{/if}

<style>
.spinner { width: 14px; height: 14px; flex-shrink: 0; border: 2px solid #d7e2fa; border-top-color: #2b59c3; border-radius: 50%; animation: rotate 1s linear infinite; }
@keyframes rotate { to { transform: rotate(360deg); } }
@media (prefers-reduced-motion: reduce) { .spinner { animation: none; } }
</style>
