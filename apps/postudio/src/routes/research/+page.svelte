<script lang="ts">
import { page } from '$app/state';
import { board } from '$lib/board.svelte';
import { ResearchMutationGate, researchApi, type ResearchPaper, type ResearchProject, type ResearchStatus } from '$lib/research';

const host = $derived(page.url.searchParams.get('host') || board.origin || undefined);
let status = $state<ResearchStatus | null>(null);
let projects = $state.raw<ResearchProject[]>([]);
let papers = $state.raw<ResearchPaper[]>([]);
let loading = $state(true);
let connecting = $state(false);
let registering = $state(false);
let error = $state('');
let projectError = $state('');
let paperError = $state('');
let name = $state('');
let path = $state('');
let query = $state('');
let searching = $state(false);
const mutationGate = new ResearchMutationGate();
let requestGeneration = 0;
let scopeGeneration = 0;
let paperGeneration = 0;

async function refresh() {
 const origin = host;
 const scope = scopeGeneration;
 const generation = ++requestGeneration;
 const api = researchApi(origin);
 loading = true;
 error = '';
 const [statusResult, projectsResult] = await Promise.allSettled([api.status(), api.projects()]);
 if (generation !== requestGeneration || scope !== scopeGeneration || origin !== host) return;
 if (statusResult.status === 'fulfilled') status = statusResult.value;
 else error = statusResult.reason instanceof Error ? statusResult.reason.message : '연구 런타임 상태를 확인하지 못했어요.';
 if (projectsResult.status === 'fulfilled') { projects = projectsResult.value; }
 else error = `${error ? `${error} · ` : ''}${projectsResult.reason instanceof Error ? projectsResult.reason.message : '프로젝트 목록을 불러오지 못했어요.'}`;
 loading = false;
}

async function connectRuntime() {
 if (connecting) return;
 const origin = host; const scope = scopeGeneration;
 const release = mutationGate.enter(`connect:${scope}`); if (!release) return;
 connecting = true; error = '';
 try {
  const next = await researchApi(origin).connect();
  if (scope !== scopeGeneration || origin !== host) return;
  status = next; await refresh();
 } catch (cause) {
  if (scope === scopeGeneration && origin === host) error = `연결 응답을 확인하지 못했어요. 다시 연결하기 전에 상태를 새로고침해 주세요. ${cause instanceof Error ? cause.message : ''}`;
 } finally { release(); if (scope === scopeGeneration && origin === host) connecting = false; }
}

async function registerProject(event: SubmitEvent) {
 event.preventDefault();
 if (registering || !name.trim() || !path.trim()) return;
 const origin = host; const scope = scopeGeneration; const projectName = name.trim(); const projectPath = path.trim();
 const release = mutationGate.enter(`register:${scope}`); if (!release) return;
 registering = true; projectError = '';
 try {
  await researchApi(origin).registerProject(projectName, projectPath);
  if (scope !== scopeGeneration || origin !== host) return;
  name = ''; path = '';
  await refresh();
 } catch (cause) {
  if (scope === scopeGeneration && origin === host) projectError = `등록 응답을 확인하지 못했어요. 중복 등록하지 말고 목록을 새로고침해 주세요. ${cause instanceof Error ? cause.message : ''}`;
 } finally { release(); if (scope === scopeGeneration && origin === host) registering = false; }
}

async function searchPapers(event: SubmitEvent) {
 event.preventDefault();
 const origin = host; const scope = scopeGeneration;
 const release = mutationGate.enter(`papers:${scope}`); if (!release) return;
 const generation = ++paperGeneration;
 if (!query.trim()) { papers = []; paperError = ''; release(); return; }
 searching = true; paperError = '';
 try {
  const result = await researchApi(origin).papers(query.trim());
  if (generation === paperGeneration && origin === host) papers = result;
 } catch (cause) {
  if (generation === paperGeneration && origin === host) paperError = cause instanceof Error ? cause.message : '논문 검색에 실패했어요.';
 } finally { release(); if (generation === paperGeneration && scope === scopeGeneration && origin === host) searching = false; }
}

$effect(() => {
 const origin = host;
 scopeGeneration++; requestGeneration++; paperGeneration++;
 status = null; projects = []; papers = []; error = ''; projectError = ''; paperError = '';
 connecting = false; registering = false; searching = false;
 void refresh();
 return () => { requestGeneration++; paperGeneration++; };
});
</script>

<svelte:head><title>연구 워크스페이스 · Postudio</title></svelte:head>

