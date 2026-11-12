<script lang="ts">
 import { page } from '$app/state';
 import { getJson } from '$lib/api';
 import { configuredUntrustedOrigin, type PortEntry } from '$lib/artifact-browser';
 import { sessionOrigin } from '$lib/fleet';
 import type { NodeInfo } from '$lib/types';
 const requestedHost = $derived(page.url.searchParams.get('host'));
 const hostState = $derived.by(() => {
  try { return { host: sessionOrigin(page.url.search), error: '' }; }
  catch { return { host: null, error: '호스트 주소가 설정 목록에 없거나 유효하지 않습니다. 설정에서 호스트를 확인하세요.' }; }
 });
 const host = $derived(hostState.host);
 let ports = $state<PortEntry[]>([]);
 let origin = $state<string | null>(null);
 let error = $state('');
 let requestGeneration = 0;
 $effect(() => {
  const pinnedHost = host, hasRequestedHost = requestedHost !== null;
  const generation = ++requestGeneration;
  ports = []; origin = null; error = hostState.error;
  if (hostState.error) return;
  if (hasRequestedHost && !pinnedHost) { error = '호스트 주소가 설정 목록에 없거나 유효하지 않습니다. 설정에서 호스트를 확인하세요.'; return; }
  const apiHost = pinnedHost ?? (typeof window !== 'undefined' ? window.location.origin : '');
  if (!apiHost) { error = '호스트 주소를 확인할 수 없습니다.'; return; }
  void getJson<NodeInfo>('/api/node', apiHost).then((node) => {
   if (generation === requestGeneration) origin = configuredUntrustedOrigin(node, apiHost);
  }).catch(() => { if (generation === requestGeneration) error = '노드 정보를 불러오지 못했습니다.'; });
  void getJson<{ ports: PortEntry[] }>('/api/ports', apiHost).then((result) => {
   if (generation === requestGeneration) ports = result.ports;
  }).catch(() => { if (generation === requestGeneration) error = '수신 대기 포트를 찾을 수 없습니다.'; });
 });
</script>
<svelte:head><title>포트 · Postudio</title></svelte:head>
<section class="mx-auto flex w-full max-w-4xl flex-col gap-4 overflow-auto p-4">
 <h1 class="text-xl font-semibold">수신 대기 포트</h1>
 {#if error}<p role="alert" class="text-red-600">{error}</p>{/if}
 {#if !origin}<p class="rounded bg-amber-100 p-3 text-sm text-amber-950">미리보기를 사용할 수 없습니다. 노드에 유효한 untrustedOrigin 설정이 필요합니다. --untrusted-origin을 설정하세요.</p>{/if}
 {#if ports.length === 0 && !error}<p class="text-sm text-neutral-500">연결 가능한 수신 대기 포트가 없습니다.</p>{/if}
 <ul class="divide-y rounded border">{#each ports as item (item.port)}
  <li class="flex items-center justify-between gap-4 p-3"><span>:{item.port}{#if item.process} <span class="text-neutral-500">{item.process}</span>{/if}</span>
  {#if origin}<a class="underline" href="{origin}/p/{item.port}/" target="_blank" rel="noreferrer">열기</a>{/if}</li>
 {/each}</ul>
</section>
