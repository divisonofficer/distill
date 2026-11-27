<script lang="ts">
import { onDestroy, onMount, tick } from 'svelte';
import { createSpeechDictationController } from '$lib/speech-dictation.js';

let { id, active = true, disabled = false, onappend, onpreview }: { id: string; active?: boolean; disabled?: boolean; onappend: (text: string) => void; onpreview?: (text: string) => void } = $props();
let language = $state<'ko-KR' | 'en-US'>('ko-KR');
let panelOpen = $state(false);
let capture = $state({ supported: false, requested: false, listening: false, interim: '', message: '음성 인식 기능을 확인하고 있습니다…', error: false });
let controller: ReturnType<typeof createSpeechDictationController> | null = null;
let trigger: HTMLButtonElement;
let panel: HTMLDivElement;
let languageSelect: HTMLSelectElement;
let panelStyle = $state('');
const popoverId = $derived(`${id}-settings`);

function positionPanel() {
  if (!panelOpen || !trigger || !panel) return;
  const bounds = trigger.getBoundingClientRect();
  const width = Math.min(288, window.innerWidth - 32);
  const left = Math.max(16, Math.min(bounds.right - width, window.innerWidth - width - 16));
  const bottom = Math.max(16, window.innerHeight - bounds.top + 8);
  panelStyle = `left:${left}px;bottom:${bottom}px;width:${width}px`;
}

onMount(() => {
  controller = createSpeechDictationController({
    getRecognition: () => {
      const speech = window as Window & { SpeechRecognition?: new () => any; webkitSpeechRecognition?: new () => any };
      return speech.SpeechRecognition ?? speech.webkitSpeechRecognition;
    },
    secureContext: window.isSecureContext,
    onFinal: (text: string) => onappend(text),
    onState: (next: typeof capture) => {
      capture = next;
      onpreview?.(next.interim);
      if (next.error) panelOpen = true;
    }
  });
  const reposition = () => positionPanel();
  window.addEventListener('resize', reposition);
  window.addEventListener('scroll', reposition, true);
  return () => {
    window.removeEventListener('resize', reposition);
    window.removeEventListener('scroll', reposition, true);
  };
});

$effect(() => {
  if ((!active || disabled) && controller?.state.requested) {
    controller.stop(!active ? '현재 작업공간이 숨겨져 음성 입력을 중지했습니다.' : '음성 입력을 중지했습니다.');
  }
  if (panelOpen) void tick().then(positionPanel);
});

function changeLanguage(event: Event) {
  language = (event.currentTarget as HTMLSelectElement).value as 'ko-KR' | 'en-US';
  controller?.setLanguage(language);
}

function handleTriggerClick() {
  if (capture.requested) {
    controller?.finish();
    return;
  }
  panelOpen = !panelOpen;
}

function startCapture() {
  controller?.start();
}

function stopCapture() {
  controller?.finish();
}

function handleEscape(event: KeyboardEvent) {
  if (event.key !== 'Escape' || !panelOpen) return;
  event.preventDefault();
  event.stopPropagation();
  panelOpen = false;
  trigger?.focus();
}

function handleTriggerKeydown(event: KeyboardEvent) {
  if (event.key === 'ArrowDown' && panelOpen) {
    event.preventDefault();
    languageSelect?.focus();
  } else {
    handleEscape(event);
  }
}

onDestroy(() => controller?.reset());
</script>

