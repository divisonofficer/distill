<script lang="ts">
let { variant = 'chat' }: { variant?: 'chat' | 'work' } = $props();
const statusLabel = $derived(variant === 'chat' ? '채팅 기록을 불러오는 중입니다' : '작업 기록을 불러오는 중입니다');
</script>

<div role="status" aria-label={statusLabel} class={['flex w-full flex-col', variant === 'chat' ? 'gap-[30px]' : 'gap-2.5']}>
<span class="sr-only">{statusLabel}</span>
{#if variant === 'chat'}
<div class="skeleton-bubble h-[46px] w-[44%] self-end rounded-2xl"></div>
<div class="flex w-full max-w-2xl flex-col gap-3">
<div class="flex items-center gap-3"><span class="skeleton-circle size-[30px] shrink-0"></span><span class="skeleton-line h-[11px] w-[120px]"></span></div>
<div class="skeleton-line h-[14px] w-[94%]"></div><div class="skeleton-line h-[14px] w-[88%]"></div><div class="skeleton-line h-[14px] w-[62%]"></div>
</div>
<div class="skeleton-bubble h-[46px] w-[30%] self-end rounded-2xl"></div>
<div class="flex w-full max-w-2xl flex-col gap-3"><div class="flex items-center gap-3"><span class="skeleton-circle size-[30px] shrink-0"></span><span class="skeleton-line h-[11px] w-[120px]"></span></div><div class="skeleton-line h-[14px] w-[90%]"></div><div class="skeleton-block h-[76px] w-full rounded-xl"></div><div class="skeleton-line h-[14px] w-[48%]"></div></div>
{:else}
<div class="skeleton-bubble h-[46px] w-[52%] self-end rounded-2xl"></div>
<div class="skeleton-pill h-7 w-[140px] rounded-full"></div>
<div class="skeleton-row h-11 w-full rounded-lg"></div><div class="skeleton-row h-11 w-full rounded-lg"></div>
<div class="skeleton-output h-24 w-full rounded-xl"></div>
<div class="skeleton-line h-[14px] w-[92%]"></div><div class="skeleton-line h-[14px] w-[70%]"></div>
<div class="skeleton-row h-11 w-full rounded-lg"></div>
{/if}
</div>

<style>
.skeleton-line, .skeleton-bubble, .skeleton-circle, .skeleton-block, .skeleton-pill, .skeleton-row, .skeleton-output { background: linear-gradient(90deg, #ececef 25%, #f7f7f9 50%, #ececef 75%); background-size: 200% 100%; animation: shimmer 1.3s ease-in-out infinite; }
.skeleton-circle { border-radius: 9999px; }
.skeleton-line { border-radius: 9999px; }
.skeleton-block, .skeleton-output { opacity: 0.75; }
@keyframes shimmer { to { background-position: -200% 0; } }
@media (prefers-reduced-motion: reduce) { .skeleton-line, .skeleton-bubble, .skeleton-circle, .skeleton-block, .skeleton-pill, .skeleton-row, .skeleton-output { animation: none; } }
</style>