<div class="min-h-0 flex-1 overflow-y-auto bg-neutral-50 px-4 py-6 dark:bg-neutral-950 sm:px-8">
 <div class="mx-auto max-w-5xl space-y-6">
  <header class="flex flex-wrap items-start justify-between gap-3">
   <div><p class="text-xs font-medium text-neutral-500">Postudio · 로컬 연구</p><h1 class="mt-1 text-2xl font-semibold">연구 워크스페이스</h1><p class="mt-2 text-sm text-neutral-600 dark:text-neutral-300">실험 프로젝트를 등록하고 결과와 논문 자료를 살펴보세요.</p></div>
   <div class="flex items-center gap-2 rounded-xl border border-neutral-200 bg-white px-3 py-2 text-sm dark:border-neutral-800 dark:bg-neutral-900">
    <span class="size-2 rounded-full" class:bg-emerald-500={status?.connected} class:bg-amber-500={!status?.connected}></span>
    <span>{status?.connected ? `연결됨${status.version ? ` · ${status.version}` : ''}` : status?.available ? '연결되지 않았어요' : '런타임을 사용할 수 없어요'}</span>
    <button type="button" class="ml-2 text-xs underline disabled:opacity-50" disabled={loading || connecting} onclick={() => void refresh()}>상태 새로고침</button>
    {#if !status?.connected && status?.available}<button class="ml-2 rounded-lg bg-neutral-900 px-3 py-1.5 text-xs font-medium text-white disabled:opacity-50 dark:bg-white dark:text-neutral-900" disabled={connecting} onclick={() => void connectRuntime()}>{connecting ? '연결 중…' : '연결하기'}</button>{/if}
   </div>
  </header>

  {#if error}<p role="alert" class="rounded-xl border border-red-200 bg-red-50 p-3 text-sm text-red-800 dark:border-red-900 dark:bg-red-950 dark:text-red-200">연구 상태를 가져오지 못했어요. {error}<button class="ml-2 underline" onclick={() => void refresh()}>다시 불러오기</button></p>{/if}
  {#if status?.error}<p role="status" class="rounded-xl border border-amber-200 bg-amber-50 p-3 text-sm text-amber-900 dark:border-amber-900 dark:bg-amber-950 dark:text-amber-100">{status.error}</p>{/if}

  <section class="rounded-2xl border border-neutral-200 bg-white p-5 dark:border-neutral-800 dark:bg-neutral-900">
   <div class="mb-4"><h2 class="text-lg font-semibold">프로젝트</h2><p class="mt-1 text-sm text-neutral-600 dark:text-neutral-300">이미 있는 Git 저장소만 등록해요. 저장소를 만들거나 복제하지 않아요.</p></div>
   {#if loading && projects.length === 0}<p role="status" class="py-3 text-sm text-neutral-500">프로젝트를 불러오는 중이에요.</p>{:else if projects.length === 0}<p class="rounded-xl bg-neutral-50 p-4 text-sm text-neutral-600 dark:bg-neutral-800 dark:text-neutral-300">등록된 프로젝트가 없어요.</p>{:else}<ul class="mb-4 divide-y divide-neutral-100 dark:divide-neutral-800">{#each projects as project (project.id)}<li><a href="/research/{encodeURIComponent(project.id)}{host ? `?host=${encodeURIComponent(host)}` : ''}" class="flex min-h-14 flex-wrap items-center gap-2 py-3 hover:text-blue-700 dark:hover:text-blue-300"><span class="min-w-0 flex-1 truncate font-medium">{project.name}</span><span class="max-w-full truncate text-xs text-neutral-500" title={project.repoPath}>{project.repoPath}</span></a></li>{/each}</ul>{/if}
   <form class="grid gap-3 border-t border-neutral-100 pt-4 dark:border-neutral-800 sm:grid-cols-[1fr_2fr_auto]" onsubmit={registerProject}>
    <label class="grid gap-1 text-xs font-medium text-neutral-600 dark:text-neutral-300">프로젝트 이름<input bind:value={name} required maxlength="120" class="h-10 rounded-lg border border-neutral-300 bg-transparent px-3 text-sm dark:border-neutral-700" placeholder="예: 모델 평가" /></label>
    <label class="grid gap-1 text-xs font-medium text-neutral-600 dark:text-neutral-300">기존 Git 저장소 경로<input bind:value={path} required class="h-10 rounded-lg border border-neutral-300 bg-transparent px-3 text-sm dark:border-neutral-700" placeholder="/Users/me/project" /></label>
    <button class="self-end rounded-lg bg-blue-700 px-4 py-2.5 text-sm font-medium text-white disabled:opacity-50" disabled={!status?.connected || registering}>{registering ? '등록 중…' : '등록하기'}</button>
   </form>
   {#if projectError}<p role="alert" class="mt-3 text-sm text-red-700 dark:text-red-300">프로젝트를 등록하지 못했어요. {projectError}</p>{/if}
  </section>

  <section class="rounded-2xl border border-neutral-200 bg-white p-5 dark:border-neutral-800 dark:bg-neutral-900">
   <h2 class="text-lg font-semibold">논문 검색</h2><p class="mt-1 text-sm text-neutral-600 dark:text-neutral-300">alphaXiv 검색 결과를 표시해요. 검색이 되지 않으면 연결이나 서비스 상태를 확인해 주세요.</p>
   <form class="mt-4 flex gap-2" onsubmit={searchPapers}><label class="sr-only" for="paper-query">논문 검색어</label><input id="paper-query" bind:value={query} class="h-10 min-w-0 flex-1 rounded-lg border border-neutral-300 bg-transparent px-3 text-sm dark:border-neutral-700" placeholder="논문 제목이나 키워드" /><button class="rounded-lg border border-neutral-300 px-4 text-sm font-medium disabled:opacity-50 dark:border-neutral-700" disabled={searching || !query.trim()}>{searching ? '검색 중…' : '검색'}</button></form>
   {#if paperError}<p role="alert" class="mt-3 text-sm text-red-700 dark:text-red-300">논문을 검색하지 못했어요. {paperError}</p>{:else if papers.length}<ul class="mt-4 divide-y divide-neutral-100 dark:divide-neutral-800">{#each papers as paper (paper.id)}<li class="py-3"><div class="font-medium">{paper.title}</div><p class="mt-1 line-clamp-3 text-sm text-neutral-600 dark:text-neutral-300">{paper.abstract || '초록 정보가 제공되지 않았어요.'}</p>{#if paper.url}<a href={paper.url} target="_blank" rel="noopener noreferrer" class="mt-2 inline-block text-xs text-blue-700 underline dark:text-blue-300">논문 원문 열기</a>{/if}</li>{/each}</ul>{:else if query.trim() && !searching && !paperError}<p class="mt-3 text-sm text-neutral-500">검색 결과가 없어요.</p>{/if}
  </section>
 </div>
</div>
