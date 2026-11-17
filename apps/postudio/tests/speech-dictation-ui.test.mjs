import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const root = new URL('../src/', import.meta.url);
const component = await readFile(new URL('lib/components/SpeechDictation.svelte', root), 'utf8');
const session = await readFile(new URL('lib/components/SessionConversation.svelte', root), 'utf8');
const chat = await readFile(new URL('routes/c/[id]/+page.svelte', root), 'utf8');
const newTask = await readFile(new URL('routes/new/+page.svelte', root), 'utf8');

test('speech control starts as an icon-only trigger with hidden, accessible settings', () => {
  const trigger = component.slice(component.indexOf('<button'), component.indexOf('</button>'));
  assert.match(trigger, /aria-haspopup="dialog"/);
  assert.match(trigger, /aria-expanded=\{panelOpen\}/);
  assert.match(trigger, /aria-label=\{capture\.requested \? '음성 입력 중지'/);
  assert.match(trigger, /<svg aria-hidden="true"/);
  assert.doesNotMatch(trigger, /음성 입력 시작|음성 입력 중지</);
  assert.match(component, /hidden=\{!panelOpen\}/);
  assert.match(component, /value="ko-KR"/);
  assert.match(component, /value="en-US"/);
  assert.match(component, /오디오를 처리할 수 있습니다/);
  assert.match(component, /class="fixed z-50 max-h-\[min\(26rem,calc\(100dvh-2rem\)\)\] overflow-y-auto/);
  assert.match(component, /if \(next\.error\) panelOpen = true/);
  assert.match(component, /handleEscape/);
  assert.match(component, /다시 시도/);
});

test('all composer routes place mic beside send without changing send control', () => {
  const sessionForm = session.slice(session.indexOf('<form'), session.indexOf('</form>'));
  const chatForm = chat.slice(chat.indexOf('<form class="flex flex-col gap-1 rounded-\[26px\]'), chat.indexOf('</form>', chat.indexOf('<form class="flex flex-col gap-1 rounded-\[26px\]')));
  const taskToolbar = newTask.slice(newTask.indexOf('<div class="flex min-w-0 flex-nowrap items-center gap-0.5">'), newTask.indexOf('{#if error}', newTask.indexOf('<div class="flex min-w-0 flex-nowrap items-center gap-0.5">')));
  for (const form of [sessionForm, chatForm, taskToolbar]) {
    assert.ok(form.indexOf('<SpeechDictation') >= 0, 'mic control is present');
    assert.ok(form.indexOf('<SpeechDictation') < form.indexOf('type="submit"'), 'mic precedes existing send button');
  }
  assert.match(sessionForm, /<SpeechDictation id=\{`session-speech-/);
  assert.match(chatForm, /<SpeechDictation id=\{`chat-speech-/);
  assert.match(taskToolbar, /<SpeechDictation id="new-task-speech"/);
  assert.match(sessionForm, /aria-label="보내기"/);
  assert.match(chatForm, /size-9 items-center justify-center rounded-full/);
  assert.match(taskToolbar, /aria-label="작업 시작"/);
});
