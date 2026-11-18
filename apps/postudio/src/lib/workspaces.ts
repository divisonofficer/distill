import type { BoardRow } from './types';

export interface WorkspaceSession {
row: BoardRow;
children: WorkspaceSession[];
}

export interface WorkspaceGroup {
id: string;
host: string;
cwd?: string;
label: string;
title?: string;
secondary?: string;
rows: BoardRow[];
sessions: WorkspaceSession[];
}

export interface WorkspaceChoice {
cwd: string;
label: string;
secondary?: string;
}

export interface WorkspaceChoices {
normal: WorkspaceChoice[];
temporary: WorkspaceChoice[];
}

export function isTemporaryWorkspace(cwd: string): boolean {
const path = cwd.replace(/\\/g, '/');
return /^(?:\/private)?\/tmp(?:\/|$)|^(?:\/private)?\/var\/folders\/.*\/T(?:\/|$)/i.test(path)
|| /^[a-z]:\/(?:temp|windows\/temp|users\/[^/]+\/appdata\/local\/temp)(?:\/|$)/i.test(path);
}

export function workspaceDisplayName(cwd?: string): string {
if (!cwd) return '정보 없음';
return isTemporaryWorkspace(cwd) ? '임시 작업공간' : cwd;
}

export function groupWorkspaceChoices(paths: readonly string[]): WorkspaceChoices {
const uniquePaths = [...new Set(paths.filter((path) => path.trim()))];
const choices: WorkspaceChoice[] = uniquePaths.map((cwd) => ({ cwd, label: pathBasename(cwd) }));
const basenameCounts = new Map<string, number>();
for (const choice of choices) {
basenameCounts.set(choice.label, (basenameCounts.get(choice.label) ?? 0) + 1);
}
for (const choice of choices) {
if ((basenameCounts.get(choice.label) ?? 0) > 1) choice.secondary = parentPath(choice.cwd);
}
return {
normal: choices.filter(({ cwd }) => !isTemporaryWorkspace(cwd)),
temporary: choices.filter(({ cwd }) => isTemporaryWorkspace(cwd))
};
}

export function workspacesForHost(
rows: readonly Pick<BoardRow, 'apiOrigin' | 'cwd' | 'sessionId' | 'parentSessionId' | 'sessionKind' | 'sourceWorkspaceDir'>[],
recent: readonly string[],
ownerHost: string,
activeHost: string
): WorkspaceChoices {
const ownedRows = rows.filter((row) => (row.apiOrigin ?? activeHost) === ownerHost);
const paths = groupWorkspaceRows(ownedRows as BoardRow[])
.map(({ cwd }) => cwd)
.filter((cwd): cwd is string => !!cwd);
if (ownerHost === activeHost) paths.push(...recent);
return groupWorkspaceChoices(paths);
}

export function visibleWorkspaceRows(
rows: readonly BoardRow[],
showTemporary: boolean,
selectedSessionId: string | undefined,
selectedHost: string,
defaultHost: string
): BoardRow[] {
if (showTemporary) return [...rows];
const tempRows = new Set(groupWorkspaceRows([...rows])
.filter((group) => isTemporaryWorkspace(group.cwd ?? ''))
.flatMap((group) => group.rows));
return rows.filter((row) => !tempRows.has(row)
|| (row.sessionId === selectedSessionId && (row.apiOrigin ?? defaultHost) === selectedHost));
}

export function groupWorkspaceRows(rows: BoardRow[]): WorkspaceGroup[] {
const parentOf = new Map<BoardRow, BoardRow>();
const bySession = new Map<string, BoardRow>();
const rowKey = (row: BoardRow) => JSON.stringify([row.apiOrigin ?? '', row.sessionId]);
for (const row of rows) {
if (!bySession.has(rowKey(row))) bySession.set(rowKey(row), row);
}
for (const row of rows) {
const parentId = row.parentSessionId;
if (!row.sessionKind?.startsWith('subagent') || !parentId) continue;
const parent = bySession.get(JSON.stringify([row.apiOrigin ?? '', parentId]));
if (parent && parent !== row) parentOf.set(row, parent);
}

for (const row of rows) {
const seen = new Map<BoardRow, number>();
const chain: BoardRow[] = [];
let current: BoardRow | undefined = row;
while (current) {
const cycleStart = seen.get(current);
if (cycleStart !== undefined) {
const cycleRoot = chain.slice(cycleStart).sort((a, b) => rowKey(a).localeCompare(rowKey(b)))[0];
if (cycleRoot) parentOf.delete(cycleRoot);
break;
}
seen.set(current, chain.length);
chain.push(current);
current = parentOf.get(current);
}
}

const nodes = rows.map((row) => ({ row, children: [] as WorkspaceSession[] }));
const nodeForRow = new Map(nodes.map((node) => [node.row, node]));
const groups = new Map<string, WorkspaceGroup>();
let temporaryCount = 0;
const workspaceFor = (row: BoardRow): string | undefined => {
let root = row;
const seen = new Set<BoardRow>();
while (parentOf.has(root) && !seen.has(root)) {
seen.add(root);
root = parentOf.get(root)!;
}
return nonempty(root.sourceWorkspaceDir) || nonempty(root.cwd)
|| nonempty(row.sourceWorkspaceDir) || nonempty(row.cwd);
};

for (const node of nodes) {
const { row } = node;
const host = row.apiOrigin ?? '';
const cwd = workspaceFor(row);
const id = JSON.stringify([host, cwd ?? null]);
let group = groups.get(id);
if (!group) {
group = {
id,
host,
cwd,
label: cwd ? (isTemporaryWorkspace(cwd) ? `임시 작업공간 ${++temporaryCount}` : pathBasename(cwd)) : '작업공간 없음',
title: cwd,
rows: [],
sessions: []
};
groups.set(id, group);
}
group.rows.push(row);
const parent = parentOf.get(row);
const parentNode = parent && nodeForRow.get(parent);
if (parentNode) parentNode.children.push(node);
else group.sessions.push(node);
}

const basenameCounts = new Map<string, number>();
const cwdCounts = new Map<string, number>();
for (const group of groups.values()) {
if (!group.cwd) continue;
basenameCounts.set(group.label, (basenameCounts.get(group.label) ?? 0) + 1);
cwdCounts.set(group.cwd, (cwdCounts.get(group.cwd) ?? 0) + 1);
}
for (const group of groups.values()) {
if (!group.cwd) continue;
if ((cwdCounts.get(group.cwd) ?? 0) > 1) group.secondary = group.host || '호스트';
else if ((basenameCounts.get(group.label) ?? 0) > 1) group.secondary = parentPath(group.cwd);
}
return [...groups.values()];
}

function nonempty(path?: string): string | undefined {
return typeof path === 'string' && path.length > 0 ? path : undefined;
}

function pathBasename(path: string): string {
const trimmed = path.replace(/[\\/]+$/, '');
if (!trimmed || /^[a-z]:$/i.test(trimmed)) return path;
const separator = Math.max(trimmed.lastIndexOf('/'), trimmed.lastIndexOf('\\'));
return trimmed.slice(separator + 1) || path;
}

function parentPath(path: string): string {
const normalized = path.replace(/[\\/]+$/, '') || path;
const separator = Math.max(normalized.lastIndexOf('/'), normalized.lastIndexOf('\\'));
if (separator === 0) return normalized[0];
if (separator === 2 && /^[a-z]:/i.test(normalized)) return path.slice(0, 3);
if (separator > 0) return normalized.slice(0, separator);
return /^[a-z]:/i.test(normalized) ? path.slice(0, 2) : '.';
}
