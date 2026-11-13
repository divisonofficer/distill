<script lang="ts">
import { tick, untrack } from 'svelte';
import SessionConversation from '$lib/components/SessionConversation.svelte';
import Icon from '$lib/components/Icon.svelte';
import SessionMonitor from '$lib/components/SessionMonitor.svelte';
import { directChildren, scopedChildren } from '$lib/session-monitor';
import { activityBadge } from '$lib/format';
import { ui } from '$lib/ui.svelte';
import type { BoardRow } from '$lib/types';
import WorkspaceDocument from '$lib/components/WorkspaceDocument.svelte';
import type { ArtifactEntry } from '$lib/artifact-browser';
import type { ToolItem } from '$lib/session.svelte';
import { activateTab, closeOthers, closeTab, createLayout, joinAll, moveTab, openTab, parseLayout, splitRight, tabDropIndex, type WorkbenchLayout, type WorkbenchTab } from '$lib/workbench-layout';

let { sessionId, host, workspaceId, conversations, sessionRows = [], blocked = false, onConversationSelect }: { sessionId: string; host: string; workspaceId: string; conversations: Array<{ sessionId: string; title: string }>; sessionRows?: BoardRow[]; blocked?: boolean; onConversationSelect?: (id: string) => void } = $props();
const storageKey = $derived(`postudio-workbench:${JSON.stringify([host, workspaceId])}`);
let layout = $state<WorkbenchLayout>(untrack(() => createLayout({ kind: 'conversation', id: `conversation:${sessionId}`, sessionId, title: conversations.find((item) => item.sessionId === sessionId)?.title || sessionId })));
let drafts = new Map<string, { text: string; attachments: import('$lib/attachments').DraftAttachment[] }>();
let toolCache = $state<Record<string, ToolItem[]>>({});
let pending = $state<Record<string, boolean>>({});
let controls = $state<Record<string, { connected: boolean; running: boolean; cancel: () => boolean }>>({});
let readyKey = $state('');
let previousRoute = '';
let dropMarker = $state<{ groupId: string; tabId?: string; after: boolean } | null>(null);
const conversationTabId = (id: string) => `conversation:${id}`;
const currentGroup = $derived(layout.groups.find((group) => group.id === layout.focusedGroup) ?? layout.groups[0]);
function cacheKey(session: string) { return JSON.stringify([host, session]); }
const childrenFor = (parent: string) => blocked ? [] : directChildren(sessionRows, parent, host);
const scopedChildIds = () => blocked ? [] : scopedChildren(sessionRows, [sessionId, ...conversations.map(item => item.sessionId)], host);
function verifiedChild(id: string) { return scopedChildIds().includes(id); }
function setControl(id: string, scope: string, control: { connected: boolean; running: boolean; cancel: () => boolean } | null) { if (scope !== storageKey) return; const key = JSON.stringify([scope, id]); if (control) { if (controls[key]?.connected === control.connected && controls[key]?.running === control.running) return; controls = { ...controls, [key]: control }; } else if (Object.hasOwn(controls, key)) { const next = { ...controls }; delete next[key]; controls = next; } }
function controlCallback(id: string, scope: string) { return (control: { connected: boolean; running: boolean; cancel: () => boolean } | null) => setControl(id, scope, control); }
function controlFor(id: string) { return controls[JSON.stringify([storageKey, id])]; }
function canCancel(id: string) { const control = controlFor(id); return !blocked && readyKey === storageKey && verifiedChild(id) && !!control?.connected && !!control.running; }
function cancelChild(id: string) { if (!canCancel(id)) return; const sent = controlFor(id)?.cancel() ?? false; const title = sessionRows.find(row => row.sessionId === id)?.title || '하위 세션'; ui.notify(`${title}: ${sent ? '중지 요청을 보냈습니다.' : '중지 요청을 보내지 못했습니다.'}`, sent ? 'success' : 'error'); }
function save() { if (readyKey !== storageKey || blocked) return; try { localStorage.setItem(storageKey, JSON.stringify(layout)); } catch {} }
function setLayout(next: WorkbenchLayout) {
 layout = next; save();
 const active = next.groups.find((group) => group.id === next.focusedGroup)?.active;
 const tab = active ? next.tabs[active] : undefined;
 if (tab?.kind === 'conversation' && tab.sessionId !== sessionId && !verifiedChild(tab.sessionId)) onConversationSelect?.(tab.sessionId);
}
function activate(id: string) { setLayout(activateTab(layout, id)); }
function close(id: string) { setLayout(closeTab(layout, id)); focusActiveTab(); }
function titleFor(id: string) { return conversations.find((item) => item.sessionId === id)?.title || id; }
function selectTab(id: string) { if (layout.tabs[id]) activate(id); }
function openConversation(id: string, groupId = layout.focusedGroup) { const tabId = conversationTabId(id); setLayout(openTab(layout, { kind: 'conversation', id: tabId, sessionId: id, title: titleFor(id) }, groupId)); focusActiveTab(); }
function updateTitle(id: string, title: string) { if (title === id && titleFor(id) !== id) return; const tabId = conversationTabId(id), tab = layout.tabs[tabId]; if (tab && tab.title !== title) { layout = { ...layout, tabs: { ...layout.tabs, [tabId]: { ...tab, title: title.slice(0, 512) } } }; save(); } }
function updateTools(id: string, tools: ToolItem[]) { toolCache = { ...toolCache, [cacheKey(id)]: tools }; }
function openTool(session: string, tool: ToolItem, view: 'details' | 'diffs' | 'logs' = 'details') {
 const input = tool.input as Record<string, unknown> | string | null | undefined;
 const command = typeof input === 'string' ? input : input && typeof input === 'object' ? (input.command ?? input.cmd) : undefined;
 const title = String(typeof command === 'string' && command.trim() ? command : tool.title ?? tool.name ?? tool.id).slice(0, 512);
 open({ kind: 'tool', id: `tool:${session}:${tool.id}:${view}`, sessionId: session, toolId: tool.id, view, title });
}
function open(tab: WorkbenchTab, groupId = layout.focusedGroup) { setLayout(openTab(layout, tab, groupId)); focusActiveTab(); }
function openExplorer(groupId: string) {
 const existing = Object.values(layout.tabs).find((tab) => tab.kind === 'explorer');
 open(existing ?? { kind: 'explorer', id: 'explorer', sessionId, title: '탐색기' }, groupId);
}
function openRecords(groupId: string) {
 const active = layout.groups.find((group) => group.id === groupId)?.active;
 const owner = active ? layout.tabs[active]?.sessionId ?? sessionId : sessionId;
 open({ kind: 'tool', id: `records:${owner}`, sessionId: owner, toolId: '', view: 'logs', title: '명령 실행 기록' }, groupId);
}
function openedFile(entry: ArtifactEntry, owner: string, groupId = layout.focusedGroup) { const id = `file:${entry.path}`; if (layout.tabs[id]) { activate(id); return; } open({ kind: 'file', id, sessionId: owner, path: entry.path, mime: entry.mime, title: entry.name }, groupId); }
function openPreview(port?: number, groupId = layout.focusedGroup) { open({ kind: 'preview', id: `preview:${port ?? 'available'}`, sessionId, port, title: port ? `${new URL(host).hostname}:${port}` : '포트 미리보기' }, groupId); }
const tabDragType = 'application/x-postudio-tab';
function acceptsTabDrag(event: DragEvent) { return !!event.dataTransfer && Array.from(event.dataTransfer.types).includes(tabDragType); }
function draggedTab(event: DragEvent) {
 const transfer = event.dataTransfer;
 if (!transfer || !Array.from(transfer.types).includes(tabDragType)) return null;
 const id = transfer.getData(tabDragType);
 return layout.tabs[id] ? id : null;
}
function transfer(event: DragEvent, id?: string) {
 if (id) { event.dataTransfer?.setData(tabDragType, id); if (event.dataTransfer) event.dataTransfer.effectAllowed = 'move'; return; }
 const data = draggedTab(event); if (!data) { dropMarker = null; return; }
 event.preventDefault(); const element = event.currentTarget as HTMLElement, groupId = element.dataset.group;
 if (!groupId) return;
 const target = layout.groups.find(group => group.id === groupId);
 if (!target) return;
 const index = target.tabs.length;
 if (index !== null && index !== undefined) setLayout(moveTab(layout, data, groupId, index));
 dropMarker = null;
}
function dragOverTab(event: DragEvent, groupId: string, tabId: string) {
 if (!acceptsTabDrag(event)) return;
 const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
 dropMarker = { groupId, tabId, after: event.clientX >= rect.left + rect.width / 2 };
 event.preventDefault();
}
function dropOnTab(event: DragEvent, groupId: string, tabId: string) {
 event.stopPropagation(); event.preventDefault();
 const id = draggedTab(event), target = layout.groups.find(group => group.id === groupId);
 if (!id || !target) { dropMarker = null; return; }
 const marker = dropMarker?.groupId === groupId && dropMarker.tabId === tabId ? dropMarker : { groupId, tabId, after: event.clientX >= (event.currentTarget as HTMLElement).getBoundingClientRect().left + (event.currentTarget as HTMLElement).getBoundingClientRect().width / 2 };
 const source = layout.groups.find(group => group.tabs.includes(id));
 const index = source ? tabDropIndex(source.tabs, target.tabs, id, tabId, marker.after) : null;
 if (index !== null) setLayout(moveTab(layout, id, groupId, index));
 dropMarker = null;
}
function keyTab(event: KeyboardEvent, group: WorkbenchLayout['groups'][number], id: string) {
 if (!['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) return;
 event.preventDefault(); const index = group.tabs.indexOf(id), next = event.key === 'Home' ? 0 : event.key === 'End' ? group.tabs.length - 1 : (index + (event.key === 'ArrowRight' ? 1 : group.tabs.length - 1)) % group.tabs.length;
 const nextId = group.tabs[next]; selectTab(nextId); document.getElementById(`tab-${nextId}`)?.focus();
}
function revealActive() {
 const group = layout.groups.find((item) => item.id === layout.focusedGroup);
 const active = group?.active;
 if (active) void tick().then(() => {
  if (layout.focusedGroup === group?.id) document.getElementById(`tab-${active}`)?.scrollIntoView({ block: 'nearest', inline: 'nearest' });
 });
}
$effect(revealActive);

let menu = $state<HTMLElement | null>(null);
let menuTrigger = $state<HTMLElement | null>(null);
let menuTab = $state('');
let menuGroup = $state('');
let menuKind = $state<'tab' | 'add'>('tab');
let menuOpen = $state(false);
function closeMenu(restore = true) { if (!menu) return; if (menu.matches(':popover-open')) menu.hidePopover(); menuOpen = false; if (restore && menuTrigger?.isConnected) menuTrigger.focus(); else if (restore && menuTab) document.getElementById(`tab-${menuTab}`)?.focus(); menuTrigger = null; }
function focusActiveTab() { void tick().then(() => { const active = layout.groups.find(group => group.id === layout.focusedGroup)?.active; if (active) document.getElementById(`tab-${active}`)?.focus(); }); }
function positionMenu(trigger: HTMLElement) {
 const rect = trigger.getBoundingClientRect();
 menu!.style.left = `${Math.max(8, Math.min(rect.left, innerWidth - menu!.offsetWidth - 8))}px`;
 menu!.style.top = `${Math.max(8, Math.min(rect.bottom, innerHeight - menu!.offsetHeight - 8))}px`;
}
function openMenu(trigger: HTMLElement, id: string, groupId: string, kind: 'tab' | 'add' = 'tab') {
 closeMenu(false); menuTrigger = trigger; menuTab = id; menuGroup = groupId; menuKind = kind;
 void tick().then(() => {
  if (!menu || !menu.isConnected || menuGroup !== groupId) return;
  if (!menu.matches(':popover-open')) menu.showPopover();
  menuOpen = true;
  positionMenu(trigger);
  menu.querySelector<HTMLElement>('button:not(:disabled)')?.focus();
 });
}
function menuToggle(event: Event) { menuOpen = (event as ToggleEvent).newState === 'open'; if (!menuOpen) { menuTrigger = null; menuTab = ''; } }
function closeForViewport(event: Event) { if (event.target instanceof Node && menu?.contains(event.target)) return; if (menu?.matches(':popover-open')) closeMenu(false); }
$effect(() => { if (!menuOpen) return; window.addEventListener('scroll', closeForViewport, true); return () => window.removeEventListener('scroll', closeForViewport, true); });
function context(id: string, action: string) { if (action === 'close') close(id); else if (action === 'others') setLayout(closeOthers(layout, id)); else if (action === 'split') setLayout(splitRight(layout, id)); else if (action === 'join') setLayout(joinAll(layout)); else if (action.startsWith('move:')) setLayout(moveTab(layout, id, action.slice(5))); closeMenu(false); focusActiveTab(); }
function menuKey(event: KeyboardEvent) {
 if (event.key === 'Escape') { event.preventDefault(); closeMenu(); return; }
 const buttons = [...(menu?.querySelectorAll<HTMLButtonElement>('button:not(:disabled)') ?? [])];
 const index = buttons.indexOf(document.activeElement as HTMLButtonElement);
 let next = index;
 if (event.key === 'ArrowDown') next = (index + 1) % buttons.length;
 else if (event.key === 'ArrowUp') next = (index + buttons.length - 1) % buttons.length;
 else if (event.key === 'Home') next = 0;
 else if (event.key === 'End') next = buttons.length - 1;
 else return;
 event.preventDefault(); buttons[next]?.focus();
}

$effect(() => {
 const key = storageKey, route = sessionId, roster = conversations;
 untrack(() => {
  if (key !== readyKey) {
   if (menu?.matches(':popover-open')) closeMenu(false);
   const switchedScope = readyKey !== '';
   readyKey = key; toolCache = {}; pending = {}; controls = {};
   let restored: WorkbenchLayout | null = null;
   if (!blocked) try { restored = parseLayout(JSON.parse(localStorage.getItem(key) ?? 'null')); } catch {}
   if (restored) {
    for (const [id, tab] of Object.entries(restored.tabs)) if (tab.kind === 'conversation' && !roster.some(item => item.sessionId === tab.sessionId) && !scopedChildIds().includes(tab.sessionId)) restored = closeTab(restored, id);
    for (const [id, tab] of Object.entries(restored.tabs)) if (tab.kind !== 'conversation' && !roster.some(item => item.sessionId === tab.sessionId)) restored = closeTab(restored, id);
   }
   layout = restored ?? createLayout({ kind: 'conversation', id: conversationTabId(route), sessionId: route, title: roster.find(item => item.sessionId === route)?.title || route });
   if ((switchedScope || !layout.tabs[conversationTabId(route)]) && !blocked) layout = openTab(layout, { kind: 'conversation', id: conversationTabId(route), sessionId: route, title: titleFor(route) });
   previousRoute = route;
   if (!blocked) save();
  } else if (route !== previousRoute) {
   previousRoute = route;
   const tabId = conversationTabId(route);
   layout = openTab(layout, { kind: 'conversation', id: tabId, sessionId: route, title: roster.find(item => item.sessionId === route)?.title || route });
   layout = activateTab(layout, tabId); save();
  }
  for (const item of roster) { const id = conversationTabId(item.sessionId), tab = layout.tabs[id]; if (tab?.kind === 'conversation' && tab.title !== item.title) layout = { ...layout, tabs: { ...layout.tabs, [id]: { ...tab, title: item.title.slice(0, 512) } } }; }
  save();
 });
});
</script>

<svelte:window onresize={(event) => { revealActive(); closeForViewport(event); }} />

<div class="workbench" class:blocked>
 <div class="groups" style={`--group-count:${layout.groups.length}`}>
 {#each layout.groups as group, index (group.id)}
  <header class="group-header" class:mobile-hidden={currentGroup.id !== group.id}>
   <strong>작업 그룹 {index + 1}</strong>
   <div class="tabstrip" role="tablist" tabindex="-1" aria-label={`작업 그룹 ${index + 1}`} ondragover={(event) => { if (acceptsTabDrag(event)) event.preventDefault(); }} ondragleave={(event) => { if (!event.currentTarget.contains(event.relatedTarget as Node)) dropMarker = null; }} ondrop={transfer} data-group={group.id}>
   <span class="navigation-controls" role="presentation"><button class="mobile-menu" type="button" aria-label="메뉴 열기" onclick={() => (ui.drawerOpen = true)}><Icon name="menu" /></button>{#if index === 0 && ui.sidebarHidden}<button class="desktop-sidebar" type="button" aria-label="사이드바 펼치기" title="사이드바 펼치기" onclick={() => ui.toggleSidebar()}><Icon name="sidebar" /></button>{/if}</span>
   {#each group.tabs as id (id)}{@const tab = layout.tabs[id]}
    <div class="tab" class:current={group.active === id} class:drop-before={dropMarker?.groupId === group.id && dropMarker.tabId === id && !dropMarker.after} class:drop-after={dropMarker?.groupId === group.id && dropMarker.tabId === id && dropMarker.after}>
     <button type="button" role="tab" draggable="true" ondragstart={(event) => transfer(event, id)} ondragend={() => dropMarker = null} ondragover={(event) => dragOverTab(event, group.id, id)} ondrop={(event) => dropOnTab(event, group.id, id)} id={`tab-${id}`} aria-selected={group.active === id} tabindex={group.active === id ? 0 : -1} aria-label={tab.title} title={pending[cacheKey(tab.sessionId)] ? `${tab.title} · 응답 필요` : tab.title} onclick={() => selectTab(id)} onkeydown={(event) => { keyTab(event, group, id); if (event.key === 'ContextMenu' || (event.shiftKey && event.key === 'F10')) { event.preventDefault(); openMenu(event.currentTarget, id, group.id); } }} oncontextmenu={(event) => { event.preventDefault(); openMenu(event.currentTarget as HTMLElement, id, group.id); }} aria-haspopup="menu" aria-expanded={menuTab === id && menuOpen}><Icon name={tab.kind === 'conversation' ? 'chat' : tab.kind === 'explorer' ? 'folder' : tab.kind === 'file' ? 'document' : tab.kind === 'tool' ? 'terminal' : 'globe'} class="tab-icon"/><span class="tab-title">{tab.title}</span>{#if tab.kind === 'conversation' && pending[cacheKey(tab.sessionId)]}<span class="pending-mark" role="status" aria-label="응답 필요">●</span>{/if}</button><button class="close" type="button" aria-label={`닫기 ${tab.title}`} onclick={() => close(id)}>×</button>
     <button class="more" type="button" aria-label={`탭 메뉴 ${tab.title}`} aria-haspopup="menu" aria-expanded={menuTab === id && menuOpen} onclick={(event) => openMenu(event.currentTarget, id, group.id)}>⋯</button>
    </div>
   {/each}
   </div><button class="add" type="button" aria-label="탭 열기" aria-haspopup="menu" aria-expanded={!menuTab && menuGroup === group.id && menuOpen} onclick={(event) => openMenu(event.currentTarget, '', group.id, 'add')}>＋</button>
   <label class="group-picker">작업 그룹 <select value={layout.focusedGroup} onchange={(event) => setLayout({ ...layout, focusedGroup: event.currentTarget.value })}>{#each layout.groups as option, optionIndex (option.id)}<option value={option.id}>{optionIndex + 1}</option>{/each}</select></label>
  </header>
 {/each}
 {#key storageKey}
 {#each Object.values(layout.tabs) as tab (tab.id)}
  {@const draftKey = JSON.stringify([host, tab.sessionId])}
  {@const owner = layout.groups.find(group => group.tabs.includes(tab.id))}
  {@const ownerIndex = owner ? layout.groups.indexOf(owner) : 0}
  <section class="panel" class:mobile-hidden={currentGroup.id !== owner?.id} style={`grid-column:${ownerIndex + 1}`} hidden={!owner || owner.active !== tab.id} inert={!owner || owner.active !== tab.id}>
   {#if tab.kind === 'conversation'}<div class="conversation-panel">{#if childrenFor(tab.sessionId).length}<SessionMonitor sessions={childrenFor(tab.sessionId)} onopen={(id) => { if (childrenFor(tab.sessionId).some(row => row.sessionId === id)) openConversation(id, owner?.id); }} {canCancel} oncancel={cancelChild} />{/if}<SessionConversation sessionId={tab.sessionId} host={host} runningAgentCount={childrenFor(tab.sessionId).filter(row => row.activity === 'working').length} blocked={blocked || readyKey !== storageKey} initialDraft={drafts.get(draftKey)} onDraftChange={(draft) => drafts.set(draftKey, draft)} onTitle={(title) => updateTitle(tab.sessionId, title)} onTools={(tools) => updateTools(tab.sessionId, tools)} onOpenTool={(tool) => openTool(tab.sessionId, tool)} onPending={(value) => pending = { ...pending, [draftKey]: value }} onControlChange={controlCallback(tab.sessionId, storageKey)} /></div>
   {:else}<WorkspaceDocument {tab} {host} blocked={blocked || readyKey !== storageKey} tools={toolCache[cacheKey(tab.sessionId)] ?? []} onOpenFile={(entry, sid) => openedFile(entry, sid, owner?.id)} onOpenPreview={(port) => openPreview(port, owner?.id)} onOpenTool={(tool, view) => openTool(tab.sessionId, tool, view)} />{/if}
  </section>
 {/each}
 {/key}
 {#each layout.groups as group (group.id)}{#if !group.active}<div class="empty" style={`grid-column:${layout.groups.indexOf(group) + 1}`}><button onclick={() => openExplorer(group.id)}>탐색기 열기</button><button onclick={() => openRecords(group.id)}>명령 실행 기록</button><button onclick={() => openPreview(undefined, group.id)}>포트 미리보기</button>{#each conversations as item (item.sessionId)}<button onclick={() => openConversation(item.sessionId, group.id)}>{item.title}</button>{/each}</div>{/if}{/each}
 </div>

 <div bind:this={menu} class="popover-menu" popover="auto" role="menu" tabindex="-1" aria-label={menuKind === 'tab' ? '탭 작업' : '탭 열기'} onkeydown={menuKey} ontoggle={menuToggle}>
  {#if menuKind === 'tab' && menuTab}
   <button role="menuitem" onclick={() => context(menuTab, 'close')}>닫기</button><button role="menuitem" onclick={() => context(menuTab, 'others')}>다른 탭 닫기</button><button role="menuitem" disabled={(layout.groups.find(g => g.id === menuGroup)?.tabs.length ?? 0) <= 1 || layout.groups.length >= 3} onclick={() => context(menuTab, 'split')}>오른쪽 분할</button><button role="menuitem" disabled={layout.groups.length <= 1} onclick={() => context(menuTab, 'join')}>분할 합치기</button>
   {#each layout.groups.filter(g => g.id !== menuGroup) as target}<button role="menuitem" onclick={() => context(menuTab, `move:${target.id}`)}>그룹 {layout.groups.indexOf(target) + 1} 이동</button>{/each}
  {:else}
   <button role="menuitem" onclick={() => { openExplorer(menuGroup); closeMenu(false); }}>탐색기</button><button role="menuitem" onclick={() => { openRecords(menuGroup); closeMenu(false); }}>명령 실행 기록</button><button role="menuitem" onclick={() => { openPreview(undefined, menuGroup); closeMenu(false); }}>포트 미리보기</button>
   {#each conversations as item (item.sessionId)}<button role="menuitem" onclick={() => { openConversation(item.sessionId, menuGroup); closeMenu(false); }}>{item.title}</button>{/each}
  {/if}
 </div>
</div>
<style>
.workbench {
 display:flex; width:100%; height:100%; min-width:0; min-height:0; overflow:hidden;
 background:white; font-size:13px;
}
.groups {
 display:grid; grid-template-columns:repeat(var(--group-count),minmax(0,1fr));
 grid-template-rows:40px minmax(0,1fr); flex:1; min-width:0; min-height:0;
 overflow:hidden;
}
.group-header {
 grid-row:1; display:flex; align-items:center; min-width:0;
 background:#f1f1f3; border-right:1px solid #ddd; border-bottom:1px solid #ddd;
}
.group-header:nth-child(1){grid-column:1}
.group-header:nth-child(2){grid-column:2}
.group-header:nth-child(3){grid-column:3}
.group-header strong{padding:0 6px;white-space:nowrap;font-size:11px}
.tabstrip{display:flex;flex:1;min-width:0;overflow-x:auto}.navigation-controls{display:flex;flex:none;align-items:center}.navigation-controls button{display:flex;width:34px;height:34px;align-items:center;justify-content:center;border:0;background:transparent}.desktop-sidebar{display:none}.mobile-menu{display:flex}
.tab{display:flex;flex:none;align-items:center;height:40px;border-top:2px solid transparent}
.tab.current{background:white;border-top-color:#2b59c3}
.tab.drop-before{box-shadow:inset 2px 0 #2b59c3}.tab.drop-after{box-shadow:inset -2px 0 #2b59c3}
.tab button{height:34px;padding:0 6px;white-space:nowrap;border:0;background:transparent}.tab button[role="tab"]{display:flex;align-items:center;gap:6px;max-width:230px;min-width:70px;overflow:hidden;text-align:left}.tab-title{overflow:hidden;text-overflow:ellipsis}.tab :global(.tab-icon){width:15px;height:15px;flex:none}.pending-mark{padding-left:2px;color:#9a5200;font-size:10px}.tab .close,.tab .more{width:24px;padding:0;color:#666}.tab .close:hover,.tab .more:hover{background:#e9e9ed;border-radius:4px}
.add{flex:none;width:30px;height:30px;border:0;background:transparent;font-size:18px}:global(.popover-menu){position:fixed;inset:0 auto auto 0;width:max-content;max-width:calc(100vw - 16px);max-height:calc(100vh - 16px);overflow:auto;padding:4px;background:white;border:1px solid #ddd;border-radius:6px;box-shadow:0 8px 24px #0003;z-index:1000}:global(.popover-menu button){display:block;width:100%;padding:8px 12px;border:0;background:white;text-align:left;white-space:nowrap}:global(.popover-menu button:hover:not(:disabled)){background:#f1f3f8}:global(.popover-menu button:disabled){color:#999}.empty{display:flex;align-items:flex-start;gap:8px;padding:16px}.empty button{padding:8px 12px}
.panel{grid-row:2;display:flex;min-width:0;min-height:0;overflow:hidden}.conversation-panel{display:flex;flex-direction:column;flex:1;min-width:0;min-height:0;overflow:hidden}.conversation-panel :global(.session-conversation){flex:1;min-height:0}
.panel[hidden]{display:none}
.group-picker{display:none}
.empty{grid-row:2;padding:16px}
button:focus-visible,select:focus-visible{outline:2px solid #2b59c3}
@media(min-width:1024px){.mobile-menu{display:none}.desktop-sidebar{display:flex}}
@media(max-width:640px){
 .groups{display:block;position:relative;width:100%}
 .group-header{position:relative;height:42px}
 .group-header.mobile-hidden,.panel.mobile-hidden{display:none}
 .panel{position:absolute;inset:42px 0 0}
 .group-picker{display:block;margin-left:auto;white-space:nowrap}
 .group-picker select{width:44px}
}
</style>
