<script lang="ts">
import { ui } from '$lib/ui.svelte';
import Icon from '$lib/components/Icon.svelte';
</script>

{#if ui.snackbar}
<div class="snackbar" role={ui.snackbar.tone === 'error' ? 'alert' : 'status'} aria-live={ui.snackbar.tone === 'error' ? 'assertive' : 'polite'}>
 {#if ui.snackbar.tone === 'error'}<svg aria-hidden="true" class="state-icon warning" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 9v4m0 4h.01M10.3 3.9 1.8 18a2 2 0 0 0 1.7 3h17a2 2 0 0 0 1.7-3L13.7 3.9a2 2 0 0 0-3.4 0Z" /></svg>{:else}<Icon name="check" class="state-icon success" />{/if}
 <span class="message">{ui.snackbar.message}</span>
 {#if ui.snackbar.action}<button type="button" class="action" onclick={() => ui.snackbar?.action?.callback()}>{ui.snackbar.action.label}</button>{/if}
 <button type="button" class="close" aria-label="알림 닫기" onclick={() => ui.dismissSnackbar()}><Icon name="close" class="size-3" /></button>
</div>
{/if}

<style>
.snackbar{position:fixed;z-index:100;left:50%;bottom:calc(100px + env(safe-area-inset-bottom));transform:translateX(-50%);box-sizing:border-box;width:max-content;max-width:min(560px,calc(100vw - 32px));display:flex;align-items:center;gap:12px;padding:10px 8px 10px 14px;border-radius:12px;background:#1f1f23;color:#fff;box-shadow:0 10px 30px #0004;font-size:13.5px;animation:appear .2s ease-out}.message{min-width:0;overflow-wrap:anywhere}.snackbar :global(.state-icon){width:17px;height:17px;flex:none}.warning{color:#f5b14c}.snackbar :global(.success){color:#5fd08a}.action,.close{flex:none}.action{min-height:30px;padding:0 8px;color:#9cbcff;font-weight:600}.close{display:flex;width:30px;height:30px;align-items:center;justify-content:center;border-radius:8px;color:#b4b4ba}.close:hover{background:#ffffff15}.action:focus-visible,.close:focus-visible{outline:2px solid #9cbcff;outline-offset:2px}
@keyframes appear{from{opacity:0;translate:0 6px}to{opacity:1;translate:0 0}}
@media(prefers-reduced-motion:reduce){.snackbar{animation:none}}
</style>
