<script lang="ts">
 import { researchApi, type ResearchPaper, type ResearchSource, type ProjectResearchRefs, type ResearchExperiment } from '../../research';
 import { safeResearchUrl } from '../../research-guide';

 type Props = {
  host?: string;
  projectId: string;
  refs: ProjectResearchRefs;
  experiments: ResearchExperiment[];
  onChange: (payload: { host?: string; projectId: string; refs: ProjectResearchRefs }) => void;
 };
 let { host, projectId, refs, experiments, onChange }: Props = $props();

 const sources: { id: ResearchSource; label: string }[] = [
  { id: 'alphaXiv', label: 'alphaXiv' },
  { id: 'openAlex', label: 'OpenAlex' },
  { id: 'bioRxiv', label: 'bioRxiv' },
  { id: 'pubMed', label: 'PubMed' }
 ];
 let query = $state('');
 let selectedSources = $state<ResearchSource[]>(['alphaXiv','openAlex']);
 let results = $state<ResearchPaper[]>([]);
 let sourceErrors = $state<Record<string, string>>({});
 let searchError = $state('');
 let searching = $state(false);
 let actionError = $state('');
 let pendingIds = $state<string[]>([]);
 let searchGeneration = 0;
 let scopeGeneration = 0;
 let scopeKey = $derived(`${host ?? ''}\u0000${projectId}`);
 let linkedIds = $derived(new Set(refs.literature.map((paper) => paper.id)));
 let currentExperiments = $derived(experiments.filter((experiment) => experiment.projectId === projectId));

 function experimentUsage(paperId: string): string[] {
  return currentExperiments
   .filter((experiment) => refs.experiments[experiment.id]?.literatureIds.includes(paperId))
   .map((experiment) => experiment.title || experiment.slug);
 }

 function notify(next: ProjectResearchRefs, capturedScope: string, generation: number) {
  if (scopeKey !== capturedScope || scopeGeneration !== generation) return;
  onChange({ host, projectId, refs: next });
 }

 function scopeChanged() {
  scopeGeneration += 1;
  searchGeneration += 1;
  results = [];
  sourceErrors = {};
  searchError = '';
  actionError = '';
  pendingIds = [];
  searching = false;
 }
 $effect(() => {
  void scopeKey;
  scopeChanged();
 });

 function invalidateSearch() {
  searchGeneration += 1;
  searching = false;
 }

 function updateQuery(value: string) {
  query = value;
  invalidateSearch();
 }

 function toggleSource(source: ResearchSource) {
  invalidateSearch();
  selectedSources = selectedSources.includes(source)
   ? selectedSources.filter((item) => item !== source)
   : [...selectedSources, source];
 }

 async function search(event?: SubmitEvent) {
  event?.preventDefault();
  const term = query.trim();
  if (!term || selectedSources.length === 0 || searching) return;
  const generation = ++searchGeneration;
  const capturedScope = scopeKey;
  const scopeVersion = scopeGeneration;
  searching = true;
  searchError = '';
  sourceErrors = {};
  try {
   const response = await researchApi(host).searchPapers(term, [...selectedSources]);
   if (generation !== searchGeneration || scopeKey !== capturedScope || scopeGeneration !== scopeVersion) return;
   results = response.papers;
   sourceErrors = response.sourceErrors ?? {};
  } catch (error) {
   if (generation !== searchGeneration || scopeKey !== capturedScope || scopeGeneration !== scopeVersion) return;
   searchError = error instanceof Error ? error.message : '문헌 검색에 실패했습니다.';
  } finally {
   if (generation === searchGeneration && scopeKey === capturedScope && scopeGeneration === scopeVersion) searching = false;
  }
 }

 async function togglePaper(paper: ResearchPaper) {
  if (pendingIds.includes(paper.id)) return;
  const capturedScope = scopeKey;
  const scopeVersion = scopeGeneration;
  pendingIds = [...pendingIds, paper.id];
  actionError = '';
  try {
   const next = linkedIds.has(paper.id)
    ? await researchApi(host).unlinkPaper(projectId, paper.id)
    : await researchApi(host).linkPaper(projectId, paper);
   notify(next, capturedScope, scopeVersion);
  } catch (error) {
   if (scopeKey === capturedScope && scopeGeneration === scopeVersion) {
    actionError = error instanceof Error ? error.message : '문헌 연결을 변경하지 못했습니다.';
   }
  } finally {
   if (scopeKey === capturedScope && scopeGeneration === scopeVersion) {
    pendingIds = pendingIds.filter((id) => id !== paper.id);
   }
  }
 }

 async function unlink(paper: ResearchPaper) {
  await togglePaper(paper);
 }
