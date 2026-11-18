<script lang="ts">
 import { page } from '$app/state';
 import { artifactEntries, artifactUrl, configuredUntrustedOrigin, type ArtifactEntry } from '$lib/artifact-browser';
 import { getJson } from '$lib/api';
 import { sessionOrigin } from '$lib/fleet';
 import type { NodeInfo } from '$lib/types';
 const id = $derived(page.params.id ?? '');
 const requestedHost = $derived(page.url.searchParams.get('host'));
 const hostState = $derived.by(() => {
  try { return { host: sessionOrigin(page.url.search), error: '' }; }
  catch { return { host: null, error: '호스트 주소가 설정 목록에 없거나 유효하지 않습니다. 설정에서 호스트를 확인하세요.' }; }
 });
 const host = $derived(hostState.host);
 let entries = $state<ArtifactEntry[]>([]);
 let path = $state('');
 let origin = $state<string | null>(null);
 let error = $state('');
 let selected = $state<ArtifactEntry | null>(null);
 let requestGeneration = 0;
 $effect(() => {
  const sessionId = id, pinnedHost = host, hasRequestedHost = requestedHost !== null;
  const generation = ++requestGeneration;
  path = ''; selected = null; entries = []; origin = null; error = hostState.error;
  if (hostState.error) return;
  if (hasRequestedHost && !pinnedHost) { error = '호스트 주소가 설정 목록에 없거나 유효하지 않습니다. 설정에서 호스트를 확인하세요.'; return; }
  const apiHost = pinnedHost ?? (typeof window !== 'undefined' ? window.location.origin : '');
  if (!apiHost) { error = '호스트 주소를 확인할 수 없습니다.'; return; }
  void getJson<NodeInfo>('/api/node', apiHost).then((node) => {
   if (generation !== requestGeneration) return;
   origin = configuredUntrustedOrigin(node, apiHost);
  }).catch(() => {
   if (generation === requestGeneration) error = '노드 정보를 불러오지 못했습니다.';
  });
  void load('', sessionId, apiHost, generation);
 });
 async function load(next: string, sessionId = id, apiHost = host ?? (typeof window !== 'undefined' ? window.location.origin : ''), generation = requestGeneration) {
  path = next; selected = null; error = '';
  try { const result = await artifactEntries(sessionId, next, apiHost); if (generation === requestGeneration) entries = result; }
  catch { if (generation === requestGeneration) error = '결과물 목록을 불러오지 못했습니다.'; }
 }
 function open(entry: ArtifactEntry) { if (entry.kind === 'directory') void load(entry.path); else selected = entry; }
 function href(entry: ArtifactEntry) { return origin ? artifactUrl(origin, id, entry.path) : ''; }
 const backHref = $derived(`/s/${encodeURIComponent(id)}${requestedHost ? `?host=${encodeURIComponent(requestedHost)}` : ''}`);
</script>
<svelte:head><title>세션 결과물 · {id} · Postudio</title></svelte:head>
<section class="mx-auto flex w-full max-w-4xl flex-col gap-4 overflow-auto p-4">
 <a class="text-sm underline" href={backHref}>← 세션</a>
 <h1 class="text-xl font-semibold">세션 결과물</h1>
 <p class="break-all text-sm text-neutral-500">{path || '세션 폴더'}</p>
 {#if error}<p role="alert" class="text-red-600">{error}</p>{/if}
 {#if path}<button class="self-start rounded border px-3 py-2" onclick={() => load(path.split('/').slice(0, -1).join('/'))}>상위 폴더</button>{/if}
 {#if !origin}<p class="rounded bg-amber-100 p-3 text-sm text-amber-950">미리보기를 사용할 수 없습니다. 노드에 유효한 untrustedOrigin 설정이 필요합니다. --untrusted-origin을 설정하세요. 폴더 목록은 계속 볼 수 있습니다.</p>{/if}
 <ul class="divide-y rounded border">{#each entries as entry (entry.path)}
  <li class="flex items-center justify-between gap-3 p-3"><button class="min-w-0 break-all text-left underline" onclick={() => open(entry)}>{entry.kind === 'directory' ? '폴더: ' : '파일: '}{entry.name}</button>
  {#if entry.kind === 'file' && origin}<a class="shrink-0 text-sm underline" href={href(entry)} target="_blank" rel="noreferrer">열기</a>{/if}</li>
 {/each}</ul>
 {#if selected && origin}
  {#if selected.mime === 'image/png' || selected.mime === 'image/jpeg' || selected.mime === 'image/gif' || selected.mime === 'image/webp'}
   <img class="max-h-[70vh] max-w-full self-center object-contain" src={href(selected)} alt={selected.name} />
  {:else if selected.mime === 'text/html'}
   <iframe class="h-[70vh] w-full rounded border" src={href(selected)} title={selected.name} sandbox="allow-scripts"></iframe>
  {:else}<a class="underline" href={href(selected)} target="_blank" rel="noreferrer">{selected.name} 열기</a>{/if}
 {/if}
</section>
