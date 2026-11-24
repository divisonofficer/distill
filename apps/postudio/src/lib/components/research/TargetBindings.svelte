<script lang="ts">
 import { researchApi, type ResearchTargetBinding, type ResearchTarget } from '../../research';
 import { configuredHosts } from '../../fleet';

 type Candidate = { alias: string; label: string };
 type Props = { host?: string; projectId: string; onChange: () => void };
 let { host, projectId, onChange }: Props = $props();
 let targets = $state<ResearchTarget[]>([]);
 let bindings = $state<ResearchTargetBinding[]>([]);
 let candidates = $state<Candidate[]>([]);
 let sourceError = $state('');
 let error = $state('');
 let loading = $state(false);
 let saving = $state(false);
 let selectedOrigin = $state('');
 let selectedAlias = $state('');
 let generation = 0;
 let scope = $derived(`${host ?? ''}\u0000${projectId}`);
 let origins = $derived([...new Set([...(typeof location === 'undefined' ? [] : [location.origin]), ...configuredHosts().map((item) => item.origin), ...(host ? [host] : [])])]);

 $effect(() => {
  const capturedScope = scope;
  const current = ++generation;
  targets = [];
  candidates = [];
  sourceError = '';
  error = '';
  loading = true;
  void researchApi(host).targets(projectId).then((result) => {
   if (generation !== current || scope !== capturedScope) return;
   targets = result.targets ?? [];
   bindings = result.bindings ?? [];
   candidates = result.candidates ?? [];
   sourceError = result.sourceError ?? '';
  }).catch((cause) => {
   if (generation === current && scope === capturedScope) error = cause instanceof Error ? cause.message : '대상 정보를 불러오지 못했습니다.';
  }).finally(() => {
   if (generation === current && scope === capturedScope) loading = false;
  });
 });

 async function save(next: ResearchTargetBinding[]) {
  if (saving || next.length > 32) return;
  const current = generation;
  const capturedScope = scope;
  saving = true;
  error = '';
  try {
   const result = await researchApi(host).saveTargets(projectId, next);
   if (generation !== current || scope !== capturedScope) return;
   bindings = next;
   targets = result.targets ?? targets;
   onChange();
  } catch (cause) {
   if (generation === current && scope === capturedScope) error = cause instanceof Error ? cause.message : '대상 연결을 저장하지 못했습니다.';
  } finally {
   if (generation === current && scope === capturedScope) saving = false;
  }
 }

 function add() {
  if (saving || bindings.length >= 32 || !selectedOrigin || !selectedAlias) return;
  if (bindings.some((item) => item.postudioHostId === selectedOrigin || item.orxTargetAlias === selectedAlias)) return;
  void save([...bindings, { id: crypto.randomUUID(), postudioHostId: selectedOrigin, orxTargetAlias: selectedAlias, backend: 'ssh' }]);
 }
 function remove(id: string) { void save(bindings.filter((item) => item.id !== id)); }
</script>

<section class="target-bindings" aria-labelledby="target-bindings-title">
 <header><h2 id="target-bindings-title">실행 대상 연결</h2><p>SSH 실행은 저장된 소유자 승인 절차를 항상 기다립니다.</p></header>
 {#if loading}<p role="status">불러오는 중…</p>{/if}
 {#if error}<p class="error" role="alert">{error}</p>{/if}
 {#if sourceError}<p class="notice" role="status">{sourceError}</p>{/if}
 <h3>로컬 대상</h3>
 <ul aria-label="로컬 대상 목록">
  {#each targets.filter((target) => target.backend === 'local') as target (target.id)}<li>{target.label} <span>{target.available ? '사용 가능' : '사용 불가'}</span>{#if target.reason}<small>{target.reason}</small>{/if}</li>{:else}<li>로컬 대상 정보가 없습니다.</li>{/each}
 </ul>
 <h3>저장된 SSH 연결</h3>
 <ul aria-label="저장된 SSH 연결 목록">
  {#each bindings as binding (binding.id)}
   {@const target = targets.find((item) => item.id === binding.id)}
   <li><span>{binding.postudioHostId} · {binding.orxTargetAlias}</span><span>{target ? target.available ? '사용 가능' : '사용 불가' : '실제 대상 상태 확인 불가'}</span><button type="button" disabled={saving} aria-label={`${binding.postudioHostId} ${binding.orxTargetAlias} 연결 제거`} onclick={() => remove(binding.id)}>제거</button></li>
  {:else}<li>저장된 SSH 연결이 없습니다.</li>{/each}
 </ul>
 {#if candidates.length === 0}
  <p class="notice">SSH 별칭이 없습니다. orx SSH 설정이 필요합니다.</p>
 {:else}
  <div class="form">
   <label for="binding-origin">Postudio 호스트</label>
   <select id="binding-origin" bind:value={selectedOrigin} disabled={saving || bindings.length >= 32}><option value="">호스트 선택</option>{#each origins as origin}<option value={origin}>{origin}</option>{/each}</select>
   <label for="binding-alias">orx SSH 별칭</label>
   <select id="binding-alias" bind:value={selectedAlias} disabled={saving || bindings.length >= 32}><option value="">별칭 선택</option>{#each candidates as candidate (candidate.alias)}<option value={candidate.alias}>{candidate.label}</option>{/each}</select>
   <button type="button" disabled={saving || loading || bindings.length >= 32 || !selectedOrigin || !selectedAlias || bindings.some((item) => item.postudioHostId === selectedOrigin || item.orxTargetAlias === selectedAlias)} onclick={add}>{saving ? '저장 중…' : '연결 저장'}</button>
  </div>
 {/if}
</section>

<style>
 .target-bindings{min-width:0;background:var(--bg,#fff);border:1px solid var(--line-soft,#ececef);border-radius:var(--radius-card,16px);padding:16px 18px;color:var(--text,#1a1a1c)}header h2{margin:0;font-size:15px}header p{margin:5px 0 14px;color:var(--text-4,#66666b);font-size:12px}.target-bindings h3{font-size:13px;margin:16px 0 7px}.target-bindings ul{list-style:none;padding:0;margin:0}.target-bindings li{display:flex;align-items:center;gap:10px;flex-wrap:wrap;padding:9px 0;border-top:1px solid var(--line-faint,#f0f0f2);font-size:13px}.target-bindings li span:first-child{overflow-wrap:anywhere}.target-bindings li span:nth-child(2),small,.notice{color:var(--text-4,#66666b);font-size:12px}.target-bindings li button{margin-left:auto}.form{display:grid;grid-template-columns:minmax(100px,auto) minmax(0,1fr);align-items:center;gap:9px;margin-top:12px}.form label{font-size:12px}.form select,.form button,.target-bindings li button{min-height:34px;border:1px solid var(--line-strong,#dcdce0);border-radius:8px;padding:0 10px;background:var(--bg,#fff);color:var(--text-2,#3a3a3e);font:12px var(--font-sans,system-ui)}.form button{grid-column:2;justify-self:start;cursor:pointer}.form button:disabled,.target-bindings button:disabled{opacity:.6;cursor:wait}.error{color:var(--red-text,#a52f22);font-size:12px}.notice{font-size:12px}.target-bindings :global(button:focus-visible),.target-bindings :global(select:focus-visible){outline:2px solid var(--blue,#2b59c3);outline-offset:2px}@media(max-width:560px){.target-bindings{padding:14px}.form{grid-template-columns:1fr}.form button{grid-column:1}}
</style>
