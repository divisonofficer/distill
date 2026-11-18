import assert from 'node:assert/strict';
import { after, before, test } from 'node:test';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createServer } from 'vite';

const root = fileURLToPath(new URL('..', import.meta.url));
let vite;
let groupWorkspaceRows;
let workspaceDisplayName;
let groupWorkspaceChoices;
let workspacesForHost;
let visibleWorkspaceRows;
let sortRows;

before(async () => {
	vite = await createServer({
		root,
		configFile: resolve(root, 'vite.config.ts'),
		server: { middlewareMode: true },
		appType: 'custom',
		logLevel: 'silent'
	});
	({ groupWorkspaceRows, workspaceDisplayName, groupWorkspaceChoices, workspacesForHost, visibleWorkspaceRows } = await vite.ssrLoadModule('/src/lib/workspaces.ts'));
	({ sortRows } = await vite.ssrLoadModule('/src/lib/board.svelte.ts'));
});

after(async () => {
	await vite?.close();
});

test('workspace groups use full cwd, mark missing cwd, and preserve sorted session order', () => {
	const rows = [
		{ sessionId: 'north-working', cwd: '/srv/north/app', activity: 'working', lastChangeUnixMs: 90 },
		{ sessionId: 'south-input', cwd: '/srv/south/app', activity: 'needs_input', lastChangeUnixMs: 50 },
		{ sessionId: 'north-input', cwd: '/srv/north/app', activity: 'needs_input', lastChangeUnixMs: 80 },
		{ sessionId: 'south-working', cwd: '/srv/south/app', activity: 'working', lastChangeUnixMs: 100 },
		{ sessionId: 'missing-cwd', activity: 'working', lastChangeUnixMs: 10 },
		{ sessionId: 'north-idle', cwd: '/srv/north/app', activity: 'idle', lastChangeUnixMs: 200 }
	];
	const groups = groupWorkspaceRows(sortRows(rows));

	assert.deepEqual(
		groups.map(({ cwd, label, title, secondary, rows: sessions }) => [
			cwd,
			label,
			title,
			secondary,
			sessions.map(({ sessionId }) => sessionId)
		]),
		[
			['/srv/north/app', 'app', '/srv/north/app', '/srv/north', ['north-input', 'north-working', 'north-idle']],
			['/srv/south/app', 'app', '/srv/south/app', '/srv/south', ['south-input', 'south-working']],
			[undefined, '작업공간 없음', undefined, undefined, ['missing-cwd']]
		]
	);
});

test('temporary workspace presentation hides raw names but keeps exact identities', () => {
const paths = ['/var/folders/aa/bb/T/.tmpSecret', '/tmp/raw-name', '/private/tmp/another', '/srv/app'];
const groups = groupWorkspaceRows(paths.map((cwd, index) => ({ cwd, sessionId: String(index) })));
assert.deepEqual(groups.map((group) => group.label), ['임시 작업공간 1', '임시 작업공간 2', '임시 작업공간 3', 'app']);
assert.deepEqual(groups.map((group) => group.title), paths);
assert.equal(workspaceDisplayName(paths[0]), '임시 작업공간');
assert.equal(workspaceDisplayName(paths[3]), paths[3]);
assert.equal(new Set(groups.map((group) => group.id)).size, paths.length);
});

test('temporary detection uses macOS and Windows temp roots, not path names', () => {
const paths = [
'/tmp/rename-abc',
'/private/tmp/subagent-1',
'/var/folders/aa/bb/T/test-run',
'/private/var/folders/aa/bb/T/private-session',
String.raw`C:\Users\alice\AppData\Local\Temp\rename-abc`,
'C:/Windows/Temp/subagent-1',
String.raw`C:\Temp\test-run`
];
const groups = groupWorkspaceChoices([...paths, '/srv/project-test', '/srv/rename-abc']);
assert.deepEqual(groups.temporary.map(({ cwd }) => cwd), paths);
assert.deepEqual(groups.normal.map(({ cwd }) => cwd), ['/srv/project-test', '/srv/rename-abc']);
});

test('new workspace choices deduplicate full paths and disambiguate duplicate basenames', () => {
const groups = groupWorkspaceChoices([
'/srv/north/app',
'/srv/south/app',
'/srv/north/app',
'/tmp/north/cache',
'/tmp/south/cache',
'/tmp/north/cache'
]);
assert.deepEqual(groups.normal.map(({ cwd, label, secondary }) => [cwd, label, secondary]), [
['/srv/north/app', 'app', '/srv/north'],
['/srv/south/app', 'app', '/srv/south']
]);
assert.deepEqual(groups.temporary.map(({ cwd, label, secondary }) => [cwd, label, secondary]), [
['/tmp/north/cache', 'cache', '/tmp/north'],
['/tmp/south/cache', 'cache', '/tmp/south']
]);
});

