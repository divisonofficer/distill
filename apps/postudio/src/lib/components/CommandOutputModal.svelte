<script lang="ts">
import type { CommandExecution } from '$lib/executions.svelte';
import { commandStatus } from '$lib/command-status';
import type { ToolItem } from '$lib/session.svelte';
let { task = null, history = null, fresh = false, now, onclose, onstop, stopping = false, error = '' }: { task?: CommandExecution | null; history?: ToolItem | null; fresh?: boolean; now: number; onclose: () => void; onstop?: (id: string) => void; stopping?: boolean; error?: string } = $props();
let dialog: HTMLDialogElement;
let previous: HTMLElement | null = null;
let copyStatus = $state('');
const input = $derived(history?.input && typeof history.input === 'object' && !Array.isArray(history.input) ? history.input as Record<string, unknown> : null);
const command = $derived.by(() => {
 const value = task?.command ?? (typeof history?.input === 'string' ? history.input : typeof input?.command === 'string' ? input.command : typeof input?.cmd === 'string' ? input.cmd : history?.title);
 return typeof value === 'string' && value.trim() ? value : null;
});
const cwd = $derived.by(() => {
 const value = task?.cwd ?? (typeof input?.cwd === 'string' ? input.cwd : null);
 return typeof value === 'string' && value.trim() ? value : null;
});
const output = $derived(task?.output ?? history?.content?.filter((item) => item.type === 'text').map((item) => item.text).join('\n') ?? '');
const duration = $derived(task ? task.completed ? task.endedAtUnixMs === null ? null : Math.max(0, task.endedAtUnixMs - task.startedAtUnixMs) : fresh ? Math.max(0, now - task.startedAtUnixMs) : null : null);
const taskState = $derived(task ? commandStatus(task, fresh).label : history?.status === 'in_progress' ? '실행 중' : history?.status === 'pending' ? '대기' : '알 수 없음');
function close() { if (dialog.open) dialog.close(); }
async function copy() { try { await navigator.clipboard.writeText(output); copyStatus = '출력을 복사했어요.'; } catch { copyStatus = '클립보드를 사용할 수 없어요.'; } }
$effect(() => {
 if ((task || history) && !dialog.open) { previous = document.activeElement as HTMLElement; copyStatus = ''; dialog.showModal(); }
 else if (!task && !history && dialog.open) dialog.close();
});
</script>
<dialog bind:this={dialog} aria-label="명령 출력" onclose={() => { onclose(); if (previous?.isConnected) previous.focus(); }} onclick={(event) => { if (event.target === dialog) close(); }}>
 {#if task || history}<section><header><h2>명령 출력</h2><button type="button" aria-label="닫기" onclick={close}>×</button></header><dl>
  {#if command}<dt>명령</dt><dd>{command}</dd>{/if}
  {#if cwd}<dt>작업 디렉터리</dt><dd>{cwd}</dd>{/if}
  {#if task}<dt>상태</dt><dd><span class:unknown={taskState === '알 수 없음'} class:failed={taskState === '실패'} class:running={taskState === '실행 중'} class="state">{taskState}</span></dd>
   <dt>시작</dt><dd><time>{new Date(task.startedAtUnixMs).toLocaleString('ko-KR')}</time></dd>
   {#if duration !== null}<dt>실행 시간</dt><dd>{Math.floor(duration / 1000)}초</dd>{/if}
   {#if task.completed}<dt>종료 코드</dt><dd>{task.exitCode ?? '—'}{task.signal ? ` · 신호 ${task.signal}` : ''}</dd>{/if}
   {#if task.truncated}<dt>출력 제한</dt><dd>출력이 잘렸어요.{task.outputFile ? ` · 출력 파일: ${task.outputFile}` : ''}</dd>{/if}
  {:else}<dt>기록</dt><dd>읽기 전용 도구 기록</dd>{#if taskState}<dt>상태</dt><dd>{taskState}</dd>{/if}{/if}
 </dl>
 {#if output}<pre>{output}</pre>{:else}<p>출력이 없어요.</p>{/if}
 {#if task && !task.completed && !task.canStop}<p>이 서버는 안전한 개별 중지를 지원하지 않아요.</p>{/if}
 <footer>{#if output}<button type="button" onclick={copy}>출력 복사</button>{/if}{#if task && !task.completed && task.canStop === true && onstop}<button type="button" disabled={!fresh || stopping} onclick={() => onstop?.(task!.taskId)}>{stopping ? '중지 요청 중…' : '중지'}</button>{/if}<span role="status">{copyStatus}</span></footer>
 {#if error}<p role="alert">중지 요청 결과: {error}</p>{/if}</section>{/if}
</dialog>
<style>
 dialog{box-sizing:border-box;margin:auto;width:min(760px,calc(100vw - 24px));max-height:calc(100dvh - 24px);overflow:auto;border:1px solid #ddd;border-radius:12px;padding:20px;background:white;color:#171717}
 dialog::backdrop{background:#0007}
 button{min-height:36px;padding:0 12px;border:1px solid #ddd;border-radius:8px;background:transparent;color:inherit}
 button:disabled{opacity:.5}
 button:focus-visible{outline:2px solid #2b59c3;outline-offset:2px}
 header,footer{display:flex;align-items:center;gap:8px;justify-content:space-between;flex-wrap:wrap}
 h2{margin:0 0 16px}
 dl{display:grid;grid-template-columns:130px 1fr;gap:8px}
 dt{font-weight:600}
 dd{min-width:0;margin:0;overflow-wrap:anywhere}
 .state{display:inline-flex;align-items:center;min-height:24px;border:1px solid #ddd;border-radius:999px;padding:1px 8px;font-size:12px}
 .state.running{border-color:#93c5fd;background:#eff6ff;color:#1d4ed8}
 .state.failed{border-color:#fca5a5;background:#fef2f2;color:#b91c1c}
 .state.unknown{color:#666}
 pre{white-space:pre-wrap;overflow-wrap:anywhere;max-height:45dvh;overflow:auto;background:#141417;color:#e6e6ea;padding:12px;border-radius:6px}
 footer{justify-content:flex-start}
 @media(max-width:480px){dialog{padding:14px}dl{grid-template-columns:1fr;gap:3px}dd{margin-bottom:8px}}
 @media(prefers-color-scheme:dark){dialog{background:#171717;color:#eee;border-color:#444}button{border-color:#444}pre{background:#262626}.state.running{border-color:#1d4ed8;background:#172554;color:#bfdbfe}.state.failed{border-color:#b91c1c;background:#450a0a;color:#fecaca}.state.unknown{color:#bbb}}
</style>
