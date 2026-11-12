<script lang="ts">
import type { BoardRow } from '$lib/types';
import { activityBadge } from '$lib/format';
let { sessions, onopen, oncancel, canCancel }: { sessions: BoardRow[]; onopen: (id: string) => void; oncancel?: (id: string) => void; canCancel?: (id: string) => boolean } = $props();
let expanded = $state(true);
function changed(value: number) { return Number.isFinite(value) && value > 0 ? new Date(value).toLocaleString() : ''; }
</script>
{#if sessions.length}
<section class="monitor" aria-label="서브에이전트">
 <button class="heading" type="button" aria-expanded={expanded} onclick={() => expanded = !expanded}><span>서브에이전트</span><span>{sessions.length} <span class="chevron">{expanded ? '⌄' : '›'}</span></span></button>
 {#if expanded}<div class="rows">{#each sessions as session (session.sessionId)}
  {@const state = activityBadge(session.activity || '')}
  <div class="row"><div class="info"><strong title={session.title || session.sessionId}>{session.title || session.sessionId}</strong><span title={session.modelId || '모델 정보 없음'}>{session.modelId || '모델 정보 없음'} · {state.label}{#if changed(session.lastChangeUnixMs)} · <time title={changed(session.lastChangeUnixMs)}>변경 {changed(session.lastChangeUnixMs)}</time>{/if}</span></div>{#if canCancel?.(session.sessionId)}<button type="button" aria-label={`${session.title || session.sessionId} 중지`} onclick={() => oncancel?.(session.sessionId)}>중지</button>{/if}<button type="button" aria-label={`${session.title || session.sessionId} 보기`} onclick={() => onopen(session.sessionId)}>보기</button></div>
 {/each}</div>{/if}
</section>
{/if}
<style>
.monitor{flex:none;max-height:40%;overflow:auto;border-bottom:1px solid #ddd;background:#fafafa}.heading{display:flex;justify-content:space-between;width:100%;height:28px;padding:0 10px;font-size:12px;font-weight:600}.chevron{padding-left:5px}.rows{max-height:160px;overflow:auto}.row{display:flex;align-items:center;gap:7px;min-height:30px;padding:3px 8px;border-top:1px solid #eee}.info{display:flex;flex:1;min-width:0;flex-direction:column;gap:1px}.info strong,.info span{overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.info strong{font-size:12px}.info span{color:#666;font-size:11px}.row button{flex:none;font-size:11px;padding:3px 7px}
@media(max-width:640px){.monitor{max-height:40vh}.rows{max-height:24vh}}
</style>
