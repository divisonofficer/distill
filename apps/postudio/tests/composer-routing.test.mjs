import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const root = new URL('./', import.meta.url);
const session = await readFile(new URL('../src/lib/components/SessionConversation.svelte', root), 'utf8');
const routing = await readFile(new URL('../src/lib/components/ComposerRouting.svelte', root), 'utf8');
const newPage = await readFile(new URL('../src/routes/new/+page.svelte', root), 'utf8');

test('composer mode choices call guarded setter and keep approval confirmation', () => {
  assert.match(session, /\{#each MODES as candidate/);
  assert.match(session, /setMode\(candidate\)/);
  assert.match(session, /next === 'always-approve' && !confirm\(/);
  assert.doesNotMatch(session, /onclick=\{cycleMode\}/);
});

test('routing model selection is direct; separate button shows real account routing information', () => {
  assert.match(session, /<ComposerRouting[^>]+host=\{sessionHost\}[^>]+onselect=\{chooseModel\}/);
  assert.match(routing, /getJson<unknown>\('\/api\/accounts\/routing', currentHost\)/);
  assert.match(routing, /getJson<unknown>\('\/api\/accounts', currentHost\)/);
  assert.match(routing, /<ModelSelect compact[^>]+host=\{host\}[^>]+commitOnSelect=\{false\} onselect=\{onselect\}/);
  assert.match(routing, /aria-label="계정 라우팅 정보"/);
  assert.match(routing, /height:34px/);
  const popup = routing.slice(routing.indexOf('class="routing-popover"'), routing.indexOf('{/if}', routing.indexOf('class="routing-popover"')));
  assert.doesNotMatch(popup, /<ModelSelect/);
  assert.match(routing, /settings#routing-h/);
  assert.doesNotMatch(routing, /postJson|routingPayload/);
});

test('composer form has no workspace cwd chip', () => {
  const formStart = session.indexOf('<form');
  const formEnd = session.indexOf('</form>', formStart);
  assert.notEqual(formStart, -1);
  assert.notEqual(formEnd, -1);
  const composerForm = session.slice(formStart, formEnd);
  assert.doesNotMatch(composerForm, /row\?\.cwd|row\.cwd|aria-label="작업공간/);
});

test('compact composer separates real usage and approval policy from routing', () => {
  const form = session.slice(session.indexOf('<form'), session.indexOf('</form>'));
  assert.match(form, /flex min-w-0 flex-nowrap items-center gap-0\.5/);
  assert.match(form, /max-w-\[180px\]/);
  assert.match(form, /contextUsage\.used\.toLocaleString\(\).*contextUsage\.size\.toLocaleString\(\)/);
  assert.match(form, /stroke-dasharray/);
  assert.match(form, /승인 정책:/);
  assert.match(session, /Number\.isFinite\(view\.usage\.size\) && view\.usage\.size > 0/);
  assert.match(session, /명령 실행 승인이 필요해요/);
  assert.match(session, /onControlChange\?:/);
  assert.match(form, /size-\[38px\] shrink-0 items-center/);
  assert.ok(form.indexOf('type="file"') < form.indexOf('<ComposerRouting'));
  assert.ok(form.indexOf('<ComposerRouting') < form.indexOf('승인 정책:'));
  assert.ok(form.indexOf('승인 정책:') < form.indexOf('aria-label="보내기"'));
  const newToolbar = newPage.slice(newPage.indexOf('<div class="flex min-w-0 flex-nowrap items-center gap-0.5">'), newPage.indexOf('{#if error}', newPage.indexOf('<div class="flex min-w-0 flex-nowrap items-center gap-0.5">')));
  assert.ok(newToolbar.indexOf('type="file"') < newToolbar.indexOf('<ModelSelect'));
  assert.ok(newToolbar.indexOf('<ModelSelect') < newToolbar.indexOf('id="mode"'));
  assert.ok(newToolbar.indexOf('id="mode"') < newToolbar.indexOf('aria-label="작업 시작"'));
  assert.match(newToolbar, /size-\[38px\].*rounded-full/);
  assert.match(routing, /popup\.showPopover\(\);\s*positionPopup\(\);/);
  assert.match(routing, /!event\.defaultPrevented && event\.key === 'Escape'/);
  assert.doesNotMatch(routing, /transform:translateY/);
});

test('composer preserves single running cancel control and 48px two-row input', () => {
  assert.match(session, /<textarea[\s\S]*?rows="2"[\s\S]*?min-h-12/);
  assert.match(session, /role="status"[\s\S]*?aria-label="중단"/);
  assert.equal((session.match(/aria-label="중단"/g) ?? []).length, 1);
});
