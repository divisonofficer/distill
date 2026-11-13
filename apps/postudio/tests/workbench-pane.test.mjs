import test, { before, after } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createServer } from 'vite';

const root = fileURLToPath(new URL('..', import.meta.url));
const source = await readFile(new URL('../src/lib/components/WorkbenchPane.svelte', import.meta.url), 'utf8');
const documentSource = await readFile(new URL('../src/lib/components/WorkspaceDocument.svelte', import.meta.url), 'utf8');
const layoutSource = await readFile(new URL('../src/routes/+layout.svelte', import.meta.url), 'utf8');
const sidebarSource = await readFile(new URL('../src/lib/components/Sidebar.svelte', import.meta.url), 'utf8');
let vite, Pane, WorkspaceDocument, render;
before(async () => {
 vite = await createServer({ root, configFile: resolve(root, 'vite.config.ts'), server: { middlewareMode: true }, appType: 'custom', logLevel: 'silent' });
 ({ default: Pane } = await vite.ssrLoadModule('/src/lib/components/WorkbenchPane.svelte'));
 ({ default: WorkspaceDocument } = await vite.ssrLoadModule('/src/lib/components/WorkspaceDocument.svelte'));
 ({ render } = await vite.ssrLoadModule('svelte/server'));
});
after(async () => { await vite?.close(); });

const props = { sessionId: 'test', host: 'http://localhost:5190', workspaceId: '/repo', conversations: [{ sessionId: 'test', title: '실제 대화 제목' }] };
test('work route has no shell header and pane owns responsive navigation controls', () => {
 assert.match(layoutSource, /page\.route\.id !== '\/s\/\[id\]' && page\.route\.id !== '\/c\/\[id\]'/);
 assert.doesNotMatch(layoutSource, /workContext|sessionOrigin/);
 assert.match(source, /class="mobile-menu"[^>]*aria-label="메뉴 열기"[^>]*onclick=\{\(\) => \(ui\.drawerOpen = true\)\}/);
 assert.match(source, /class="desktop-sidebar"[^>]*aria-label="사이드바 펼치기"[^>]*onclick=\{\(\) => ui\.toggleSidebar\(\)\}/);
 assert.match(source, /<span class="navigation-controls" role="presentation"><button class="mobile-menu"/);
 assert.match(source, /index === 0 && ui\.sidebarHidden/);
 assert.doesNotMatch(source, /\{#if index === 0\}<span class="navigation-controls"/);
 assert.match(source, /@media\(min-width:1024px\)\{\.mobile-menu\{display:none\}\.desktop-sidebar\{display:flex\}\}/);
 assert.match(sidebarSource, /워크스페이스/);
 assert.match(sidebarSource, /\/artifacts/);
});

test('loading route keeps navigation available without shell workspace header', async () => {
 const route = await readFile(new URL('../src/routes/s/[id]/+page.svelte', import.meta.url), 'utf8');
 assert.match(route, /{:else}<div class="flex min-h-0 flex-1 flex-col"><nav[^]*aria-label="탐색"[^]*aria-label="메뉴 열기"[^]*ui\.drawerOpen = true/);
 assert.match(route, /aria-label="사이드바 펼치기"[^]*ui\.toggleSidebar\(\)/);
 assert.match(route, /작업공간을 불러오는 중입니다/);
 assert.doesNotMatch(route, /워크스페이스 정보|아티팩트/);
});

test('blocked hosts render an error and never embed a preview', () => {
 const { body } = render(Pane, { props: { ...props, blocked: true } });
 assert.doesNotMatch(body, /<iframe|<img/);
});

test('tool details escape actual output rather than evaluating tool HTML', () => {
 const { body } = render(WorkspaceDocument, { props: { tab: { kind: 'tool', id: 'tool:test:one:logs', sessionId: 'test', toolId: 'one', view: 'logs', title: 'run' }, host: props.host, tools: [{ kind: 'tool', key: 1, id: 'one', name: 'bash', content: [{ type: 'text', text: '<script>alert(1)</script>' }] }] } });
 assert.match(body, /&lt;script>alert\(1\)&lt;\/script>/);
 assert.doesNotMatch(body, /<script>alert/);
});

test('route keeps roster polling independent from session context', async () => {
 const route = await readFile(new URL('../src/routes/s/[id]/+page.svelte', import.meta.url), 'utf8');
 const effects = [...route.matchAll(/\$effect\(\(\) => \{([\s\S]*?)\n\}\);/g)].map((match) => match[1]);
 assert.equal(effects.length, 2);
 assert.match(effects[0], /const host = apiHost, blocked = !!hostError/);
 assert.doesNotMatch(effects[0], /\bid\b|ui\.workContext/);
 assert.match(effects[1], /const route = id, host = apiHost/);
 assert.match(effects[1], /ui\.workContext = context/);
 assert.match(route, /goto\(sessionHref\(sessionId, destination\.host\), \{ noScroll: true, keepFocus: true \}\)/);
});

test('route stays thin and workspace owns dynamic descriptors', async () => {
 const route = await readFile(new URL('../src/routes/s/[id]/+page.svelte', import.meta.url), 'utf8');
 assert.doesNotMatch(route, /workbench-toolbar|workspace-shortcuts|conversation-tabs/);
 assert.match(source, /<SessionConversation/);
 assert.match(source, /<WorkspaceDocument/);
 assert.match(source, /Object\.values\(layout\.tabs\)/);
 assert.match(source, /role="tablist"/);
 assert.match(source, /bind:this=\{menu\}[^>]*popover="auto"[^>]*role="menu"/);
 assert.match(source, /role="menuitem" onclick=\{\(\) => context\(menuTab, 'close'\)\}/);
 assert.match(source, /openMenu\(event\.currentTarget, id, group\.id\)/);
 assert.match(source, /oncontextmenu=\{\(event\) => \{ event\.preventDefault\(\); openMenu\(event\.currentTarget as HTMLElement, id, group\.id\); \}\}/);
 assert.match(source, /layout = \$state/);
});

test('folder navigation is separate from file selection and requests stay pinned', () => {
 assert.match(documentSource, /let folder = \$state/);
 assert.match(documentSource, /function openFolder\(path: string\)/);
 assert.match(documentSource, /onOpenFile\?\.\(entry, tab\.sessionId\)/);
 assert.match(documentSource, /artifactEntries\(sid, dir, destination\)/);
 assert.match(documentSource, /artifactUrl\(origin, tab\.sessionId, tab\.path\)/);
 assert.match(documentSource, /configuredUntrustedOrigin\(node, host\)/);
 assert.match(documentSource, /generation/);
 assert.match(documentSource, /ports.includes\(tab.port\)/);
 assert.doesNotMatch(documentSource, /allow-same-origin/);
});

test('workspace renders dynamic tab titles from supplied conversation roster', () => {
 const { body } = render(Pane, { props });
 assert.match(body, /실제 대화 제목/);
 assert.match(body, /role="tab"/);
 assert.equal((body.match(/role="tab"/g) ?? []).length, 1);
});
