<script lang="ts">
import { childExecutionState, childNeedsInput as hasChildNeedsInput, Executions, type AgentExecution, type CommandExecution } from '$lib/executions.svelte';
import { commandStatus } from '$lib/command-status';
import type { BoardRow } from '$lib/types';
import Icon from './Icon.svelte';
import AgentDetailModal from './AgentDetailModal.svelte';
import CommandOutputModal from './CommandOutputModal.svelte';
let { host, sessionId, client: sharedClient, childSessions = [], blocked = false, active = true }: { host: string; sessionId: string; client?: Executions; childSessions?: BoardRow[]; blocked?: boolean; active?: boolean } = $props();
let client = $derived(sharedClient ?? new Executions(host, sessionId));
let selectedAgent = $state<string | null>(null);
let selectedTask = $state<string | null>(null);
let now = $state(Date.now());
let expanded = $state(false);
export function reveal() { expanded = true; }
const agent = $derived(client.agents.find(item => item.subagentId === selectedAgent) ?? null);
const task = $derived(client.tasks.find(item => item.taskId === selectedTask) ?? null);
const liveStatus = (status: string) => status === 'running' || status === 'initializing';
const childNeedsInput = (item: AgentExecution) => hasChildNeedsInput(item, childSessions, sessionId);
const agentStatus = (item: AgentExecution, fresh: boolean) => {
 const states: Record<string, { label: string; tone: string }> = {
  running: { label: '실행 중', tone: 'running' },
  blocked: { label: '확인 필요', tone: 'unknown' },
  done: { label: '완료', tone: 'completed' },
  failed: { label: '실패', tone: 'failed' },
  stopped: { label: '중지됨', tone: 'unknown' },
  unknown: { label: '상태 확인 중', tone: 'unknown' }
 };
 return states[childExecutionState(item.status, fresh, childNeedsInput(item))];
};
const agentDuration = (item: AgentExecution, fresh: boolean, timestamp: number) => {
 if (liveStatus(item.status)) return fresh ? Math.max(item.durationMs, timestamp - item.startedAtEpochMs) : null;
 return item.durationMs;
};
const commandDuration = (item: CommandExecution, fresh: boolean, timestamp: number) => {
 if (item.completed) return item.endedAtUnixMs === null ? null : Math.max(0, item.endedAtUnixMs - item.startedAtUnixMs);
 return fresh ? Math.max(0, timestamp - item.startedAtUnixMs) : null;
};
const durationText = (duration: number) => `${Math.floor(Math.max(0, duration) / 1000)}초`;
const summary = $derived([
 client.agents.length ? `서브에이전트 ${client.agents.length}개` : '',
 client.tasks.length ? `명령 ${client.tasks.length}개` : ''
].filter(Boolean).join(' · '));
$effect(() => { host; sessionId; selectedAgent = null; selectedTask = null; expanded = false; });
$effect(() => { if (!active || blocked) { selectedAgent = null; selectedTask = null; } });
$effect(() => {
 const instance = client;
 if (active && !blocked) instance.start();
 return () => instance.stop();
});
$effect(() => {
 if (!active || blocked || !expanded) return;
 const hasFreshLiveAgent = client.agents.some(item => liveStatus(item.status) && client.agentFresh[item.subagentId]);
 const hasFreshLiveTask = client.tasksFresh && client.tasks.some(item => !item.completed);
 if (!hasFreshLiveAgent && !hasFreshLiveTask) return;
 const timer = setInterval(() => now = Date.now(), 1000);
 return () => clearInterval(timer);
});
</script>
{#if client.agents.length > 0 || client.tasks.length > 0 || !client.agentLoaded || !client.taskLoaded || client.agentUnavailable || client.taskUnavailable || client.agentError || client.taskError}
<section class="monitor" aria-label="실행 현황">
 <header>
  <button type="button" aria-expanded={expanded} onclick={() => expanded = !expanded}>실행 현황 {expanded ? '접기' : '펼치기'}</button>
  {#if !client.agentLoaded || !client.taskLoaded}<span>실행 현황을 불러오고 있어요.</span>
  {:else if client.agentUnavailable && client.taskUnavailable}<span>이 서버에서는 실행 현황을 지원하지 않아요.</span>
  {:else if client.agentError || client.taskError}<span>실행 현황을 불러오지 못했어요.</span>
  {:else if client.agentUnavailable || client.taskUnavailable}<span>일부 실행 정보를 불러오지 못했어요.</span>
  {:else if summary}<span>{summary}</span>{/if}
  {#if client.agentUnavailable || client.taskUnavailable || client.agentError || client.taskError}<button type="button" disabled={!active || blocked} onclick={() => client.retry()}>다시 시도</button>{/if}
 </header>
 {#if expanded}
  {#if client.agentError || client.taskError}<details class="technical"><summary>오류 정보</summary>{#if client.agentError}<p>{client.agentError}</p>{/if}{#if client.taskError}<p>{client.taskError}</p>{/if}</details>{/if}
  {#if client.agents.length > 0}
   <section class="category" aria-label="서브에이전트"><h3>서브에이전트</h3>
    <ul>{#each client.agents as item (item.subagentId)}
     {@const fresh = active && !blocked && client.agentFresh[item.subagentId] === true}
     {@const state = agentStatus(item, fresh)}
     {@const duration = agentDuration(item, fresh, now)}
     <li class="execution agent-row">
      <span class="avatar" aria-hidden="true"><Icon name="server" class="size-4" /></span>
      <div class="identity">
       <div class="identity-head"><strong>{item.subagentType}</strong><span class="state state-{state.tone}">{state.label}</span></div>
       {#if item.description}<p>{item.description}</p>{/if}
      </div>
      {#if duration !== null}<time aria-label="실행 시간 {durationText(duration)}">{durationText(duration)}</time>{/if}
      <div class="actions">
       <button type="button" onclick={() => selectedAgent = item.subagentId}>상세 보기</button>
       {#if liveStatus(item.status)}<button type="button" disabled={!active || blocked || !fresh || client.pendingAgentStops.includes(item.subagentId)} onclick={() => client.stopAgent(item.subagentId)}>{client.pendingAgentStops.includes(item.subagentId) ? '중지 요청 중…' : '중지'}</button>{/if}
      </div>
      {#if client.agentSnapshotErrors[item.subagentId]}<details class="row-error"><summary>상세 정보를 불러오지 못했어요.</summary><small>{client.agentSnapshotErrors[item.subagentId]}</small></details>{/if}
      {#if client.agentStopErrors[item.subagentId]}<small class="row-error" role="alert">중지 요청 결과: {client.agentStopErrors[item.subagentId]}</small>{/if}
     </li>
    {/each}</ul>
   </section>
  {/if}
  {#if client.tasks.length > 0}
   <section class="category" aria-label="백그라운드 명령"><h3>백그라운드 명령</h3>
    <ul>{#each client.tasks as item (item.taskId)}
     {@const state = commandStatus(item, active && !blocked && client.tasksFresh)}
     {@const duration = commandDuration(item, active && !blocked && client.tasksFresh, now)}
     <li class="execution command-row">
      <div class="identity">
       <div class="identity-head"><strong title={item.command}>{item.description || item.command}</strong><span class="state state-{state.tone}">{state.label}</span></div>
       {#if item.description && item.description !== item.command}<p class="command">{item.command}</p>{/if}
      </div>
      {#if duration !== null}<time aria-label="실행 시간 {durationText(duration)}">{durationText(duration)}</time>{/if}
      <div class="actions">
       <button type="button" onclick={() => selectedTask = item.taskId}>출력 보기</button>
       {#if !item.completed && item.canStop === true}<button type="button" disabled={!active || blocked || !client.tasksFresh || client.pendingTaskStops.includes(item.taskId)} onclick={() => client.stopTask(item.taskId)}>{client.pendingTaskStops.includes(item.taskId) ? '중지 요청 중…' : '중지'}</button>{/if}
      </div>
      {#if client.taskStopErrors[item.taskId]}<small class="row-error" role="alert">중지 요청 결과: {client.taskStopErrors[item.taskId]}</small>{/if}
     </li>
    {/each}</ul>
   </section>
  {/if}
 {/if}
</section>
{/if}
<AgentDetailModal {agent} {now} fresh={active && !blocked && !!agent && !!client.agentFresh[agent.subagentId]} blocked={!!agent && childNeedsInput(agent)} onclose={() => selectedAgent = null} onstop={(id) => client.stopAgent(id)} stopping={!!agent && client.pendingAgentStops.includes(agent.subagentId)} error={agent ? client.agentStopErrors[agent.subagentId] : ''} />
<CommandOutputModal {task} {now} fresh={active && !blocked && client.tasksFresh} onclose={() => selectedTask = null} onstop={(id) => client.stopTask(id)} stopping={!!task && client.pendingTaskStops.includes(task.taskId)} error={task ? client.taskStopErrors[task.taskId] : ''} />
<style>
 .monitor{flex:none;padding:4px 14px;border-bottom:1px solid #ddd;max-height:min(30vh,320px);overflow:auto}
 .monitor header{display:flex;align-items:center;gap:8px;min-height:32px;flex-wrap:wrap;font-size:12px}
 .monitor header span{min-width:0;overflow-wrap:anywhere;color:#666}
 .monitor button{flex:none;min-height:32px;padding:0 9px;border:1px solid #ddd;border-radius:8px;background:transparent;color:inherit}
 .monitor button:disabled{opacity:.5}
 .monitor button:focus-visible{outline:2px solid #2b59c3;outline-offset:2px}
 .category h3{font-size:12px;margin:8px 0 4px}
 .category ul{list-style:none;padding:0;margin:0}
 .execution{display:flex;align-items:center;gap:8px;padding:6px 0;flex-wrap:wrap;border-top:1px solid #eee}
 .avatar{display:grid;place-items:center;flex:none;width:30px;height:30px;border-radius:50%;background:#f0f0f0;color:#555}
 .identity{flex:1;min-width:150px;overflow-wrap:anywhere}
 .identity-head{display:flex;align-items:center;gap:8px;min-width:0;flex-wrap:wrap}
 .identity-head strong{min-width:0;overflow-wrap:anywhere}
 .identity p{margin:2px 0 0;font-size:12px;line-height:1.45;color:#666;overflow-wrap:anywhere}
 .identity .command{font-family:ui-monospace,monospace;white-space:pre-wrap}
 .state{display:inline-flex;align-items:center;min-height:22px;flex:none;border:1px solid #ddd;border-radius:999px;padding:1px 8px;font-size:11px;line-height:1.4;white-space:nowrap}
 .state-running{border-color:#93c5fd;background:#eff6ff;color:#1d4ed8}
 .state-completed{border-color:#a3a3a3;background:#f5f5f5;color:#404040}
 .state-failed{border-color:#fca5a5;background:#fef2f2;color:#b91c1c}
 .state-unknown{border-color:#d4d4d4;background:#fafafa;color:#525252}
 .execution time{flex:none;color:#666;font-size:12px;white-space:nowrap}
 .actions{display:flex;align-items:center;gap:6px;flex:none}
 .row-error{flex-basis:100%;margin-left:38px;color:#a33;font-size:12px;overflow-wrap:anywhere}
 .technical p{overflow-wrap:anywhere}
 @media(max-width:520px){
  .monitor{padding:4px 10px;max-height:min(38dvh,360px)}
  .execution{align-items:flex-start;gap:6px}
  .identity{flex-basis:calc(100% - 40px)}
  .command-row .identity{flex-basis:100%}
  .execution time{margin-left:38px}
  .actions{margin-left:38px}
  .command-row .actions{margin-left:0}
  .row-error{margin-left:38px}
 }
 @media(prefers-color-scheme:dark){
  .monitor{border-color:#444}
  .monitor header span,.identity p,.execution time{color:#bbb}
  .execution{border-color:#333}
  .avatar{background:#262626;color:#ddd}
  .state-running{border-color:#1d4ed8;background:#172554;color:#bfdbfe}
  .state-completed{border-color:#555;background:#262626;color:#ddd}
  .state-failed{border-color:#b91c1c;background:#450a0a;color:#fecaca}
  .state-unknown{border-color:#555;background:#262626;color:#ddd}
  .monitor button{border-color:#444}
 }
</style>