<div class="relative flex shrink-0" aria-label="음성 입력">
  <button
    bind:this={trigger}
    type="button"
    aria-haspopup="dialog"
    aria-controls={popoverId}
    aria-expanded={panelOpen}
    aria-pressed={capture.requested}
    aria-label={capture.requested ? '음성 입력 중지' : capture.supported ? '음성 입력 설정 열기' : '음성 입력 지원 정보'}
    title={capture.requested ? '음성 입력 중지' : '음성 입력'}
    disabled={disabled || !active}
    onclick={handleTriggerClick}
    onkeydown={handleTriggerKeydown}
    class="flex size-[34px] shrink-0 items-center justify-center rounded-full border border-neutral-300 text-neutral-700 hover:bg-neutral-100 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-neutral-500 disabled:cursor-not-allowed disabled:opacity-40 dark:border-neutral-700 dark:text-neutral-200 dark:hover:bg-neutral-800"
  >
    {#if capture.requested}
      <svg aria-hidden="true" class="size-3.5" viewBox="0 0 24 24" fill="currentColor"><rect x="6.5" y="6.5" width="11" height="11" rx="1.5" /></svg>
    {:else}
      <svg aria-hidden="true" class="size-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="2.5" width="6" height="12" rx="3" /><path d="M5.5 11.5a6.5 6.5 0 0 0 13 0M12 18v3m-4 0h8" /></svg>
    {/if}
  </button>
  <div
    id={popoverId}
    role="dialog"
    aria-label="음성 입력 설정"
    aria-hidden={!panelOpen}
    hidden={!panelOpen}
    onkeydown={handleEscape}
    bind:this={panel}
    style={panelStyle}
    class="fixed z-50 max-h-[min(26rem,calc(100dvh-2rem))] overflow-y-auto rounded-[14px] border border-neutral-200 bg-white p-3.5 text-sm shadow-xl dark:border-neutral-700 dark:bg-neutral-900"
  >
    <label for={`${popoverId}-language`} class="text-xs font-medium text-neutral-600 dark:text-neutral-300">음성 인식 언어</label>
    <select bind:this={languageSelect} id={`${popoverId}-language`} aria-label="음성 인식 언어" value={language} onchange={changeLanguage} disabled={disabled || !active} class="mt-1.5 h-9 w-full rounded-lg border border-neutral-300 bg-white px-2 text-sm focus-visible:outline-2 focus-visible:outline-neutral-500 dark:border-neutral-700 dark:bg-neutral-950">
      <option value="ko-KR">한국어</option>
      <option value="en-US">English</option>
    </select>
    <p role={capture.error ? 'alert' : 'status'} aria-live={capture.error ? 'assertive' : 'polite'} class="mt-2 text-xs leading-5 text-neutral-600 dark:text-neutral-300">{capture.message}</p>
    {#if capture.interim && !onpreview}
      <p class="mt-1 break-words text-xs leading-5 text-neutral-500 dark:text-neutral-400">{capture.interim}</p>
    {/if}
    {#if capture.error}
      <button type="button" disabled={!capture.supported || disabled || !active} onclick={startCapture} class="mt-2 min-h-9 rounded-lg border border-neutral-300 px-3 text-xs font-medium hover:bg-neutral-100 focus-visible:outline-2 focus-visible:outline-neutral-500 disabled:opacity-40 dark:border-neutral-700 dark:hover:bg-neutral-800">다시 시도</button>
    {:else if capture.requested}
      <button type="button" disabled={disabled || !active} onclick={stopCapture} class="mt-2 min-h-9 rounded-lg border border-neutral-300 px-3 text-xs font-medium hover:bg-neutral-100 focus-visible:outline-2 focus-visible:outline-neutral-500 disabled:opacity-40 dark:border-neutral-700 dark:hover:bg-neutral-800">중지하고 입력하기</button>
    {:else if capture.supported}
      <button type="button" disabled={disabled || !active} onclick={startCapture} class="mt-2 min-h-9 rounded-lg bg-neutral-900 px-3 text-xs font-medium text-white hover:bg-neutral-700 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-neutral-500 disabled:opacity-40 dark:bg-neutral-100 dark:text-neutral-900">음성 입력 시작</button>
    {/if}
    <p class="mt-3 border-t border-neutral-200 pt-2 text-[11px] leading-4 text-neutral-500 dark:border-neutral-700 dark:text-neutral-400">브라우저 음성 인식 서비스가 오디오를 처리할 수 있습니다. 지원 여부와 연결 상태는 브라우저와 네트워크에 따라 다릅니다.</p>
  </div>
</div>
