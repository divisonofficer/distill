import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { after, before, test } from 'node:test';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createServer } from 'vite';

const root = fileURLToPath(new URL('..', import.meta.url));
let vite;
let permissions;
let models;
let SessionView;
let ToolRow;
let render;
let mock;
let activityBadge;

before(async () => {
vite = await createServer({ root, configFile: resolve(root, 'vite.config.ts'), server: { middlewareMode: true }, appType: 'custom', logLevel: 'silent' });
permissions = await vite.ssrLoadModule('/src/lib/permissions.ts');
models = await vite.ssrLoadModule('/src/lib/models.ts');
({ SessionView } = await vite.ssrLoadModule('/src/lib/session.svelte.ts'));
({ default: ToolRow } = await vite.ssrLoadModule('/src/lib/components/ToolRow.svelte'));
({ render } = await vite.ssrLoadModule('svelte/server'));
mock = await vite.ssrLoadModule('/src/lib/mock.ts');
({ activityBadge } = await vite.ssrLoadModule('/src/lib/format.ts'));
});

after(async () => { await vite?.close(); });

test('permission actions preserve IDs, order safe defaults, and translate No / permanent options', () => {
const options = [
{ id: 'yes-every-time', kind: 'allow_always', name: "Don't ask again" },
{ id: 'no', kind: 'reject_once', name: 'No' },
{ id: 'yes-once', kind: 'allow_once', name: 'Yes' }
];
const actions = permissions.permissionActions(options);
assert.equal(actions.once.id, 'yes-once');
assert.equal(actions.reject.id, 'no');
assert.deepEqual(actions.permanent.map(({ id }) => id), ['yes-every-time']);
assert.deepEqual(actions.other, []);
assert.equal(permissions.permissionOptionLabel(actions.reject), '거부');
assert.equal(permissions.permissionOptionLabel(actions.permanent[0]), '다시는 묻지 않기 · 항상 허용');
});

test('pending permission, question and plan requests map to needs-input and resolved permission clears', () => {
assert.equal(permissions.permissionPromptState(1, 0), true);
assert.equal(permissions.permissionPromptState(0, 1), true);
assert.equal(permissions.permissionPromptState(0, 0), false);
const view = new SessionView();
view.apply({ t: 'permission.request', id: 3, tool_call_id: 'call-3', title: 'Run `test`', options: [{ id: 'yes', kind: 'allow_once', name: 'Yes' }] });
assert.equal(permissions.permissionPromptState(view.permissions.length, 0), true);
view.apply({ t: 'permission.resolved', tool_call_id: 'call-3' });
assert.equal(permissions.permissionPromptState(view.permissions.length, 0), false);
});

test('model filtering groups by known actual ID prefix and keyboard navigation wraps', () => {
const entries = [
{ modelId: 'chatgpt/gpt-6.1-sol', name: 'GPT Sol' },
{ modelId: 'claude/claude-sonnet-5', name: 'Claude Sonnet' },
{ modelId: 'vendor/custom', name: 'Custom' }
];
assert.deepEqual(models.groupModels(entries, 'CLAUDE').map(({ provider, models: grouped }) => [provider, grouped.map(({ modelId }) => modelId)]), [['Claude', ['claude/claude-sonnet-5']]]);
assert.deepEqual(models.groupModels(entries, '').map(({ provider }) => provider), ['ChatGPT', 'Claude', '기타']);
assert.equal(models.moveModelIndex(entries, '', 'ArrowDown'), 0);
assert.equal(models.moveModelIndex(entries, entries[0].modelId, 'ArrowUp'), 2);
assert.equal(models.moveModelIndex(entries, '', 'Home'), 0);
assert.equal(models.moveModelIndex(entries, '', 'End'), 2);
assert.equal(models.moveModelIndex([], '', 'ArrowDown'), null);
assert.equal(models.modelKeyAction('ArrowDown', false), 'open');
assert.equal(models.modelKeyAction('ArrowUp', true), 'move');
assert.equal(models.modelKeyAction('Home', true), 'move');
assert.equal(models.modelKeyAction('End', true), 'move');
assert.equal(models.modelKeyAction('Enter', true), 'select');
assert.equal(models.modelKeyAction('Enter', false), undefined);
assert.equal(models.modelKeyAction('Escape', true), 'close');
assert.equal(models.modelKeyAction('Escape', false), undefined);
});




test('fixture permission updates board from pending to idle after one-time response', async () => {
const rows = [];
const board = mock.mockSocket('/ws/board', { onOpen() {}, onClose() {}, onMessage: (message) => { if (message.t === 'board.upsert') rows.push(...message.sessions); } });
let requested = false;
const session = mock.mockSocket('/ws/session/tool_permission_allow', { onOpen() {}, onClose() {}, onMessage: (message) => { if (message.t === 'permission.request') requested = true; } });
const waitFor = async (predicate) => {
const until = Date.now() + 2000;
while (!predicate() && Date.now() < until) await new Promise((resolve) => setTimeout(resolve, 20));
assert.equal(predicate(), true);
};
try {
await waitFor(() => rows.some((row) => row.sessionId === 'tool_permission_allow' && row.activity === 'needs_input') && requested);
session.send({ t: 'permission', id: 0, option_id: 'allow-once' });
await waitFor(() => rows.some((row) => row.sessionId === 'tool_permission_allow' && row.activity === 'idle'));
} finally {
session.close();
board.close();
}
});


