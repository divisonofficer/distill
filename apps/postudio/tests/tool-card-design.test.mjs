import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const toolRow = await readFile(new URL('../src/lib/components/ToolRow.svelte', import.meta.url), 'utf8');
const interaction = await readFile(new URL('../src/lib/components/InteractionCard.svelte', import.meta.url), 'utf8');

test('counts complete diff line additions and removals', () => {
  const match = toolRow.match(/export function countDiffChanges\([^)]*\) \{([\s\S]*?)\n\}\n<\/script>/);
  assert.ok(match, 'diff counter exists');
  const countDiffChanges = new Function('diffs', match[1]);
  assert.deepEqual(countDiffChanges([{ old: 'same\n' + Array.from({ length: 100 }, (_, i) => `old-${i}`).join('\n') + '\nend', new: 'same\n' + Array.from({ length: 100 }, (_, i) => `old-${i}`).join('\n') + '\nend!' }]), { added: 1, removed: 1 });
  assert.deepEqual(countDiffChanges([{ old: 'a\nb', new: 'a\nb\nc\nd' }]), { added: 2, removed: 0 });
  assert.deepEqual(countDiffChanges([{ old: '', new: '' }]), { added: 0, removed: 0 });
  assert.deepEqual(countDiffChanges([{ old: '', new: 'a\nb\n' }]), { added: 2, removed: 0 });
  assert.deepEqual(countDiffChanges([{ old: 'a\r\nb\r\n', new: '' }]), { added: 0, removed: 2 });
  assert.deepEqual(countDiffChanges([{ old: 'x\n', new: 'x' }]), { added: 0, removed: 0 });
  assert.deepEqual(countDiffChanges([{ old: 'a\nb\nc\nd', new: 'a\nx\nc\ny' }]), { added: 2, removed: 2 });
  assert.equal(countDiffChanges([{ old: 'a\n'.repeat(1001), new: 'b\n'.repeat(1000) }]), null);
});

test('tool rows keep command output and auxiliary call data separate', () => {
  assert.match(toolRow, /overflow-hidden rounded-xl border/);
  assert.match(toolRow, /bg-neutral-950 p-2/);
  assert.match(toolRow, /<details class="text-xs text-neutral-400"><summary[^>]*>입력 데이터/);
  assert.match(toolRow, /<summary[^>]*>호출 정보/);
  assert.match(toolRow, /onselect\?\.\(item\)/);
  assert.match(toolRow, /item\.status === 'failed' \|\| item\.status === 'in_progress'/);
  assert.match(toolRow, /commandExitCode !== 0/);
  assert.doesNotMatch(toolRow, /Badge/);
});

test('plan card offers explicit action outcomes and only asks for feedback on request', () => {
  assert.match(interaction, /계획대로 실행/);
  assert.match(interaction, /onclick=\{\(\) => planReply\('approved'\)\}/);
  assert.match(interaction, /editingRequest \? planReply\('cancelled'\) : \(editingRequest = true\)/);
  assert.match(interaction, />취소<\/button>/);
  assert.match(interaction, /onclick=\{\(\) => planReply\('abandoned'\)\}/);
  assert.match(interaction, /\{#if editingRequest\}[\s\S]*?수정 의견/);
});
