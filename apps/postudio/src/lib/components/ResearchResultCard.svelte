<script lang="ts">
  import { onDestroy } from 'svelte';
  import { researchApi, type ResearchApproval, type ResearchRun } from '$lib/research';

  let { host, projectId, marker }: { host: string; projectId: string; marker: { projectId: string; experimentId: string; approvalId?: string; runId?: string; title?: string; kind?: string } } = $props();
  let approval = $state<ResearchApproval | null>(null);
  let run = $state<ResearchRun | null>(null);
  let error = $state('');
  let busy = $state(false);
  let generation = 0;
  let timer: ReturnType<typeof setInterval> | undefined;
  let observer: (() => void) | undefined;
  const scoped = $derived(marker.projectId === projectId);
  const phase = $derived(run?.status ?? approval?.status ?? 'unknown');
  const phaseLabel = $derived(({ pending: '승인 대기', approved: '승인됨', rejected: '거부됨', submitted: '제출됨', queued: '대기 중', running: '실행 중', done: '완료', completed: '완료', failed: '실패', cancelled:'취소됨',canceled:'취소됨',submitting:'대기',starting:'대기', lost: '상태 확인 필요', unknown: '상태 확인 중' } as Record<string, string>)[String(phase)] ?? '상태 확인 필요');
  const terminal = $derived(['rejected', 'done', 'completed', 'failed', 'lost', 'cancelled'].includes(String(phase)));

  function stopPolling() {
    if (timer !== undefined) clearInterval(timer);
    timer = undefined;
    observer?.();
    observer = undefined;
  }
  async function refresh(token = generation) {
    if (!scoped || !host || token !== generation || busy) return;
    busy = true;
    error = '';
    try {
      const api = researchApi(host);
      let nextApproval: ResearchApproval | null = null;
      let nextRun: ResearchRun | null = null;
      if (marker.approvalId) {
        const result = await api.approval(projectId, marker.approvalId) as { approval: ResearchApproval; run?: ResearchRun };
        nextApproval = result.approval;
        nextRun = result.run ?? null;
      }
      if (!nextRun && marker.runId) nextRun = (await api.runs(projectId)).find((candidate) => candidate.id === marker.runId) ?? null;
      if (token !== generation) return;
      approval = nextApproval;
      run = nextRun;
    } catch (cause) {
      if (token === generation) error = cause instanceof Error ? cause.message : '실행 상태를 확인하지 못했습니다.';
    } finally {
      if (token === generation) busy = false;
    }
  }
  async function decide(approved: boolean) {
    if (!marker.approvalId || approval?.status !== 'pending' || busy) return;
    const token=generation;const p=projectId,h=host,a=marker.approvalId;
    busy = true;
    error = '';
    try {
      const result = await researchApi(h).approve(projectId, marker.approvalId, approved) as { approval: ResearchApproval; run?: ResearchRun };
      if(token!==generation)return;
      approval = result.approval;
      run = result.run ?? null;
    } catch (cause) { if(token===generation)error = cause instanceof Error ? cause.message : '승인 요청을 처리하지 못했습니다.'; }
    finally { if(token===generation)busy = false; }
  }
  $effect(() => {
    const key = `${host}\u0000${projectId}\u0000${marker.projectId}\u0000${marker.experimentId}\u0000${marker.approvalId ?? ''}\u0000${marker.runId ?? ''}`;
    const token = ++generation;
    approval = null; run = null; error = '';busy=false;
    stopPolling();
    if (!scoped || !host) return;
    void refresh(token);
    const poll = () => { if (document.visibilityState === 'visible') void refresh(token); };
    timer = setInterval(poll, 5000);
    document.addEventListener('visibilitychange', poll);
    observer = () => document.removeEventListener('visibilitychange', poll);
    return () => { generation++; stopPolling(); };
  });
  onDestroy(stopPolling);
</script>

{#if scoped}
  <section class="research-result" aria-label="Postudio 실험 상태">
    <strong>{marker.title ?? 'Postudio 실험'}</strong><span>{phaseLabel}</span>
    {#if run?.contact?.type}<span>{run.contact.type}</span>{/if}
    {#if approval?.error}<p role="alert">{approval.error}</p>{/if}
    {#if error}<p role="alert">{error}</p>{/if}
    {#if marker.approvalId && approval?.status === 'pending'}<span>커밋 {approval.expectedCommit.slice(0, 12)} · 대상 {approval.targetId}</span><button type="button" onclick={() => decide(true)} disabled={busy}>승인 및 실행</button><button type="button" onclick={() => decide(false)} disabled={busy}>거부</button>{/if}
    <a href={`/research/${encodeURIComponent(projectId)}?tab=experiments&experiment=${encodeURIComponent(marker.experimentId)}${(run?.id ?? marker.runId) ? `&run=${encodeURIComponent(run?.id ?? marker.runId!)}` : ''}&host=${encodeURIComponent(host)}`}>실험 보기</a>
    <button type="button" onclick={() => refresh()} disabled={busy}>{busy ? '확인 중' : '상태 새로고침'}</button>
  </section>
{/if}

<style>
  .research-result { display: flex; flex-wrap: wrap; align-items: center; gap: .5rem; padding: .75rem; border: 1px solid #e6e6e8; border-radius: 12px; color: #1a1a1c; }
  .research-result span { color: #55555a; }
  .research-result p { flex-basis: 100%; margin: 0; color: #a52f22; }
  .research-result a { color: #1f48a3; }
  .research-result button { border: 1px solid #dcdce0; border-radius: 8px; padding: .35rem .65rem; background: white; }
</style>