test('workspaces and recents stay scoped to active host', () => {
const rows = [
{ cwd: '/host-a/board', apiOrigin: 'host-a', sessionId: 'a' },
{ cwd: '/host-b/board', apiOrigin: 'host-b', sessionId: 'b' },
{ cwd: '/host-a/legacy', sessionId: 'legacy' }
];
const active = workspacesForHost(rows, ['/host-a/recent', '/host-a/board'], 'host-a', 'host-a');
assert.deepEqual(active.normal.map(({ cwd }) => cwd), ['/host-a/board', '/host-a/legacy', '/host-a/recent']);
const pinned = workspacesForHost(rows, ['/host-a/recent'], 'host-b', 'host-a');
assert.deepEqual(pinned.normal.map(({ cwd }) => cwd), ['/host-b/board']);
});

test('actual parent metadata nests children and nested children under origin workspace', () => {
const rows = [
{ sessionId: 'parent', cwd: '/srv/project', apiOrigin: 'host-a', title: 'Parent' },
{ sessionId: 'child', cwd: '/tmp/agent-child', apiOrigin: 'host-a', parentSessionId: 'parent', sessionKind: 'subagent' },
{ sessionId: 'nested', cwd: '/tmp/nested-child', apiOrigin: 'host-a', parentSessionId: 'child', sessionKind: 'subagent_fork' }
];
const [group] = groupWorkspaceRows(rows);
assert.equal(group.cwd, '/srv/project');
assert.deepEqual(group.rows.map(({ sessionId }) => sessionId), ['parent', 'child', 'nested']);
assert.equal(group.sessions[0].row.sessionId, 'parent');
assert.equal(group.sessions[0].children[0].row.sessionId, 'child');
assert.equal(group.sessions[0].children[0].children[0].row.sessionId, 'nested');
assert.deepEqual(workspacesForHost(rows, [], 'host-a', 'host-a').normal.map(({ cwd }) => cwd), ['/srv/project']);
});

test('orphan and cyclic parent metadata break safely and deterministically', () => {
const rows = [
{ sessionId: 'orphan', cwd: '/tmp/orphan', apiOrigin: 'host-a', parentSessionId: 'missing', sessionKind: 'subagent' },
{ sessionId: 'fork', cwd: '/srv/fork', apiOrigin: 'host-a', parentSessionId: 'orphan', sessionKind: 'fork' },
{ sessionId: 'cycle-a', cwd: '/tmp/cycle-a', apiOrigin: 'host-a', parentSessionId: 'cycle-b', sessionKind: 'subagent' },
{ sessionId: 'cycle-b', cwd: '/tmp/cycle-b', apiOrigin: 'host-a', parentSessionId: 'cycle-a', sessionKind: 'subagent' }
];
const groups = groupWorkspaceRows(rows);
assert.equal(groups.find(({ cwd }) => cwd === '/tmp/orphan').sessions[0].row.sessionId, 'orphan');
assert.equal(groups.find(({ cwd }) => cwd === '/srv/fork').sessions[0].row.sessionId, 'fork');
const cycle = groups.filter(({ cwd }) => cwd === '/tmp/cycle-a' || cwd === '/tmp/cycle-b');
assert.equal(cycle.length, 1);
const flatten = (items) => items.flatMap((item) => [item.row.sessionId, ...flatten(item.children)]);
assert.deepEqual(flatten(cycle[0].sessions).sort(), ['cycle-a', 'cycle-b']);
});

test('identical session ids stay isolated per host', () => {
const rows = [
{ sessionId: 'same', cwd: '/host-a/project', apiOrigin: 'host-a' },
{ sessionId: 'same', cwd: '/host-b/project', apiOrigin: 'host-b' },
{ sessionId: 'child-a', cwd: '/tmp/a', apiOrigin: 'host-a', parentSessionId: 'same', sessionKind: 'subagent' },
{ sessionId: 'child-b', cwd: '/tmp/b', apiOrigin: 'host-b', parentSessionId: 'same', sessionKind: 'subagent' }
];
const groups = groupWorkspaceRows(rows);
assert.equal(groups.length, 2);
assert.equal(new Set(groups.map(({ id }) => id)).size, 2);
assert.deepEqual(groups.map((group) => group.sessions[0].children.map(({ row }) => row.sessionId)), [['child-a'], ['child-b']]);
});

test('temporary children inherit origin visibility and selected temp rows stay host-scoped', () => {
const rows = [
{ sessionId: 'parent', cwd: '/srv/project', apiOrigin: 'host-a' },
{ sessionId: 'child', cwd: '/tmp/child', apiOrigin: 'host-a', parentSessionId: 'parent', sessionKind: 'subagent' },
{ sessionId: 'selected', cwd: '/tmp/selected', apiOrigin: 'host-a' },
{ sessionId: 'selected', cwd: '/tmp/other-host', apiOrigin: 'host-b' },
{ sessionId: 'hidden', cwd: '/tmp/hidden', apiOrigin: 'host-a' }
];
const visible = visibleWorkspaceRows(rows, false, undefined, 'host-a', 'host-a');
assert.deepEqual(visible.map(({ sessionId }) => sessionId), ['parent', 'child']);
const selected = visibleWorkspaceRows(rows, false, 'selected', 'host-a', 'host-a');
assert.deepEqual(selected.map(({ sessionId }) => sessionId), ['parent', 'child', 'selected']);
const [group] = groupWorkspaceRows(visible);
assert.equal(group.sessions[0].children[0].row.sessionId, 'child');
});
