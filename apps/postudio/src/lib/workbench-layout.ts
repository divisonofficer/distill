export type WorkbenchTab =
  | { kind: 'conversation' | 'explorer'; id: string; title: string; sessionId: string }
  | { kind: 'file'; id: string; title: string; sessionId: string; path: string; mime?: string }
  | { kind: 'tool'; id: string; title: string; sessionId: string; toolId: string; view: 'details' | 'diffs' | 'logs' }
  | { kind: 'preview'; id: string; title: string; sessionId: string; port?: number };

export interface WorkbenchLayout {
  tabs: Record<string, WorkbenchTab>;
  groups: Array<{ id: string; tabs: string[]; active: string | null }>;
  focusedGroup: string;
}

const validTab = (tab: WorkbenchTab): boolean => !!tab && ['conversation', 'explorer', 'file', 'tool', 'preview'].includes(tab.kind) &&
  [tab.id, tab.title, tab.sessionId].every((v) => typeof v === 'string' && v.length > 0 && v.length <= 512) &&
  (tab.kind !== 'file' || (typeof tab.path === 'string' && tab.path.length > 0 && tab.path.length <= 4096 && (tab.mime === undefined || (typeof tab.mime === 'string' && tab.mime.length <= 256)))) &&
  (tab.kind !== 'tool' || (typeof tab.toolId === 'string' && tab.toolId.length <= 512 && ['details', 'diffs', 'logs'].includes(tab.view))) &&
  (tab.kind !== 'preview' || tab.port === undefined || (Number.isInteger(tab.port) && tab.port >= 1 && tab.port <= 65535));

const clone = (layout: WorkbenchLayout): WorkbenchLayout => ({ tabs: { ...layout.tabs }, groups: layout.groups.map((g) => ({ ...g, tabs: [...g.tabs] })), focusedGroup: layout.focusedGroup });
const normalize = (layout: WorkbenchLayout): WorkbenchLayout => {
  layout.groups = layout.groups.filter((g) => g.tabs.length > 0);
  if (layout.groups.length === 0) layout.groups.push({ id: 'group-1', tabs: [], active: null });
  for (const group of layout.groups) if (!group.tabs.includes(group.active ?? '')) group.active = group.tabs[0] ?? null;
  if (!layout.groups.some((g) => g.id === layout.focusedGroup)) layout.focusedGroup = layout.groups[0].id;
  return layout;
};

export function createLayout(initialTab: WorkbenchTab): WorkbenchLayout {
  if (!validTab(initialTab)) throw new TypeError('Invalid workbench tab');
  return { tabs: { [initialTab.id]: { ...initialTab } }, groups: [{ id: 'group-1', tabs: [initialTab.id], active: initialTab.id }], focusedGroup: 'group-1' };
}

export function openTab(layout: WorkbenchLayout, tab: WorkbenchTab, groupId?: string): WorkbenchLayout {
  const next = clone(layout);
  if (next.tabs[tab.id]) return activateTab(next, tab.id);
  if (Object.keys(next.tabs).length >= 50) return next;
  const group = next.groups.find((g) => g.id === (groupId ?? next.focusedGroup));
  if (!validTab(tab) || !group) return next;
  next.tabs[tab.id] = { ...tab }; group.tabs.push(tab.id); group.active = tab.id; next.focusedGroup = group.id;
  return next;
}
export function activateTab(layout: WorkbenchLayout, tabId: string): WorkbenchLayout {
  const next = clone(layout), group = next.groups.find((g) => g.tabs.includes(tabId));
  if (group) { group.active = tabId; next.focusedGroup = group.id; }
  return next;
}
export function closeTab(layout: WorkbenchLayout, tabId: string): WorkbenchLayout {
  const next = clone(layout);
  if (next.tabs[tabId]) delete next.tabs[tabId];
  for (const group of next.groups) group.tabs = group.tabs.filter((id) => id !== tabId);
  return normalize(next);
}
export function closeOthers(layout: WorkbenchLayout, tabId: string): WorkbenchLayout {
  const next = clone(layout), owner = next.groups.find((g) => g.tabs.includes(tabId));
  if (!owner) return next;
  for (const id of owner.tabs) if (id !== tabId) delete next.tabs[id];
  owner.tabs = [tabId]; owner.active = tabId; next.focusedGroup = owner.id;
  return next;
}
export function tabDropIndex(sourceTabs: string[], targetTabs: string[], sourceId: string, targetId: string, after: boolean): number | null {
  if (sourceId === targetId) return null;
  const remaining = sourceTabs === targetTabs ? targetTabs.filter((id) => id !== sourceId) : targetTabs;
  const targetIndex = remaining.indexOf(targetId);
  if (targetIndex < 0) return null;
  return targetIndex + (after ? 1 : 0);
}

