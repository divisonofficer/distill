import type { BoardRow } from './types';
import type { AgentExecution } from './executions.svelte';

export function joinChildSessions(agents: AgentExecution[], children: BoardRow[], parentId: string) {
 const scoped = children.filter(row => row.parentSessionId === parentId && row.sessionKind?.startsWith('subagent'));
 const matched = new Set(agents.filter(agent => agent.parentSessionId === parentId).map(agent => agent.childSessionId));
 return {
  childFor: (agent: AgentExecution) => agent.parentSessionId === parentId ? scoped.find(row => row.sessionId === agent.childSessionId) : undefined,
  unmatched: scoped.filter((row, index) => !matched.has(row.sessionId) && scoped.findIndex(other => other.sessionId === row.sessionId) === index)
 };
}

export function directChildren(rows: BoardRow[], parentId: string, host: string): BoardRow[] {
 let origin: string;
 try { origin = new URL(host).origin; } catch { return []; }
 return rows.filter((row) => {
  if (row.sessionId === parentId || row.parentSessionId !== parentId || !row.sessionKind?.startsWith('subagent')) return false;
  if (!row.apiOrigin) return true;
  try { return new URL(row.apiOrigin).origin === origin; } catch { return false; }
 });
}

export function scopedChildren(rows: BoardRow[], roots: string[], host: string): string[] {
 const visited = new Set(roots), result = new Set<string>();
 const queue = [...roots];
 while (queue.length) {
  const parent = queue.shift()!;
  for (const row of directChildren(rows, parent, host)) {
   result.add(row.sessionId);
   if (visited.has(row.sessionId)) continue;
   visited.add(row.sessionId); queue.push(row.sessionId);
  }
 }
 return [...result];
}
