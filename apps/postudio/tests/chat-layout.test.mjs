import assert from 'node:assert/strict';
import { test } from 'node:test';
import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));

test('inline chat edit keeps original visible, saves as sibling, and hides footer composer', async () => {
const source = await readFile(resolve(root, 'src/routes/c/[id]/+page.svelte'), 'utf8');
assert.match(source, /composerMode\?\.kind === 'edit' \? composerMode\.messageId : selectedLeafId/);
assert.match(source, /내 메시지 수정[\s\S]*?메시지 수정[\s\S]*?새 분기로 저장/);
assert.match(source, /void saveMessage\(false\)/);
assert.match(source, /history && composerMode\?\.kind !== 'edit'/);
assert.match(source, /const parentId = submittedMode\?\.kind === 'edit' \? submittedMode\.parentId : selectedLeafId \|\| null/);
assert.match(source, /composerMode\?\.kind !== 'edit' && message\.role === 'user'/);
assert.match(source, /getChatComposerMode\(activeChatId\)\?\.kind === 'edit'\) return/);
assert.match(source, /mode\.kind === 'edit'[\s\S]*chat-edit-\$\{mode\.messageId\}/);
assert.match(source, /chatAncestry\(history\.messages, composerMode\?\.kind === 'edit' \? composerMode\.messageId : selectedLeafId\)/);
});

test('chat composer owns model selector and separates unsupported tools from save actions', async () => {
const source = await readFile(resolve(root, 'src/routes/c/[id]/+page.svelte'), 'utf8');
const header = source.slice(source.indexOf('<header class="flex min-h-14'), source.indexOf('</header>', source.indexOf('<header class="flex min-h-14')));
const composer = source.slice(source.indexOf('<form class="flex flex-col gap-1 rounded-[26px]'), source.indexOf('</form>', source.indexOf('<form class="flex flex-col gap-1 rounded-[26px]')));
assert.doesNotMatch(header, /<ModelSelect/);
assert.match(header, /chatDisplayTitle/);
assert.match(composer, /<div class="min-w-0 shrink-0"><ModelSelect id="chat-response-model" bind:value=\{modelId\} label="응답 모델" compact initialFromCatalog disabled=\{saving\}/);
assert.match(composer, /aria-label="첨부 및 도구"/);
assert.match(composer, /파일 첨부/);
assert.match(composer, /서버에서 지원하지 않습니다/);
assert.match(composer, /메시지만 저장/);
assert.match(composer, /새 시작 분기/);
assert.match(source, /popovertarget="chat-composer-menu"/);
assert.match(source, /popovertarget="chat-action-menu"/);
assert.match(source, /id="chat-composer-menu"[\s\S]*ontoggle=\{\(event\) => handleMenuToggle\('composer', event\)\}/);
assert.match(source, /id="chat-action-menu"[\s\S]*ontoggle=\{\(event\) => handleMenuToggle\('actions', event\)\}/);
assert.match(source, /function handleMenuToggle\(which: 'composer' \| 'actions', event: ToggleEvent\)/);
assert.match(source, /if \(activeMenu === which\) \{ activeMenu = null; stopMenuPositionTracking\(\); \}/);
assert.doesNotMatch(source, /restoreMenuFocus|function openMenu/);
assert.match(source, /event\.key === 'Escape'[\s\S]*closeMenu\(which, true\)/);
assert.match(source, /menu\.style\.margin = '0'; menu\.style\.inset = 'auto'/);
assert.match(source, /window\.addEventListener\('resize', reposition\)[\s\S]*window\.addEventListener\('scroll', reposition, true\)/);
assert.match(source, /menu\.querySelector<HTMLButtonElement>\('button:not\(:disabled\)'\)\?\.focus\(\)/);
assert.match(composer, /max-h-\[min\(70vh,32rem\)\] overflow-y-auto/);
assert.match(source, /event\.key === 'ArrowDown'[\s\S]*event\.key === 'Home'[\s\S]*event\.key === 'End'/);
assert.match(source, /menu\.querySelector<HTMLButtonElement>\('button:not\(:disabled\)'\)\?\.focus\(\)/);
assert.match(composer, /<\/div>\s*<p class="min-w-0 break-words text-xs/);
assert.doesNotMatch(composer, /<p class="w-full[^>]*class:sr-only/);
assert.match(composer, /role="status"[\s\S]*?중복 전송은 막혀 있습니다/);
assert.match(composer, /중지 기능은 서버에서 지원하지 않습니다/);
});
