<script lang="ts">
import { childExecutionState, type AgentExecution } from '$lib/executions.svelte';
let { agent, fresh = false, blocked = false, now, stopping = false, error = '', onclose, onstop }: { agent: AgentExecution | null; fresh?: boolean; blocked?: boolean; now: number; stopping?: boolean; error?: string; onclose: () => void; onstop?: (id: string) => void } = $props();
let dialog: HTMLDialogElement;
let previous: HTMLElement | null = null;
const live = $derived(agent !== null && ['running', 'initializing'].includes(agent.status));
const duration = $derived(agent ? live ? fresh ? Math.max(agent.durationMs, now - agent.startedAtEpochMs) : null : agent.durationMs : null);
const state = $derived(agent ? ({ running: '실행 중', blocked: '확인 필요', done: '완료', failed: '실패', stopped: '중지됨', unknown: '상태 확인 중' } as const)[childExecutionState(agent.status, fresh, blocked)] : '');
function close() { if (dialog.open) dialog.close(); }
$effect(() => {
 if (agent && !dialog.open) { previous = document.activeElement as HTMLElement; dialog.showModal(); }
 else if (!agent && dialog.open) dialog.close();
});
</script>
<dialog bind:this={dialog} aria-label="서브에이전트 상세" onclose={() => { onclose(); if (previous?.isConnected) previous.focus(); }} onclick={(event) => { if (event.target === dialog) close(); }}>
 {#if agent}<section><header><h2>서브에이전트 상세</h2><button type="button" aria-label="닫기" onclick={close}>×</button></header>
 <dl>
  {#if agent.subagentType}<dt>역할</dt><dd>{agent.subagentType}</dd>{/if}
  {#if agent.description}<dt>설명</dt><dd>{agent.description}</dd>{/if}
  <dt>상태</dt><dd><span class:unknown={state === '상태 확인 중' || state === '중지됨'} class:failed={state === '실패'} class:running={state === '실행 중' || state === '확인 필요'} class="state">{state}</span></dd>
  <dt>시작</dt><dd><time datetime={new Date(agent.startedAtEpochMs).toISOString()}>{new Date(agent.startedAtEpochMs).toLocaleString('ko-KR')}</time></dd>
  {#if duration !== null}<dt>실행 시간</dt><dd>{Math.floor(Math.max(0, duration) / 1000)}초</dd>{/if}
  {#if agent.turnCount != null}<dt>턴</dt><dd>{agent.turnCount}</dd>{/if}
  {#if agent.toolCallCount != null}<dt>도구 호출</dt><dd>{agent.toolCallCount}</dd>{/if}
  {#if agent.tokensUsed != null}<dt>사용 토큰</dt><dd>{agent.tokensUsed}</dd>{/if}
  {#if agent.contextWindowTokens != null}<dt>컨텍스트 크기</dt><dd>{agent.contextWindowTokens}</dd>{/if}
  {#if agent.errorCount != null}<dt>오류</dt><dd>{agent.errorCount}</dd>{/if}
  {#if agent.toolsUsed?.length}<dt>사용 도구</dt><dd>{agent.toolsUsed.join(', ')}</dd>{/if}
  {#if agent.output}<dt>결과</dt><dd class="output">{agent.output}</dd>{/if}
  {#if agent.failureError}<dt>실패</dt><dd class="output">{agent.failureError}</dd>{/if}
  {#if agent.cancelReason}<dt>취소 이유</dt><dd class="output">{agent.cancelReason}</dd>{/if}
 </dl>
 {#if live && onstop}<button type="button" disabled={!fresh || stopping} onclick={() => onstop?.(agent!.subagentId)}>{stopping ? '중지 요청 중…' : '중지'}</button>{/if}
 {#if error}<p role="alert">중지 요청 결과: {error}</p>{/if}</section>{/if}
</dialog>
<style>
 dialog{box-sizing:border-box;margin:auto;width:min(600px,calc(100vw - 24px));max-height:calc(100dvh - 24px);overflow:auto;border:1px solid #ddd;border-radius:12px;padding:20px;background:white;color:#171717}
 dialog::backdrop{background:#0007}
 button{min-height:36px;padding:0 12px;border:1px solid #ddd;border-radius:8px;background:transparent;color:inherit}
 button:disabled{opacity:.5}
 button:focus-visible{outline:2px solid #2b59c3;outline-offset:2px}
 header{display:flex;align-items:center;justify-content:space-between;gap:8px}
 h2{margin:0 0 16px}
 dl{display:grid;grid-template-columns:minmax(100px,auto) 1fr;gap:10px;margin:0 0 16px}
 dt{font-weight:600}
 dd{min-width:0;margin:0;overflow-wrap:anywhere}
 .state{display:inline-flex;align-items:center;min-height:24px;border:1px solid #ddd;border-radius:999px;padding:1px 8px;font-size:12px}
 .state.running{border-color:#93c5fd;background:#eff6ff;color:#1d4ed8}
 .state.failed{border-color:#fca5a5;background:#fef2f2;color:#b91c1c}
 .state.unknown{color:#666}
 .output{white-space:pre-wrap;max-height:35dvh;overflow:auto}
 @media(max-width:480px){dialog{padding:14px}dl{grid-template-columns:1fr;gap:4px}dd{margin-bottom:10px}}
 @media(prefers-color-scheme:dark){dialog{background:#171717;color:#eee;border-color:#444}button{border-color:#444}.state.running{border-color:#1d4ed8;background:#172554;color:#bfdbfe}.state.failed{border-color:#b91c1c;background:#450a0a;color:#fecaca}.state.unknown{color:#bbb}}
</style>