</script>

<section class="literature" aria-label="프로젝트 문헌">
 <header class="lit-head">
  <div><h2>연결된 문헌</h2><span class="count">{refs.literature.length}</span></div>
  <p>프로젝트와 실험에 연결된 논문</p>
 </header>
 {#if refs.literature.length}
  <ul class="paper-list linked-list" aria-label="연결된 문헌 목록">
   {#each refs.literature as paper (paper.id)}
    {@const usages = experimentUsage(paper.id)}
    <li class="lit-row">
     <div class="paper-content">
      {#if safeResearchUrl(paper.url)}<a class="paper-title" href={safeResearchUrl(paper.url)} target="_blank" rel="noopener noreferrer">{paper.title}</a>{:else}<h3 class="paper-title">{paper.title}</h3>{/if}
      <p class="metadata">{[paper.source, paper.year, paper.venue, paper.authors?.length ? paper.authors.join(', ') : null].filter(Boolean).join(' · ')}</p>
      {#if paper.abstract}<p class="snippet">{paper.abstract}</p>{/if}
      {#if usages.length}<p class="used">사용 중: {usages.join(', ')}</p>{/if}
     </div>
     <button class="link-button linked" type="button" aria-pressed="true" disabled={pendingIds.includes(paper.id)} onclick={() => unlink(paper)}>{pendingIds.includes(paper.id) ? '처리 중' : '연결 해제'}</button>
    </li>
   {/each}
  </ul>
 {:else}
  <p class="empty">아직 연결된 문헌이 없습니다.</p>
 {/if}

 <section class="search-section" aria-labelledby="literature-search-title">
  <h2 id="literature-search-title">문헌 검색</h2>
  <form class="search-form" onsubmit={search}>
   <label class="visually-hidden" for="literature-query">논문 제목 또는 검색어</label>
   <input id="literature-query" type="search" value={query} oninput={(event) => updateQuery(event.currentTarget.value)} placeholder="논문 제목 또는 검색어" autocomplete="off" />
   <button class="search-button" type="submit" disabled={!query.trim() || selectedSources.length === 0 || searching}>{searching ? '검색 중…' : '검색'}</button>
  </form>
  <div class="source-list" aria-label="검색 출처">
   {#each sources as source (source.id)}
    <button type="button" class="source-chip" aria-pressed={selectedSources.includes(source.id)} onclick={() => toggleSource(source.id)}>{source.label}</button>
   {/each}
  </div>
  {#if selectedSources.length === 0}<p class="notice" role="status">검색 출처를 하나 이상 선택하세요.</p>{/if}
  {#if searchError}<p class="error" role="alert">{searchError}</p>{/if}
  {#each Object.entries(sourceErrors) as [source, message] (source)}
   <p class="partial-error" role="status">{source}: {message}</p>
  {/each}
  {#if results.length}
   <ul class="paper-list results-list" aria-label="검색 결과">
    {#each results as paper (paper.id)}
     <li class="lit-row">
      <div class="paper-content">
       {#if safeResearchUrl(paper.url)}<a class="paper-title" href={safeResearchUrl(paper.url)} target="_blank" rel="noopener noreferrer">{paper.title}</a>{:else}<h3 class="paper-title">{paper.title}</h3>{/if}
       <p class="metadata">{[paper.source, paper.year, paper.venue, paper.authors?.length ? paper.authors.join(', ') : null].filter(Boolean).join(' · ')}</p>
       {#if paper.abstract}<p class="snippet">{paper.abstract}</p>{/if}
      </div>
      <button class="link-button" class:linked={linkedIds.has(paper.id)} type="button" aria-pressed={linkedIds.has(paper.id)} disabled={pendingIds.includes(paper.id)} onclick={() => togglePaper(paper)}>{pendingIds.includes(paper.id) ? '처리 중' : linkedIds.has(paper.id) ? '연결됨' : '연결'}</button>
     </li>
    {/each}
   </ul>
  {:else if searching}
   <p class="empty" role="status">검색 중…</p>
  {:else if query.trim() && !searchError && Object.keys(sourceErrors).length === 0}
   <p class="empty" role="status">검색 결과가 없습니다.</p>
  {/if}
  {#if actionError}<p class="error" role="alert">{actionError}</p>{/if}
 </section>
</section>

<style>
 .literature{min-width:0;background:var(--bg,#fff);border:1px solid var(--line-soft,#ececef);border-radius:var(--radius-card,16px);color:var(--text,#1a1a1c)}
 .lit-head{padding:14px 18px 10px;border-bottom:1px solid var(--line-faint,#f0f0f2)}
 .lit-head>div{display:flex;align-items:center;gap:8px}.lit-head h2,.search-section h2{margin:0;font-size:15px;font-weight:600}.lit-head p{margin:4px 0 0;color:var(--text-4,#66666b);font-size:12px}.count{font:600 11.5px var(--font-mono,monospace);color:var(--text-4,#66666b)}
 .paper-list{list-style:none;padding:0;margin:0}.lit-row{display:flex;align-items:flex-start;gap:12px;padding:12px 18px;border-top:1px solid var(--line-faint,#f0f0f2)}.lit-row:first-child{border-top:0}.paper-content{flex:1;min-width:0}.paper-title{display:block;margin:0;color:var(--text,#1a1a1c);font-size:14px;font-weight:600;line-height:1.45;overflow-wrap:anywhere}.metadata{margin:4px 0 0;color:var(--text-4,#66666b);font-size:12px;line-height:1.5;overflow-wrap:anywhere}.snippet{margin:6px 0 0;color:var(--text-3,#55555a);font-size:13px;line-height:1.55;white-space:pre-line}.used{margin:6px 0 0;color:var(--blue-text,#1f48a3);font-size:12px}.empty{margin:0;padding:14px 18px;color:var(--text-5,#8a8a90);font-size:13px}
 .link-button,.search-button{flex:none;min-height:30px;padding:0 12px;border:1px solid var(--line-strong,#dcdce0);border-radius:8px;background:var(--bg,#fff);color:var(--text-2,#3a3a3e);font:600 12px var(--font-sans,system-ui);cursor:pointer}.link-button.linked{border-color:var(--green-line,#cfe6d8);background:var(--green-soft,#eef8f2);color:var(--green-text,#17693f)}button:disabled{opacity:.6;cursor:wait}
 .search-section{padding:16px 18px 18px;border-top:1px solid var(--line-soft,#ececef)}.search-section h2{margin-bottom:10px}.search-form{display:flex;gap:8px}.search-form input{flex:1;min-width:0;height:42px;padding:0 14px;border:1px solid var(--line-strong,#dcdce0);border-radius:12px;background:var(--bg,#fff);color:var(--text,#1a1a1c);font:14px var(--font-sans,system-ui)}.search-button{height:42px;padding:0 16px}.source-list{display:flex;flex-wrap:wrap;gap:7px;margin-top:10px}.source-chip{min-height:28px;padding:0 10px;border:1px solid var(--line,#e6e6e8);border-radius:14px;background:var(--bg,#fff);color:var(--text-3,#55555a);font:600 11.5px var(--font-sans,system-ui);cursor:pointer}.source-chip[aria-pressed="true"]{border-color:var(--blue-line,#d7e2fa);background:var(--blue-bg,#e8eefc);color:var(--blue-text,#1f48a3)}.results-list{margin-top:14px;border-top:1px solid var(--line-faint,#f0f0f2)}
 .notice,.partial-error{margin:9px 0 0;color:var(--amber-text,#8a4a00);font-size:12px;overflow-wrap:anywhere}.error{margin:10px 0 0;color:var(--red-text,#a52f22);font-size:12px;overflow-wrap:anywhere}.visually-hidden{position:absolute;width:1px;height:1px;padding:0;margin:-1px;overflow:hidden;clip:rect(0,0,0,0);white-space:nowrap;border:0}
 :global(.research-ui) button:focus-visible,:global(.research-ui) a:focus-visible,:global(.research-ui) input:focus-visible{outline:2px solid var(--blue,#2b59c3);outline-offset:2px}
 @media(max-width:600px){.lit-row{padding:12px;gap:8px}.lit-head{padding-inline:14px}.search-section{padding-inline:14px}.link-button{padding-inline:9px}.metadata{font-size:11.5px}}
 @media(prefers-reduced-motion:reduce){.literature *{scroll-behavior:auto}}
</style>