export function moveTab(layout: WorkbenchLayout, tabId: string, targetGroup: string, index?: number): WorkbenchLayout {
  const next = clone(layout), source = next.groups.find((g) => g.tabs.includes(tabId)), target = next.groups.find((g) => g.id === targetGroup);
  if (!source || !target) return next;
  source.tabs = source.tabs.filter((id) => id !== tabId);
  const position = Math.max(0, Math.min(index ?? target.tabs.length, target.tabs.length));
  target.tabs.splice(position, 0, tabId); target.active = tabId; next.focusedGroup = target.id;
  return normalize(next);
}
export function splitRight(layout: WorkbenchLayout, tabId: string): WorkbenchLayout {
  const next = clone(layout), source = next.groups.find((g) => g.tabs.includes(tabId));
  if (!source || source.tabs.length < 2 || next.groups.length >= 3) return next;
  source.tabs = source.tabs.filter((id) => id !== tabId);
  let number = 1;
  while (next.groups.some((group) => group.id === `group-${number}`)) number++;
  const group = { id: `group-${number}`, tabs: [tabId], active: tabId };
  next.groups.splice(next.groups.indexOf(source) + 1, 0, group); next.focusedGroup = group.id;
  return normalize(next);
}
export function joinAll(layout: WorkbenchLayout): WorkbenchLayout {
  const next = clone(layout), focused = next.groups.find((g) => g.id === next.focusedGroup), previousActive = focused?.active;
  const tabs = next.groups.flatMap((g) => g.tabs);
  next.groups = [{ id: focused?.id ?? next.groups[0].id, tabs, active: previousActive && tabs.includes(previousActive) ? previousActive : tabs[0] ?? null }];
  next.focusedGroup = next.groups[0].id;
  return next;
}

export function parseLayout(value: unknown): WorkbenchLayout | null {
  if (!value || typeof value !== 'object') return null;
  const raw = value as Record<string, unknown>;
  if (Object.keys(raw).sort().join(',') !== 'focusedGroup,groups,tabs' || !raw.tabs || typeof raw.tabs !== 'object' || Array.isArray(raw.tabs) || !Array.isArray(raw.groups) || raw.groups.length < 1 || raw.groups.length > 3) return null;
  const entries = Object.entries(raw.tabs as Record<string, unknown>);
  if (entries.length > 50) return null;
  const tabs: Record<string, WorkbenchTab> = {};
  for (const [key, value] of entries) {
    if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
    const tab = value as WorkbenchTab;
    if (Object.keys(tab).some((k) => !['kind','id','title','sessionId','path','mime','toolId','view','port'].includes(k)) || key !== tab.id || ['__proto__', 'constructor', 'prototype'].includes(key) || !validTab(tab)) return null;
    tabs[key] = { ...tab };
  }
  const groups: WorkbenchLayout['groups'] = [], owned = new Set<string>(), ids = new Set<string>();
  for (const value of raw.groups) {
    if (!value || typeof value !== 'object') return null;
    const g = value as Record<string, unknown>;
    if (Object.keys(g).sort().join(',') !== 'active,id,tabs' || typeof g.id !== 'string' || !g.id || g.id.length > 128 || ids.has(g.id) || !Array.isArray(g.tabs) || (g.active !== null && typeof g.active !== 'string')) return null;
    ids.add(g.id);
    const groupTabs = new Set<string>();
    for (const id of g.tabs) {
      if (typeof id !== 'string' || !Object.hasOwn(tabs, id) || owned.has(id) || groupTabs.has(id)) return null;
      groupTabs.add(id);
      owned.add(id);
    }
    if (g.active !== null && !(g.tabs as string[]).includes(g.active)) return null;
    groups.push({ id: g.id, tabs: [...g.tabs] as string[], active: g.active as string | null });
  }
  if (owned.size !== entries.length || typeof raw.focusedGroup !== 'string' || !ids.has(raw.focusedGroup) || (groups.length > 1 && groups.some((g) => !g.tabs.length))) return null;
  return normalize({ tabs, groups, focusedGroup: raw.focusedGroup });
}