test('activity badges distinguish resident idle, dormant, completed and unknown states', () => {
assert.deepEqual(activityBadge('idle'), { label: '유휴', tone: 'idle' });
assert.deepEqual(activityBadge('dormant'), { label: '비상주', tone: 'idle' });
assert.deepEqual(activityBadge('completed'), { label: '완료', tone: 'off' });
assert.deepEqual(activityBadge('dead'), { label: '오류', tone: 'err' });
assert.deepEqual(activityBadge('future_state'), { label: '확인 중', tone: 'off' });
});

test('markdown tables keep readable cells and scroll horizontally without weakening sanitization', async () => {
const source = await readFile(resolve(root, 'src/lib/components/Markdown.svelte'), 'utf8');
const renderer = await readFile(resolve(root, 'src/lib/markdown.ts'), 'utf8');
assert.match(renderer, /gfm: true/);
assert.match(renderer, /DOMPurify\.sanitize\(html\)/);
assert.match(source, /:global\(table\) \{[^}]*display: block;[^}]*overflow-x: auto;/s);
assert.match(source, /:global\(th\),\s*\.md :global\(td\) \{[^}]*border: 1px solid/);
});

test('shell motion keeps one page, stable sidebar width, inert hiding and reduced motion', async () => {
const layout = await readFile(resolve(root, 'src/routes/+layout.svelte'), 'utf8');
assert.doesNotMatch(layout, /\{#key|lg:hidden'|transition-all/);
assert.equal((layout.match(/\{@render children\(\)\}/g) ?? []).length, 1);
assert.match(layout, /previousMode !== next/);
assert.match(layout, /inert=\{!sidebarVisible\}/);
const innerWidth = layout.match(/sidebar-inner[^\n]*w-\[(\d+)px\]/)?.[1];
assert.ok(innerWidth, 'sidebar inner has a stable pixel width');
assert.ok(layout.includes(`.shell-sidebar { width: ${innerWidth}px;`));
assert.ok(layout.includes(`.collapsed .sidebar-inner { transform: translateX(-${innerWidth}px);`));
assert.match(layout, /onresize=\{syncViewport\}/);
assert.match(layout, /prefers-reduced-motion: reduce/);
assert.match(layout, /\.shell-sidebar, \.sidebar-inner, \.drawer-backdrop \{ transition: none;/);
});

test('model selector ignores composition confirmation before keyboard selection', async () => {
const source = await readFile(resolve(root, 'src/lib/components/ModelSelect.svelte'), 'utf8');
assert.match(source, /async function onKeydown\(event: KeyboardEvent\) \{\s*if \(event\.isComposing \|\| event\.keyCode === 229\) return;/);
});

test('saved message restores current parent instead of overriding newer selection', async () => {
const source = await readFile(resolve(root, 'src/routes/c/[id]/+page.svelte'), 'utf8');
assert.match(source, /selectedLeafId = restoreChatParent\(id, messages\);/);
assert.doesNotMatch(source, /selectedLeafId = saved\.messageId/);
});

test('session conversation owns host, model and permission controls', async () => {
const source = await readFile(resolve(root, 'src/lib/components/SessionConversation.svelte'), 'utf8');
assert.match(source, /const hostAddress = \$derived\(new URL\(sessionHost\)\.host\);/);
assert.doesNotMatch(source.slice(source.indexOf('<form'), source.indexOf('</form>')), /row\??\.cwd/);
assert.match(source, /<ComposerRouting/);
assert.match(source, /session-model-\$\{sessionId\}/);
});

test('tool rows use semantic labels, actual command failures, and only available diff stats', () => {
const body = (item) => render(ToolRow, { props: { item } }).body;
const read = body({ kind: 'tool', key: 1, id: 'read-1', name: 'read_file', toolKind: 'read', status: 'completed', input: { path: '/repo/src/main.ts' } });
assert.match(read, /파일 읽기/);
assert.match(read, /\/repo\/src\/main\.ts/);
const failedCommand = body({ kind: 'tool', key: 2, id: 'command-1', name: 'bash', toolKind: 'execute', status: 'completed', input: { command: 'rg missing' }, content: [{ type: 'text', text: 'exit: 2\nnot found' }] });
assert.match(failedCommand, /명령 실행/);
assert.match(failedCommand, /aria-label="실패"/);
assert.match(failedCommand, /exit 2/);
assert.match(failedCommand, /bg-neutral-950/);
assert.match(failedCommand, /not found/);
const successCommand = body({ kind: 'tool', key: 3, id: 'command-2', name: 'bash', toolKind: 'execute', status: 'completed', input: { command: 'printf ok' }, content: [{ type: 'text', text: 'exit: 0\nok' }] });
assert.match(successCommand, /완료/);
assert.doesNotMatch(successCommand, /명령 실패/);
const edit = body({ kind: 'tool', key: 4, id: 'edit-1', name: 'apply_patch', toolKind: 'edit', status: 'completed', content: [{ type: 'diff', path: 'src/main.ts', old: 'old line', new: 'new line' }] });
assert.match(edit, /파일 수정/);
assert.match(edit, /\+1/);
assert.match(edit, /−1/);
const incompleteDiff = body({ kind: 'tool', key: 5, id: 'edit-2', name: 'apply_patch', toolKind: 'edit', status: 'completed', content: [{ type: 'diff', path: 'src/main.ts', new: 'new line' }] });
assert.doesNotMatch(incompleteDiff, /\+\d|−\d/);
const noStats = body({ kind: 'tool', key: 6, id: 'read-2', name: 'read_file', toolKind: 'read', status: 'completed', input: { path: 'README.md' } });
assert.doesNotMatch(noStats, /\+\d|−\d/);
});

test('settings contains absolutely positioned accessible inputs inside its scroll pane', async () => {
const source = await readFile(resolve(root, 'src/routes/settings/+page.svelte'), 'utf8');
assert.match(source, /class="settings-content relative min-h-0[^"\n]*overflow-y-auto overscroll-contain/);
assert.match(source, /aria-label="호스트 주소 목록"/);
assert.match(source, /\{#if fleetError\}<p role="alert"/);
assert.match(source, /prefers-reduced-motion: reduce/);
});

test('session has no duplicate shell header and keeps accessible route heading', async () => {
const source = await readFile(resolve(root, 'src/routes/s/[id]/+page.svelte'), 'utf8');
const layout = await readFile(resolve(root, 'src/routes/+layout.svelte'), 'utf8');
const sidebar = await readFile(resolve(root, 'src/lib/components/Sidebar.svelte'), 'utf8');
assert.doesNotMatch(source, /<header/);
assert.match(source, /<h1 class="sr-only">/);
assert.match(layout, /page\.route\.id !== '\/s\/\[id\]' && page\.route\.id !== '\/c\/\[id\]'/);
assert.doesNotMatch(layout, /workContext/);
assert.match(sidebar, /selectedRow\.sessionId\)\}\/artifacts\{page\.url\.search\}/);
assert.match(source, /<WorkbenchPane /);
assert.match(source, /host=\{destination\.host\}/);
assert.match(source, /roster\.filter\(\(item\) => item\.cwd === row\.cwd\)/);
assert.doesNotMatch(source, /workbench-toolbar|workspace-shortcuts|conversation-tabs|work-grid/);
assert.doesNotMatch(source, /shadow-\[0_-8px_18px/);
});

test('session roster polling stays host-only and current route updates workbar context', async () => {
const source = await readFile(resolve(root, 'src/routes/s/[id]/+page.svelte'), 'utf8');
const effects = [...source.matchAll(/\$effect\(\(\) => \{([\s\S]*?)\n\}\);/g)].map((match) => match[1]);
assert.equal(effects.length, 2);
assert.match(effects[0], /const host = apiHost, blocked = !!hostError/);
assert.doesNotMatch(effects[0], /\bid\b|ui\.workContext/);
assert.match(effects[1], /const route = id, host = apiHost/);
assert.match(effects[1], /workspaceName: basename\(sessionRow\?\.cwd/);
assert.match(effects[1], /ui\.workContext = context/);
assert.match(source, /goto\(sessionHref\(sessionId, destination\.host\), \{ noScroll: true, keepFocus: true \}\)/);
});

test('permission buttons stay tied to offered actions and fit mobile', async () => {
const source = await readFile(resolve(root, 'src/lib/components/SessionConversation.svelte'), 'utf8');
assert.match(source, /aria-label="실행 승인"/);
assert.match(source, /flex flex-wrap items-center gap-2 rounded-b-\[13px\]/);
assert.match(source, /popover="auto"[^>]*aria-label="추가 승인 선택"/);
assert.match(source, /class="ml-auto inline-flex h-9/);
assert.match(source, /\{#if onceOption\}[\s\S]*?option_id: onceOption\.id/);
assert.match(source, /\{#if rejectOption\}[\s\S]*?option_id: rejectOption\.id/);
assert.match(source, /\{#if actions\.permanent\.length \|\| actions\.other\.length\}/);
assert.match(source, /const pendingId = \$derived\(`pending-interactions-\$\{sessionId\}`\)/);
assert.match(source, /id=\{pendingId\}/);
assert.ok(source.indexOf('id={pendingId}') < source.indexOf('class="relative mx-auto'));
});
