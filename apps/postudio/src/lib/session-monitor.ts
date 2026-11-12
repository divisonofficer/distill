import type { BoardRow } from './types';

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
