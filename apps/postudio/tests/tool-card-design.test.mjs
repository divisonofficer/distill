import assert from 'node:assert/strict';
import { after, before, test } from 'node:test';
import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createServer } from 'vite';

const toolRow = await readFile(new URL('../src/lib/components/ToolRow.svelte', import.meta.url), 'utf8');
const interaction = await readFile(new URL('../src/lib/components/InteractionCard.svelte', import.meta.url), 'utf8');
const root = fileURLToPath(new URL('..', import.meta.url));
let vite;
let ToolRow;
let InteractionCard;
let render;

before(async () => {
  vite = await createServer({ root, configFile: resolve(root, 'vite.config.ts'), server: { middlewareMode: true }, appType: 'custom', logLevel: 'silent' });
  ({ default: ToolRow } = await vite.ssrLoadModule('/src/lib/components/ToolRow.svelte'));
  ({ default: InteractionCard } = await vite.ssrLoadModule('/src/lib/components/InteractionCard.svelte'));
  ({ render } = await vite.ssrLoadModule('svelte/server'));
});
after(async () => { await vite?.close(); });

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
  assert.match(toolRow, /group overflow-hidden rounded-xl border/);
  assert.match(toolRow, /bg-neutral-950 p-2/);
  assert.match(toolRow, /<details class="text-xs text-neutral-400"><summary[^>]*>입력 데이터/);
  assert.match(toolRow, /<summary[^>]*>호출 정보/);
  assert.match(toolRow, /상세 탭 열기/);
  assert.doesNotMatch(toolRow, /<summary[^>]*onclick=/);
  assert.match(toolRow, /item\.status === 'failed' \|\| item\.status === 'in_progress'/);
  assert.match(toolRow, /commandExitCode !== 0/);
  assert.doesNotMatch(toolRow, /Badge/);
});

test('tool rows render operation labels and real arguments for qualified tool names', () => {
  const body = (item, props = {}) => { const html = render(ToolRow, { props: { item, ...props } }).body; return html.slice(html.indexOf('<summary'), html.indexOf('</summary>')); };
  const read = body({ kind: 'tool', key: 1, id: 'r', name: 'distill.read_file', toolKind: 'read', input: { target_file: '/repo/src/a.ts' } });
  assert.match(read, /파일 읽기/);
  assert.match(read, /\/repo\/src\/a\.ts/);
  const grep = body({ kind: 'tool', key: 2, id: 'g', name: 'mcp__tools__grep', input: { pattern: 'needle', path: 'src', glob: '*.ts' } });
  assert.match(grep, /검색/);
  assert.match(grep, /needle/);
  assert.match(grep, /src/);
  assert.match(grep, /\*\.ts/);
  const todos = body({ kind: 'tool', key: 3, id: 't', name: 'todo_write', toolKind: 'write', input: { todos: [{ content: 'one', status: 'completed' }, { content: 'two', status: 'pending' }] } });
  assert.match(todos, /작업 목록 갱신/);
  assert.match(todos, /할 일 2개/);
  assert.doesNotMatch(todos, /파일 수정/);
  const agent = body({ kind: 'tool', key: 4, id: 'a', name: 'spawn_subagent', input: { description: 'Review parser changes', subagent_type: 'explore' } });
  assert.match(agent, /하위 에이전트/);
  assert.match(agent, /Review parser changes/);
  assert.doesNotMatch(agent, /완료|결과|실행 ID/);
  const command = body({ kind: 'tool', key: 5, id: 'c', name: 'execute', toolKind: 'execute', input: { command: 'npm test -- --run' } });
  assert.match(command, /명령 실행/);
  assert.match(command, /npm test -- --run/);
  const searchReplace = body({ kind: 'tool', key: 6, id: 's', name: 'search_replace', toolKind: 'edit', input: { path: 'src/a.ts' } });
  assert.match(searchReplace, /파일 수정/);
  assert.doesNotMatch(searchReplace, /검색/);
  const generic = body({ kind: 'tool', key: 7, id: 'x', name: 'custom_tool', title: 'custom_tool' });
  assert.doesNotMatch(generic, /<code[^>]*>custom_tool<\/code>/);
});

test('tool summary has no tab action and body button opens existing tool tab', () => {
  const html = render(ToolRow, { props: { item: { kind: 'tool', key: 1, id: 'call', name: 'read_file', input: {} }, onselect: () => {} } }).body;
  const summary = html.slice(html.indexOf('<summary'), html.indexOf('</summary>'));
  assert.doesNotMatch(summary, /onclick|상세 탭 열기/);
  assert.match(html, /<button[^>]*>상세 탭 열기<\/button>/);
  assert.match(toolRow, /event\.stopPropagation\(\); onselect\(item\)/);
  const grouped = render(ToolRow, { props: { item: { kind: 'tool', key: 1, id: 'call', name: 'read_file' }, grouped: true } }).body;
  assert.match(grouped, /<details[^>]*class="group bg-white/);
  assert.doesNotMatch(grouped, /<details[^>]*class="group overflow-hidden rounded-xl border/);
});

test('question card renders neutral compact presentation and preserves required and disconnected states', () => {
  const body = render(InteractionCard, { props: { connected: false, reply: () => true, request: { t: 'question.request', id: 'q', session_id: 's', tool_call_id: 'tc', questions: [{ question: 'Pick one', options: [{ label: 'A' }, { label: 'B' }], multiSelect: false }] } } }).body;
  assert.match(body, /rounded-xl border border-neutral-200 bg-white p-3/);
  assert.doesNotMatch(body, /border-sky|bg-sky|shadow-lg/);
  assert.match(interaction, /bg-neutral-50 p-2/);
  assert.match(body, /has-\[:checked\]:border-neutral-500/);
  assert.match(body, /class="h-9 min-w-0 rounded-lg bg-neutral-900/);
  assert.doesNotMatch(body, /flex-1/);
  assert.match(body, /disabled=""/);
  assert.match(body, /type="radio"/);
  assert.match(body, /기타 답변/);
});

test('plan card offers explicit action outcomes and only asks for feedback on request', () => {
  assert.match(interaction, /계획대로 실행/);
  assert.match(interaction, /onclick=\{\(\) => planReply\('approved'\)\}/);
  assert.match(interaction, /editingRequest \? planReply\('cancelled'\) : \(editingRequest = true\)/);
  assert.match(interaction, />취소<\/button>/);
  assert.match(interaction, /onclick=\{\(\) => planReply\('abandoned'\)\}/);
  assert.match(interaction, /\{#if editingRequest\}[\s\S]*?수정 의견/);
});
