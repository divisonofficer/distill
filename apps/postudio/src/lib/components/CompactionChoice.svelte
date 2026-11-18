<script lang="ts">
import { tick } from 'svelte';
import Icon from '$lib/components/Icon.svelte';
import type { ClientMsg, InteractionRequest } from '$lib/types';

let { request, connected, reply, retryKey = 0 }: {
 request: Extract<InteractionRequest, { t: 'question.request' }>;
 connected: boolean;
 reply: (msg: ClientMsg) => boolean;
 retryKey?: number;
} = $props();
const titles = ['압축하고 계속', '전체 기록 그대로 보내기', '다시 묻지 않기'];
const descriptions = ['기록을 요약한 뒤 답합니다. 다음 요청부터 요약본을 읽습니다.', '이번 요청은 전체 기록을 보냅니다.', '전체 기록을 보내고 앞으로 묻지 않습니다. /compact는 계속 사용할 수 있습니다.'];
let selected = $state(0);
let answer = $state('');
let sent = $state(false);
let root: HTMLDivElement;
let customInput: HTMLInputElement | undefined = $state();
const question = $derived(request.questions[0]);
const options = $derived(question?.options ?? []);
const canSubmit = $derived(connected && !sent && (selected === options.length ? !!answer.trim() : !!options[selected]));
$effect(() => { void retryKey; sent = false; });

function cancel() {
 if (!connected || sent) return;
 sent = reply({ t: 'question_reply', id: request.id, session_id: request.session_id, tool_call_id: request.tool_call_id, outcome: 'cancelled' });
}
function submit() {
 if (!canSubmit || !question) return;
 const label = selected === options.length ? answer.trim() : options[selected].label;
 sent = reply({ t: 'question_reply', id: request.id, session_id: request.session_id, tool_call_id: request.tool_call_id, outcome: 'accepted', answers: { [question.question]: [label] } });
}
async function choose(index: number, focus = false) {
 if (!connected || sent) return;
 selected = index;
 await tick();
 if (index === options.length) customInput?.focus();
 else if (focus) root.querySelectorAll<HTMLButtonElement>('[role="radio"]')[index]?.focus();
}
function keydown(event: KeyboardEvent) {
 if (event.isComposing || event.defaultPrevented || !connected || sent) return;
 if (event.target instanceof HTMLInputElement || event.target instanceof HTMLTextAreaElement) return;
 if (event.key === 'Escape') { event.preventDefault(); cancel(); }
 else if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
  event.preventDefault();
  void choose((selected + (event.key === 'ArrowDown' ? 1 : options.length)) % (options.length + 1), true);
 } else if (/^[1-4]$/.test(event.key) && Number(event.key) <= options.length + 1) {
  event.preventDefault(); void choose(Number(event.key) - 1, true);
 } else if (event.key === 'Enter' && (event.target as HTMLElement).getAttribute('role') === 'radio') {
  event.preventDefault(); submit();
 }
}
</script>

<div bind:this={root} role="dialog" aria-modal="false" aria-label="대화 압축 선택" tabindex="-1" onkeydown={keydown} class="compaction-choice rounded-2xl border border-sky-200 bg-white p-3 text-neutral-800 shadow-sm dark:border-sky-900 dark:bg-neutral-900 dark:text-neutral-100">
 <h2 class="flex items-center gap-2 text-sm font-semibold"><span class="flex size-8 shrink-0 items-center justify-center rounded-lg bg-sky-100 text-sky-700 dark:bg-sky-950 dark:text-sky-300" aria-hidden="true"><Icon name="document" /></span>먼저 대화를 압축할까요?</h2>
 <p class="mt-2 text-xs leading-5 text-neutral-600 dark:text-neutral-300">쉬는 동안 프롬프트 캐시가 만료됐습니다. 그대로 보내면 전체 대화 기록을 다시 전송합니다. 압축 후 토큰 수는 완료 뒤 확인할 수 있습니다.</p>
 <div class="mt-3 flex flex-col gap-1" role="radiogroup" aria-label="압축 선택">
  {#each options as option, index (option.label)}
   <button type="button" role="radio" aria-checked={selected === index} tabindex={selected === index ? 0 : -1} disabled={!connected || sent} class="flex min-h-10 items-center gap-2 rounded-xl border px-3 py-2 text-left disabled:opacity-50" class:border-sky-500={selected === index} class:bg-sky-50={selected === index} onclick={() => void choose(index)}>
    <span aria-hidden="true" class="shrink-0 text-xs">{index + 1}</span><span class="min-w-0"><span class="block text-sm font-semibold">{titles[index] ?? option.label}</span><span class="block text-xs leading-5 text-neutral-500">{descriptions[index] ?? option.description}</span></span>
   </button>
  {/each}
  <button type="button" role="radio" aria-checked={selected === options.length} tabindex={selected === options.length ? 0 : -1} disabled={!connected || sent} class="min-h-10 rounded-xl border px-3 text-left text-sm disabled:opacity-50" class:border-sky-500={selected === options.length} class:bg-sky-50={selected === options.length} onclick={() => void choose(options.length)}>{options.length + 1}. 직접 답변 입력</button>
 </div>
 {#if selected === options.length}<input bind:this={customInput} bind:value={answer} disabled={!connected || sent} aria-label="직접 답변" class="mt-2 min-h-10 w-full rounded-lg border border-neutral-300 bg-transparent px-3 text-sm dark:border-neutral-700" />{/if}
 {#if sent}<p role="status" class="mt-2 text-xs">선택을 보냈습니다. 서버 확인 중입니다.</p>{/if}
 {#if !connected}<p role="status" class="mt-2 text-xs">연결이 끊겨 선택을 보낼 수 없습니다.</p>{/if}
 <div class="mt-3 flex flex-wrap items-center justify-end gap-2"><span class="mr-auto text-xs text-neutral-500">선택에 포커스: ↑↓ · 1–4 · Enter · Esc</span><button type="button" disabled={!connected || sent} class="min-h-9 rounded-lg border px-3 text-sm disabled:opacity-40" onclick={cancel}>취소</button><button type="button" disabled={!canSubmit} class="min-h-9 rounded-lg bg-neutral-900 px-3 text-sm font-semibold text-white disabled:opacity-40 dark:bg-neutral-100 dark:text-neutral-900" onclick={submit}>{selected === options.length ? '직접 답변 보내기' : selected === 0 ? '압축하고 계속' : selected === 1 ? '전체 대화로 계속' : '다시 묻지 않기'}</button></div>
</div>
<style>
.compaction-choice{max-height: min(60vh, 520px); overflow:auto; flex:none;}
@media(prefers-reduced-motion:reduce){.compaction-choice{scroll-behavior:auto}}
</style>
