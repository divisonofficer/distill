import assert from 'node:assert/strict';
import { after, before, test } from 'node:test';
import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createServer } from 'vite';

const root = fileURLToPath(new URL('..', import.meta.url));
let vite;
let models;

before(async () => {
vite = await createServer({ root, configFile: resolve(root, 'vite.config.ts'), server: { middlewareMode: true }, appType: 'custom', logLevel: 'silent' });
models = await vite.ssrLoadModule('/src/lib/models.ts');
});

after(async () => { await vite?.close(); });

test('default provider menus show catalog-backed latest family representatives in requested order', () => {
const catalog = [
{ modelId: 'auto', name: 'Auto' },
{ modelId: 'claude/claude-haiku-5-1', name: 'Claude Haiku 5.1' },
{ modelId: 'claude/claude-sonnet-5-9-20250301', name: 'Claude Sonnet 5.9' },
{ modelId: 'claude/claude-fable-5-1', name: 'Claude Fable 5.1' },
{ modelId: 'claude/claude-opus-5-5', name: 'Claude Opus 5.5' },
{ modelId: 'claude/claude-fable-5-2', name: 'Claude Fable 5.2' },
{ modelId: 'claude/claude-sonnet-5-9-20260518', name: 'Claude Sonnet 5.9 (2026-05-18)' },
{ modelId: 'claude/legacy-custom', name: 'Claude Custom' },
{ modelId: 'chatgpt/gpt-5-astra', name: 'GPT 5 Astra' },
{ modelId: 'chatgpt/gpt-6-astra', name: 'GPT 6 Astra' },
{ modelId: 'chatgpt/gpt-6.9-sol', name: 'GPT 6.9 Sol' },
{ modelId: 'chatgpt/gpt-6-10-sol', name: 'GPT 6.10 Sol' },
{ modelId: 'chatgpt/gpt-6-terra', name: 'GPT 6 Terra' },
{ modelId: 'chatgpt/gpt-6-luna', name: 'GPT 6 Luna' },
{ modelId: 'xai/grok-4', name: 'Grok 4' },
{ modelId: 'xai/grok-4.1', name: 'Grok 4.1' },
{ modelId: 'vendor/legacy-model', name: 'Vendor Legacy' }
];
const groups = models.defaultModelGroups(catalog);
assert.deepEqual(groups.find(({ provider }) => provider === 'Claude').models.map(({ modelId }) => modelId), [
'claude/claude-fable-5-2',
'claude/claude-opus-5-5',
'claude/claude-sonnet-5-9-20260518',
'claude/claude-haiku-5-1',
'claude/legacy-custom'
]);
assert.deepEqual(groups.find(({ provider }) => provider === 'ChatGPT').models.map(({ modelId }) => modelId), [
'chatgpt/gpt-6-astra',
'chatgpt/gpt-6-10-sol',
'chatgpt/gpt-6-terra',
'chatgpt/gpt-6-luna'
]);
assert.deepEqual(groups.find(({ provider }) => provider === 'xAI').models.map(({ modelId }) => modelId), ['xai/grok-4.1']);
assert.deepEqual(groups.find(({ provider }) => provider === '기타').models.map(({ modelId }) => modelId), ['vendor/legacy-model']);
assert.equal(groups.flatMap(({ models: entries }) => entries).some(({ modelId }) => modelId === 'auto'), false);
});

