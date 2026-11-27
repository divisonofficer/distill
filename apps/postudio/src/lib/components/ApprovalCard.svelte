<script lang="ts">
import Icon from '$lib/components/Icon.svelte';
import { permissionActions, permissionOptionLabel } from '$lib/permissions';
import type { ClientMsg, PermissionOption } from '$lib/types';

let { request, details, host, connected, reply, jump, retryKey = 0 }: {
 request: { id: unknown; toolCallId: string; title: string; options: PermissionOption[] };
 details: { command: string; cwd: string; readOnly: boolean; isCommand: boolean };
 host: string;
 connected: boolean;
 reply: (message: ClientMsg) => boolean;
 jump: () => void;
 retryKey?: number;
} = $props();
let sent = $state(false);
let error = $state('');
let expanded = $state(false);
let menu = $state<HTMLDetailsElement>();
$effect(() => { void retryKey; sent = false; });
const actions = $derived(permissionActions(request.options));
const longCommand = $derived(details.command.split('\n').length > 4 || details.command.length > 400);
function decide(option: PermissionOption) {
 if (!connected || sent) return;
 if (reply({ t: 'permission', id: request.id, option_id: option.id })) {
  sent = true;
  error = '';
  if (menu) menu.open = false;
 } else error = '응답을 보내지 못했어요. 연결을 확인한 뒤 다시 시도해 주세요.';
}
function shortcut(event: KeyboardEvent) {
 if (event.key === 'Escape' && menu?.open) {
  event.preventDefault(); event.stopPropagation(); menu.open = false;
  menu.querySelector('summary')?.focus();
  return;
 }
 // Never hijack a textarea, composer, menu, or a focused button's native action.
 if (event.target !== event.currentTarget || event.isComposing || event.repeat || event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return;
 const option = event.key === 'Enter' ? actions.once : event.key === 'Escape' ? actions.reject : undefined;
 if (option) { event.preventDefault(); event.stopPropagation(); decide(option); }
}
</script>

<!-- The approval group is deliberately focusable for scoped Enter/Escape shortcuts;
     nested controls keep their native keyboard behavior (see shortcut). -->
<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<div role="group" class="approval" data-tool-call-id={request.toolCallId} aria-label="실행 승인" aria-busy={sent} tabindex="0" onkeydown={shortcut}>
{#if sent}
 <div class="submitted" role="status"><Icon name="check" class="size-4 shrink-0" /><span>응답을 보냈어요 · 서버 확인 대기</span><code title={details.command}>{details.command}</code></div>
{:else}
 <div class="head"><span class="shield"><Icon name="shield" class="size-4" /></span><div class="body"><h3>{details.isCommand ? '명령 실행 승인이 필요해요' : '도구 실행 승인이 필요해요'}</h3><p>{request.title !== details.command ? request.title : '요청 내용을 확인한 뒤 실행을 허용해 주세요.'}</p></div><span class:low={details.readOnly} class="risk">{details.readOnly ? '읽기 전용' : '위험도 확인 필요'}</span></div>
 {#if details.command}<pre class:collapsed={longCommand && !expanded}>{details.command}</pre>{/if}
 {#if longCommand}<button type="button" class="expand" aria-expanded={expanded} onclick={() => expanded = !expanded}>{expanded ? '접기' : '전체 보기'}</button>{/if}
 <div class="where">{#if details.cwd}<code title={details.cwd}>{details.cwd}</code><span>·</span>{/if}<span title={host}>{host || '호스트 정보 없음'}</span><button type="button" onclick={jump}>관련 도구 호출로 이동</button></div>
 <div class="bar">
 {#if actions.once}<button type="button" class="primary" disabled={!connected} onclick={() => actions.once && decide(actions.once)}>{permissionOptionLabel(actions.once)} <kbd>↵</kbd></button>{/if}
 {#if actions.reject}<button type="button" disabled={!connected} onclick={() => actions.reject && decide(actions.reject)}>{permissionOptionLabel(actions.reject)} <kbd>Esc</kbd></button>{/if}
 {#if actions.permanent.length || actions.other.length}<details bind:this={menu} class="more"><summary>{actions.permanent.length ? '항상 적용 ▾' : '추가 선택 ▾'}</summary><div class="options" role="group" aria-label="추가 승인 선택">{#each [...actions.permanent, ...actions.other] as option (option.id)}<button type="button" disabled={!connected} onclick={() => decide(option)}>{permissionOptionLabel(option)}</button>{/each}<p>서버가 제공한 권한 범위에만 적용돼요.</p></div></details>{/if}
 </div>
 {#if !connected}<p class="notice" role="status">연결이 끊겨 응답할 수 없어요.</p>{/if}
 {#if error}<p class="notice error" role="alert">{error}</p>{/if}
{/if}
</div>

<style>
.approval{border:1px solid #f0dcb4;border-radius:14px;background:var(--bg,#fff);color:var(--text,#1a1a1c);box-shadow:0 0 0 3px #fdf6ea;transition:all .2s ease}.approval:focus-visible,button:focus-visible,summary:focus-visible{outline:2px solid #2b59c3;outline-offset:3px}.head{display:flex;align-items:flex-start;gap:10px;padding:14px 14px 10px}.shield{width:30px;height:30px;display:grid;place-items:center;flex:none;border-radius:9px;background:#fdf3e3;color:#9a5200}.body{min-width:0;flex:1}h3{font-size:14px;font-weight:600}p{font-size:12.5px;line-height:1.5;color:var(--text-3,#55555a);margin-top:2px}.risk{font-size:11px;padding:3px 8px;border-radius:11px;background:#fdf3e3;color:#8a4a00}.risk.low{background:#e6f4ec;color:#17693f}pre{margin:0 14px;padding:9px 12px;border-radius:10px;background:#141417;color:#e6e6ea;font-size:12.5px;line-height:1.6;white-space:pre-wrap;overflow-wrap:anywhere;max-height:400px;overflow:auto}pre.collapsed{max-height:98px;overflow:hidden}.expand{margin:4px 14px;font-size:12px;color:#1f48a3}.where{display:flex;flex-wrap:wrap;align-items:center;gap:6px;padding:8px 14px 0;font-size:12px;color:var(--text-4,#66666b);overflow-wrap:anywhere}.where code{max-width:100%;overflow-wrap:anywhere}.where button{color:#1f48a3;text-decoration:underline}.bar{display:flex;flex-wrap:wrap;align-items:center;gap:8px;padding:12px 14px 14px}.bar button{min-height:32px;padding:0 12px;border:1px solid #dcdce0;border-radius:8px;font-size:12.5px;font-weight:600}.bar .primary{background:#1a1a1c;color:#fff;border-color:#1a1a1c}button:disabled{opacity:.45;cursor:not-allowed}kbd{font-size:11px;margin-left:5px;opacity:.7}.more{position:relative;margin-left:auto;font-size:12px}summary{cursor:pointer;min-height:32px;align-content:center}.options{position:absolute;z-index:5;right:0;bottom:100%;width:min(280px,70vw);padding:6px;border:1px solid #e6e6e8;border-radius:12px;background:var(--bg,#fff);box-shadow:0 12px 32px #14141e29}.options button{width:100%;text-align:left;border:0;padding:8px 10px}.options button:hover{background:var(--bg-hover,#f1f1f3)}.options p{font-size:11.5px;padding:6px 10px}.notice{padding:0 14px 12px}.error{color:#a52f22}.submitted{display:flex;flex-wrap:wrap;align-items:center;gap:8px;padding:10px;font-size:13px}.submitted code{min-width:0;flex:1;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;font-size:12px;color:var(--text-4,#66666b)}
:global(.dark) .approval{--bg:#171717;--text:#f5f5f5;--text-3:#c4c4ca;--text-4:#a3a3a3;--bg-hover:#262626;border-color:#735b35;box-shadow:0 0 0 3px #30291e}:global(.dark) .where button,:global(.dark) .expand{color:#a9c2ff}
@media(prefers-reduced-motion:reduce){.approval{transition:none}}
</style>
