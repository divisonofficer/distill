<script lang="ts">
import type { CompactionState } from '$lib/session.svelte';
let { state }: { state: CompactionState | null } = $props();
const duration = (ms: number) => ms < 1000 ? `${ms} ms` : `${(ms / 1000).toFixed(1)} s`;
</script>
{#if state}
<div role="status" class="flex items-start gap-2 min-w-0 break-words rounded-lg border border-sky-200 bg-sky-50 px-3 py-2 text-sm text-sky-950 dark:border-sky-900 dark:bg-sky-950/40 dark:text-sky-100">
{#if state.status === 'running'}<span class="mt-0.5 inline-block size-4 animate-spin rounded-full border-2 border-sky-700 border-r-transparent" aria-label="압축 중"></span><span>대화 압축 중 · {state.tokensUsed.toLocaleString()} 토큰 사용 · 예상 절감량은 아직 알 수 없어요.</span>
{:else if state.status === 'completed'}<span>대화 압축 완료 · {#if state.tokensBefore !== undefined}{state.tokensBefore.toLocaleString()} → {/if}{state.tokensAfter.toLocaleString()} 토큰{#if state.elapsedMs != null} · {duration(state.elapsedMs)}{/if}</span>
{:else if state.status === 'failed'}<span class="text-red-700 dark:text-red-300">대화 압축 실패 · {state.error}</span>
{:else if state.status === 'cancelled'}<span>대화 압축 취소 · {state.reason}</span>
{:else}<span>압축 결과를 확인할 수 없습니다. 연결 상태와 작업 오류를 확인하세요.</span>{/if}
</div>
{/if}
