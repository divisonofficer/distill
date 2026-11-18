import test, { before, after } from 'node:test';
import assert from 'node:assert/strict';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createServer } from 'vite';

const root = fileURLToPath(new URL('..', import.meta.url));
let vite, model;
before(async () => {
 vite = await createServer({ root, configFile: resolve(root, 'vite.config.ts'), server: { middlewareMode: true }, appType: 'custom', logLevel: 'silent' });
 model = await vite.ssrLoadModule('/src/lib/workbench-layout.ts');
});
after(async () => { await vite?.close(); });
const tab = (id, kind = 'file') => ({ id, kind, title: id, sessionId: 'session', ...(kind === 'file' ? { path: `/src/${id}` } : {}) });
const roundTrip = (layout) => assert.ok(model.parseLayout(JSON.parse(JSON.stringify(layout))));

test('same-kind tabs, conversations, reorder, move, split cap, join and close', () => {
 const { createLayout, openTab, moveTab, splitRight, joinAll, closeTab, closeOthers } = model;
 let layout = createLayout(tab('a'));
 roundTrip(layout);
 layout = openTab(layout, tab('b'));
 layout = openTab(layout, tab('chat', 'conversation'));
 roundTrip(layout);
 assert.deepEqual(layout.groups[0].tabs, ['a', 'b', 'chat']);
 layout = moveTab(layout, 'b', 'group-1', 0);
 roundTrip(layout);
 assert.deepEqual(layout.groups[0].tabs, ['b', 'a', 'chat']);
 layout = splitRight(layout, 'b');
 roundTrip(layout);
 assert.equal(layout.groups[0].active, 'a');
 layout = splitRight(layout, 'a');
 roundTrip(layout);
 assert.equal(layout.groups.length, 3);
 assert.equal(splitRight(layout, 'chat').groups.length, 3);
 layout = joinAll(layout);
 roundTrip(layout);
 assert.equal(layout.groups.length, 1);
 assert.equal(layout.groups[0].active, 'a');
 layout = closeTab(layout, 'a');
 roundTrip(layout);
 assert.equal(layout.tabs.a, undefined);
 assert.equal(closeOthers(layout, 'b').groups[0].tabs.length, 1);
});

test('close others preserves other group and selected draft/conversation descriptors', () => {
 let layout = model.createLayout(tab('draft-a'));
 layout = model.openTab(layout, tab('chat-a', 'conversation'));
 layout = model.openTab(layout, tab('other-a'));
 layout = model.splitRight(layout, 'other-a');
 layout = model.openTab(layout, tab('chat-b', 'conversation'));
 const foreign = structuredClone(layout.groups[1]);
 const foreignTabs = structuredClone(Object.fromEntries(foreign.tabs.map(id => [id, layout.tabs[id]])));
 layout = model.closeOthers(layout, 'chat-a');
 assert.deepEqual(layout.groups[0], { id: 'group-1', tabs: ['chat-a'], active: 'chat-a' });
 assert.deepEqual(layout.groups[1], foreign);
 assert.deepEqual(Object.fromEntries(foreign.tabs.map(id => [id, layout.tabs[id]])), foreignTabs);
 assert.deepEqual(layout.tabs['chat-a'], tab('chat-a', 'conversation'));
 assert.deepEqual(model.parseLayout(JSON.parse(JSON.stringify(layout))), layout);
});

test('drop insertion handles same-group direction, self, cross-group and append', () => {
 const { tabDropIndex, createLayout, openTab, splitRight, moveTab } = model;
 const ids = ['a', 'b', 'c', 'd'];
 assert.equal(tabDropIndex(ids, ids, 'a', 'c', false), 1);
 assert.equal(tabDropIndex(ids, ids, 'a', 'c', true), 2);
 assert.equal(tabDropIndex(ids, ids, 'd', 'b', false), 1);
 assert.equal(tabDropIndex(ids, ids, 'd', 'b', true), 2);
 assert.equal(tabDropIndex(ids, ids, 'b', 'b', true), null);
 let layout = createLayout(tab('a'));
 for (const id of ['b', 'c']) layout = openTab(layout, tab(id));
 layout = splitRight(layout, 'c');
 assert.equal(tabDropIndex(['a', 'b'], ['c'], 'b', 'c', true), 1);
 layout = moveTab(layout, 'b', 'group-2', 1);
 assert.deepEqual(layout.groups.map(group => group.tabs), [['a'], ['c', 'b']]);
 layout = moveTab(layout, 'a', 'group-2');
 assert.deepEqual(layout.groups.map(group => group.tabs), [['c', 'b', 'a']]);
 assert.deepEqual(model.parseLayout(JSON.parse(JSON.stringify(layout))), layout);
});

test('empty groups removed and repeated splits use unique IDs', () => {
 let layout = model.createLayout(tab('a'));
 layout = model.openTab(layout, tab('b'));
 layout = model.openTab(layout, tab('c'));
 layout = model.splitRight(layout, 'a');
 layout = model.moveTab(layout, 'b', 'group-2');
 layout = model.moveTab(layout, 'c', 'group-2');
 assert.equal(layout.groups.some((g) => g.id === 'group-1'), false);
 assert.deepEqual(layout.groups.map((g) => g.tabs), [['a', 'b', 'c']]);
 layout = model.moveTab(layout, 'c', 'group-2');
 layout = model.moveTab(layout, 'a', 'group-2');
 layout = model.splitRight(layout, 'b');
 assert.equal(new Set(layout.groups.map((g) => g.id)).size, layout.groups.length);
 roundTrip(layout);
});

test('open cap, tool descriptor, duplicate membership, reserved IDs and invalid persistence', () => {
 const { createLayout, openTab, parseLayout } = model;
 const tool = { id: 'tool', kind: 'tool', title: 'Tool', sessionId: 'session', toolId: '', view: 'details' };
 assert.ok(parseLayout(createLayout(tool)));
 let full = createLayout(tab('0'));
 for (let i = 1; i < 50; i++) full = openTab(full, tab(String(i)));
 assert.equal(Object.keys(openTab(full, tab('extra')).tabs).length, 50);
 const valid = createLayout(tab('a'));
 assert.ok(parseLayout(valid));
 assert.equal(parseLayout({ ...valid, extra: true }), null);
 assert.equal(parseLayout({ ...valid, groups: [{ id: 'group-1', tabs: ['missing'], active: 'missing' }] }), null);
 assert.equal(parseLayout({ ...valid, groups: [{ id: 'group-1', tabs: ['a', 'a'], active: 'a' }] }), null);
 assert.equal(parseLayout({ ...valid, tabs: { ...valid.tabs, constructor: { ...valid.tabs.a, id: 'constructor' } }, groups: [{ id: 'group-1', tabs: ['a', 'constructor'], active: 'a' }] }), null);
 assert.equal(parseLayout({ ...valid, tabs: { a: { ...valid.tabs.a, kind: 'file', html: '<script>' } } }), null);
 assert.equal(parseLayout({ ...valid, tabs: { a: { ...valid.tabs.a, kind: 'preview', port: 65536 } } }), null);
});
