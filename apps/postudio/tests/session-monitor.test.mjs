import test from 'node:test';
import assert from 'node:assert/strict';
import { directChildren, scopedChildren } from '../src/lib/session-monitor.ts';
const row = (sessionId, extra = {}) => ({ sessionId, cwd: '/same', title: sessionId, activity: 'working', resident: true, modelId: 'm', isWorktree: false, lastChangeUnixMs: 0, ...extra });
test('selects only explicit direct subagent children scoped to host', () => {
 const rows = [row('child', { parentSessionId: 'parent', sessionKind: 'subagent', apiOrigin: 'https://one.test' }), row('sibling', { parentSessionId: 'other', sessionKind: 'subagent', apiOrigin: 'https://one.test' }), row('cwd-only', { cwd: '/same', sessionKind: 'subagent', apiOrigin: 'https://one.test' }), row('wrong-host', { parentSessionId: 'parent', sessionKind: 'subagent', apiOrigin: 'https://two.test' }), row('worktree', { parentSessionId: 'parent', sessionKind: 'subagent-worktree', isWorktree: true })];
 assert.deepEqual(directChildren(rows, 'parent', 'https://one.test'), [rows[0], rows[4]]);
});
test('missing start time produces no elapsed estimate', () => {
 const child = row('child', { parentSessionId: 'parent', sessionKind: 'subagent', lastChangeUnixMs: 0 });
 assert.equal(directChildren([child], 'parent', 'https://one.test')[0].lastChangeUnixMs, 0);
});
test('scope closure excludes unrelated parent and reaches nested children by explicit links', () => {
 const rows = [row('child', { parentSessionId: 'root', sessionKind: 'subagent' }), row('grandchild', { parentSessionId: 'child', sessionKind: 'subagent-worktree' }), row('unrelated', { parentSessionId: 'elsewhere', sessionKind: 'subagent' }), row('cycle', { parentSessionId: 'grandchild', sessionKind: 'subagent' })];
 assert.deepEqual(new Set(scopedChildren(rows, ['root'], 'https://one.test')), new Set(['child', 'grandchild', 'cycle']));
 assert.deepEqual(new Set(scopedChildren(rows, ['root', 'child'], 'https://one.test')), new Set(['child', 'grandchild', 'cycle']));
 assert.deepEqual(directChildren([row('root', { parentSessionId: 'root', sessionKind: 'subagent' })], 'root', 'https://one.test'), []);
});
test('running footer counts only live command tools and scoped working children', async () => {
 const { readFile } = await import('node:fs/promises');
 const conversation = await readFile(new URL('../src/lib/components/SessionConversation.svelte', import.meta.url), 'utf8');
 const workbench = await readFile(new URL('../src/lib/components/WorkbenchPane.svelte', import.meta.url), 'utf8');
 assert.match(conversation, /item\.status === 'in_progress'.*item\.toolKind.*execute/);
 assert.match(conversation, /명령 \$\{runningCommandCount\}개 실행 중/);
 assert.match(conversation, /runningAgentCount > 0.*에이전트 \$\{runningAgentCount\}개 실행 중/);
 assert.match(workbench, /runningAgentCount=\{childrenFor\(tab\.sessionId\)\.filter\(row => row\.activity === 'working'\)\.length\}/);
});