test('numeric dot and hyphen versions and dated suffixes compare by catalog model IDs', () => {
const numeric = models.defaultModelGroups([
{ modelId: 'claude/claude-opus-5.9', name: 'Claude Opus 5.9' },
{ modelId: 'claude/claude-opus-5.10', name: 'Claude Opus 5.10' },
{ modelId: 'chatgpt/gpt-6.9-sol', name: 'GPT 6.9 Sol' },
{ modelId: 'chatgpt/gpt-6-10-sol', name: 'GPT 6.10 Sol' }
]);
assert.equal(numeric.find(({ provider }) => provider === 'Claude').models[0].modelId, 'claude/claude-opus-5.10');
assert.equal(numeric.find(({ provider }) => provider === 'ChatGPT').models[0].modelId, 'chatgpt/gpt-6-10-sol');

const dated = models.defaultModelGroups([
{ modelId: 'claude/claude-sonnet-5.5-2025-03-01', name: 'Claude Sonnet 5.5' },
{ modelId: 'claude/claude-sonnet-5-5-2026-05-18', name: 'Claude Sonnet 5.5' }
]);
assert.equal(dated[0].models[0].modelId, 'claude/claude-sonnet-5-5-2026-05-18');

const datedGpt = models.defaultModelGroups([
{ modelId: 'chatgpt/gpt-6.1-sol-2025-03-01', name: 'GPT 6.1 Sol' },
{ modelId: 'chatgpt/gpt-6.1-sol-2026-05-18', name: 'GPT 6.1 Sol' }
]);
assert.equal(datedGpt.find(({ provider }) => provider === 'ChatGPT').models[0].modelId, 'chatgpt/gpt-6.1-sol-2026-05-18');
});

test('search retains legacy catalog models and Auto stays a direct catalog option', () => {
const catalog = [
{ modelId: 'auto', name: 'Auto' },
{ modelId: 'claude/claude-sonnet-5-9', name: 'Claude Sonnet 5.9' },
{ modelId: 'claude/claude-sonnet-4-8', name: 'Claude Sonnet Legacy' },
{ modelId: 'other/custom', name: 'Custom Legacy' }
];
assert.equal(models.isAutoModel(catalog[0]), true);
assert.deepEqual(models.groupModels(catalog, 'legacy').flatMap(({ models: entries }) => entries.map(({ modelId }) => modelId)), [
'claude/claude-sonnet-4-8',
'other/custom'
]);
assert.equal(models.defaultModelGroups(catalog).find(({ provider }) => provider === 'Claude').models[0].modelId, 'claude/claude-sonnet-5-9');
});

test('root and model navigation wrap without producing absent active options', () => {
assert.equal(models.moveMenuIndex(3, 0, 'ArrowUp'), 2);
assert.equal(models.moveMenuIndex(3, 2, 'ArrowDown'), 0);
assert.equal(models.moveMenuIndex(3, -1, 'Home'), 0);
assert.equal(models.moveMenuIndex(3, -1, 'End'), 2);
assert.equal(models.moveMenuIndex(0, -1, 'ArrowDown'), null);
const entries = [{ modelId: 'a', name: 'A' }, { modelId: 'b', name: 'B' }];
assert.equal(models.moveModelIndex(entries, 'a', 'ArrowDown'), 1);
assert.equal(models.moveModelIndex(entries, 'b', 'ArrowDown'), 0);
});

test('picker keeps IME guard and exposes active descendants only for rendered menu options', async () => {
const source = await readFile(resolve(root, 'src/lib/components/ModelSelect.svelte'), 'utf8');
assert.match(source, /async function onKeydown\(event: KeyboardEvent\) \{\s*if \(event\.isComposing \|\| event\.keyCode === 229\) return;/);
assert.match(source, /aria-activedescendant=\{activeDescendant\}/);
assert.match(source, /function trackPopupPosition\(\)[\s\S]*?addEventListener\('resize', reposition\)[\s\S]*?addEventListener\('scroll', reposition, true\)/);
assert.match(source, /function stopTrackingPopup\(\)[\s\S]*?removeEventListener\('resize', reposition\)[\s\S]*?removeEventListener\('scroll', reposition, true\)/);
assert.match(source, /onDestroy\(stopTrackingPopup\)/);
assert.match(source, /function closeMenu\(\)[\s\S]*?stopTrackingPopup\(\)[\s\S]*?hidePopover\(\)/);
assert.match(source, /activeModelIndex < searchModels\.length \? `\$\{id\}-option-\$\{activeModelIndex\}` : undefined/);
assert.match(source, /if \(query\.trim\(\)\) \{\s*const model = searchModels\[activeModelIndex\];\s*if \(model\) choose\(model\.modelId\);/);
});
