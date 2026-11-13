<script lang="ts">
import { tick } from 'svelte';
import { getJson } from '$lib/api';
import { parseRoutingConfig } from '$lib/components/AccountRouting.svelte';
import ModelSelect from '$lib/components/ModelSelect.svelte';
import { isAutoModel } from '$lib/models';
import type { RoutingConfig } from '$lib/components/AccountRouting.svelte';
type Account = { id: string; label: string; provider: string };
let { id, host, value, onselect, disabled = false }: { id: string; host: string; value: string; onselect: (modelId: string) => void; disabled?: boolean } = $props();
let open = $state(false);
let root = $state<HTMLDivElement>();
let popup = $state<HTMLDivElement>();
let trigger = $state<HTMLButtonElement>();
let config = $state<RoutingConfig | null>(null);
let accounts = $state<Account[]>([]);
let error = $state('');
let generation = 0;
const automatic = $derived(isAutoModel({ modelId: value, name: value }));
const label = $derived(automatic && config?.enabled ? `Auto · ${config.options.length}개 경로` : value.split('/').at(-1) || 'Auto');
const strategyLabel = $derived(config?.strategy === 'ordered_first' ? '순서대로 시도' : config?.strategy === 'highest_remaining' ? '남은 사용량이 높은 계정 우선' : '확인 중');
$effect(() => { void host; generation++; open = false; config = null; accounts = []; error = ''; return () => { generation++; }; });
async function load() {
 const currentHost = host;
 const request = ++generation;
 config = null; accounts = []; error = '';
 try {
  const [routingResponse, accountResponse] = await Promise.all([getJson<unknown>('/api/accounts/routing', currentHost), getJson<unknown>('/api/accounts', currentHost)]);
  if (request !== generation) return;
  const routing = parseRoutingConfig(routingResponse);
  if (!Array.isArray(accountResponse)) throw new Error('계정 정보를 확인할 수 없습니다.');
  const validated = accountResponse.map((item): Account => {
   if (!item || typeof item !== 'object') throw new Error('계정 정보를 확인할 수 없습니다.');
   const account = item as Record<string, unknown>;
   if (typeof account.id !== 'string' || typeof account.label !== 'string' || typeof account.provider !== 'string') throw new Error('계정 정보를 확인할 수 없습니다.');
   return { id: account.id, label: account.label, provider: account.provider };
  });
  config = routing; accounts = validated;
 } catch (cause) { if (request === generation) error = cause instanceof Error ? cause.message : '라우팅 정보를 불러오지 못했습니다.'; }
}
function positionPopup() {
 if (!popup || !trigger) return;
 const rect = trigger.getBoundingClientRect();
 const width = Math.min(360, window.innerWidth - 24);
 Object.assign(popup.style, { left: `${Math.max(12, Math.min(rect.left, window.innerWidth - width - 12))}px`, right: 'auto', top: 'auto', bottom: `${window.innerHeight - rect.top + 8}px`, width: `${width}px`, maxHeight: `${Math.max(0, Math.min(window.innerHeight * 0.7, rect.top - 20))}px` });
}
function close(restoreFocus = true) {
 if (popup?.matches(':popover-open')) popup.hidePopover();
 open = false;
 if (restoreFocus) void tick().then(() => trigger?.focus());
}
async function toggle() {
 if (open) { close(); return; }
 open = true;
 void load();
 await tick();
 if (!open || !popup) return;
 popup.showPopover();
 positionPopup();
 popup.focus();
}
$effect(() => {
 if (!open || !popup) return;
 const panel = popup;
 const onResize = () => close(false);
 const onScroll = (event: Event) => { if (!(event.target instanceof Node) || !panel.contains(event.target)) close(false); };
 window.addEventListener('resize', onResize);
 window.addEventListener('scroll', onScroll, true);
 return () => { window.removeEventListener('resize', onResize); window.removeEventListener('scroll', onScroll, true); };
});
function onKeydown(event: KeyboardEvent) { if (!event.defaultPrevented && event.key === 'Escape' && open) { event.preventDefault(); close(); } }
function onDocumentClick(event: MouseEvent) { if (open && root && !event.composedPath().includes(root)) close(false); }
</script>
<svelte:window onkeydown={onKeydown} onclick={onDocumentClick} />
<div bind:this={root} class="routing-wrap">
<ModelSelect compact id={`${id}-model`} host={host} value={value} {disabled} commitOnSelect={false} onselect={onselect} />
<button bind:this={trigger} type="button" aria-label="계정 라우팅 정보" aria-haspopup="dialog" aria-expanded={open} aria-controls={id} {disabled} onclick={toggle} class="routing-info" title="계정 라우팅 정보"><span class="routing-dot" class:enabled={automatic && config?.enabled} class:known={config !== null}></span><span>라우팅</span></button>
{#if open}
<div bind:this={popup} {id} popover="manual" role="dialog" tabindex="-1" class="routing-popover" aria-label="계정 라우팅 정보">
<h3>계정 라우팅 정보</h3>
{#if error}<p role="alert">라우팅 정보를 불러오지 못했습니다. {error}</p><button type="button" onclick={() => void load()}>다시 시도</button>{:else if !config}<p role="status">라우팅 정보를 불러오는 중입니다.</p>{:else}
<p>계정 라우팅 · {config.enabled ? '사용' : '사용 안 함'}</p><p class="muted">Auto 선택 시 적용 · {strategyLabel}</p>
{#if config.options.length}<ol>{#each config.options as option (option.id)}<li><strong>{accounts.find((account) => account.id === option.account_id)?.label ?? option.account_id}</strong><span>{option.main_model}{#if option.worker_model} → {option.worker_model}{/if}</span></li>{/each}</ol>{:else}<p class="muted">라우팅 옵션 없음</p>{/if}
{/if}
<div class="footer"><a href={`${host}/settings#routing-h`}>호스트 라우팅 설정</a><button type="button" onclick={() => close()}>닫기</button></div>
</div>
{/if}
</div>
<style>
.routing-wrap{position:relative;display:flex;align-items:center;gap:2px;width:100%;min-width:0;flex:1 1 auto}.routing-info{display:flex;height:34px;min-width:34px;flex:none;align-items:center;justify-content:center;gap:4px;border:0;border-radius:8px;background:transparent;padding:0 6px;font-size:11px;color:#666}.routing-info span:not(.routing-dot){display:inline}@media(max-width:420px){.routing-info span:not(.routing-dot){display:none}}.routing-info:hover{background:#f5f5f5}.routing-dot{width:7px;height:7px;flex:none;border-radius:50%;background:#a3a3a3}.routing-dot.known{background:#737373}.routing-dot.enabled{background:#16a34a}.routing-popover{position:fixed;inset:auto;margin:0;z-index:50;overflow:auto;border:1px solid #e0e0e3;border-radius:14px;background:white;color:#1a1a1c;padding:8px;box-shadow:0 12px 32px #14141e29;font-size:13px}.routing-popover h3{padding:6px 10px;font-weight:600}.routing-popover p{margin:6px 10px;overflow-wrap:anywhere}.muted{font-size:12px;color:#66666b}.routing-popover ol{display:flex;flex-direction:column;gap:4px;margin:8px 0;padding:0;list-style:none;counter-reset:routes}.routing-popover li{position:relative;display:flex;flex-direction:column;gap:3px;border-radius:9px;background:#f6f6f7;padding:9px 10px 9px 36px;overflow-wrap:anywhere;counter-increment:routes}.routing-popover li:before{content:counter(routes);position:absolute;left:10px;top:10px;font-size:12px;color:#66666b}.routing-popover li span{font-size:12px;color:#66666b}.footer{display:flex;align-items:center;justify-content:space-between;gap:8px;border-top:1px solid #f0f0f2;margin-top:8px;padding:6px 10px 0}.routing-popover a,.routing-popover button:not(.routing-button){min-height:30px;color:#1f48a3}.routing-popover>button{margin-left:10px;text-decoration:underline}
</style>
