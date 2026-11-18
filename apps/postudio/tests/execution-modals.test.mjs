import test, { before, after } from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'vite';
import { fileURLToPath } from 'node:url';
import { resolve } from 'node:path';

const root = fileURLToPath(new URL('..', import.meta.url));
let vite, render, Agent, Command, ToolRow, Monitor, commandStatus;
before(async () => {
 vite = await createServer({ root, configFile: resolve(root, 'vite.config.ts'), server: { middlewareMode: true }, appType: 'custom', logLevel: 'silent' });
 ({ render } = await vite.ssrLoadModule('svelte/server'));
 ({ default: Agent } = await vite.ssrLoadModule('/src/lib/components/AgentDetailModal.svelte'));
 ({ default: Command } = await vite.ssrLoadModule('/src/lib/components/CommandOutputModal.svelte'));
 ({ default: Monitor } = await vite.ssrLoadModule('/src/lib/components/ExecutionMonitor.svelte'));
 ({ default: ToolRow } = await vite.ssrLoadModule('/src/lib/components/ToolRow.svelte'));
 ({ commandStatus } = await vite.ssrLoadModule('/src/lib/command-status.ts'));
});
after(async () => { await vite?.close(); });
const agent = { subagentId: 'a', parentSessionId: 'p', childSessionId: 'c', subagentType: 'research', description: '조사', startedAtEpochMs: 1000, durationMs: 3000, status: 'completed', output: '<script>bad</script>' };
const task = { taskId: 't', command: 'echo test', cwd: '/tmp', startedAtUnixMs: 1000, endedAtUnixMs: 5000, output: '<unsafe>', outputFile: '/tmp/out', truncated: true, exitCode: 2, signal: null, completed: true, kind: 'bash', outputTotalBytes: 50 };
test('agent detail shows frozen actual duration and escaped result', () => {
 const html = render(Agent, { props: { agent, now: 100000, fresh: true, onclose() {} } }).body;
 assert.match(html, /조사/); assert.match(html, /완료/); assert.match(html, /3초/);
 assert.match(html, /&lt;script>bad&lt;\/script>/); assert.doesNotMatch(html, /에이전트 중지/);
});
test('stale agent displays unknown status, hides stale duration, and disables stop', () => {
 const html = render(Agent, { props: { agent: { ...agent, status: 'running' }, now: 100000, fresh: false, onclose() {}, onstop() {} } }).body;
 assert.match(html, /상태 확인 중/); assert.match(html, /disabled/); assert.doesNotMatch(html, /3초|실행 시간/);
});

test('agent running and failed states use real execution data', () => {
 const running = render(Agent, { props: { agent: { ...agent, status: 'running' }, now: 10000, fresh: true, onclose() {}, onstop() {} } }).body;
 assert.match(running, /실행 중/); assert.match(running, /9초/); assert.match(running, />중지</);
 const failed = render(Agent, { props: { agent: { ...agent, status: 'failed', durationMs: 7000, failureError: 'worker exited' }, now: 10000, onclose() {}, onstop() {} } }).body;
 assert.match(failed, /실패/); assert.match(failed, /worker exited/); assert.match(failed, /7초/); assert.doesNotMatch(failed, />중지</);
});
test('command detail renders terminal data and truncation', () => {
 const html = render(Command, { props: { task, now: 100000, fresh: true, onclose() {}, onstop() {} } }).body;
 assert.match(html, /echo test/); assert.match(html, /4초/); assert.match(html, /출력이 잘렸어요/); assert.match(html, /&lt;unsafe>/); assert.doesNotMatch(html, /명령 중지/);
});
test('running, stale, and failed command states stay truthful', () => {
 const runningTask = { ...task, completed: false, endedAtUnixMs: null, exitCode: null, canStop: true };
 const running = render(Command, { props: { task: runningTask, fresh: true, now: 10000, onclose() {}, onstop() {} } }).body;
 assert.match(running, /실행 중/); assert.match(running, /9초/); assert.match(running, />중지</);
 const unknown = render(Command, { props: { task: runningTask, fresh: false, now: 10000, onclose() {}, onstop() {} } }).body;
 assert.match(unknown, /알 수 없음/); assert.match(unknown, /disabled/); assert.doesNotMatch(unknown, /실행 시간|9초/);
 const failed = render(Command, { props: { task, fresh: true, now: 10000, onclose() {} } }).body;
 assert.match(failed, /실패/); assert.match(failed, /종료 코드/); assert.doesNotMatch(failed, />중지</);
});

test('command signal completion never renders ordinary completion', () => {
 const stopped = render(Command, { props: { task: { ...task, signal: 'SIGTERM', exitCode: 0 }, fresh: true, now: 10000, onclose() {} } }).body;
 assert.match(stopped, /중지됨/); assert.doesNotMatch(stopped, /상태[^<]*완료|>완료</);
 const unknownSignal = render(Command, { props: { task: { ...task, signal: 'SIGSEGV', exitCode: 0 }, fresh: true, now: 10000, onclose() {} } }).body;
 assert.match(unknownSignal, /알 수 없음/); assert.doesNotMatch(unknownSignal, />완료</);
});

test('completed command without end time never gains an invented duration', () => {
 const props = { task: { ...task, endedAtUnixMs: null }, fresh: true, onclose() {} };
 const before = render(Command, { props: { ...props, now: 100000 } }).body;
 const after = render(Command, { props: { ...props, now: 110000 } }).body;
 assert.equal(before.replace(/<!--[\s\S]*?-->/g, ''), after.replace(/<!--[\s\S]*?-->/g, ''));
 assert.doesNotMatch(before, /정보 없음|실행 시간|99초|109초/);
});
test('completed command signals and cancellation never show ordinary completion', () => {
 const row = (text) => render(ToolRow, { props: { item: { kind: 'tool', key: 5, id: 'command', name: 'run_terminal_command', toolKind: 'execute', status: 'completed', input: { command: 'true' }, content: [{ type: 'text', text }] } } }).body;
 const signalled = row('exit: 0\nsignal: SIGTERM');
 assert.match(signalled, /신호 종료/); assert.doesNotMatch(signalled, /aria-label="완료"|>✓</);
 const cancelled = row('exit: 0\ncancelled: true');
 assert.match(cancelled, /중지됨/); assert.doesNotMatch(cancelled, /aria-label="완료"|>✓</);
 const unknown = row('exit: -1');
 assert.match(unknown, /상태 확인 중/); assert.doesNotMatch(unknown, /aria-label="완료"|>✓</);
});

test('tool history is read-only with no invented execution time', () => {
 const history = { kind: 'tool', key: 1, id: 'x', toolKind: 'execute', input: { command: 'pwd' }, content: [{ type: 'text', text: 'output' }] };
 const html = render(Command, { props: { history, now: 0, onclose() {} } }).body;
 assert.match(html, /읽기 전용 도구 기록/); assert.match(html, /pwd/); assert.doesNotMatch(html, /실행 시간|중지/);
 const row = render(ToolRow, { props: { item: history, onselect() {}, onoutput() {} } }).body;
 assert.match(row, /상세 탭 열기/); assert.match(row, /출력 보기/);
});

test('subagent tool cards show real role and description without RPC completion as child state', () => {
 const input = { subagent_type: 'research', description: 'Review the parser changes' };
 const running = render(ToolRow, { props: { item: { kind: 'tool', key: 1, id: 'spawn', name: 'spawn_subagent', status: 'in_progress', input } } }).body;
 const runningSummary = running.slice(running.indexOf('<summary'), running.indexOf('</summary>'));
 assert.match(runningSummary, /서브에이전트/); assert.match(runningSummary, /research/); assert.match(runningSummary, /Review the parser changes/);
 assert.match(runningSummary, /요청 중/); assert.doesNotMatch(runningSummary, /실행 중|완료|✓/);
 const completed = render(ToolRow, { props: { item: { kind: 'tool', key: 2, id: 'spawn-done', name: 'spawn_subagent', status: 'completed', input } } }).body;
 const completedSummary = completed.slice(completed.indexOf('<summary'), completed.indexOf('</summary>'));
 assert.doesNotMatch(completedSummary, /완료|✓|실패|실행 중/);
 const failed = render(ToolRow, { props: { item: { kind: 'tool', key: 3, id: 'spawn-failed', name: 'spawn_subagent', status: 'failed', input } } }).body;
 assert.match(failed.slice(failed.indexOf('<summary'), failed.indexOf('</summary>')), /호출 실패/);
});

test('tool row shows child state from execution metadata and only real needs-input activity blocks', () => {
 const renderState = (status, fresh = true, childNeedsInput = false) => render(ToolRow, { props: {
  item: { kind: 'tool', key: 4, id: `call-${status}`, name: 'spawn_subagent', status: 'completed', input: { subagent_type: agent.subagentType, description: agent.description } },
  childExecution: { ...agent, status }, childExecutionFresh: fresh, childNeedsInput
 } }).body;
 for (const [status, label] of [['running', '실행 중'], ['completed', '완료'], ['failed', '실패'], ['cancelled', '중지됨']]) assert.match(renderState(status), new RegExp(label));
 assert.match(renderState('running', true, true), /확인 필요/);
 assert.match(renderState('running', false), /상태 확인 중/);
});

test('monitor initially renders compact truthful loading state', () => {
 const html = render(Monitor, { props: { host: 'http://localhost', sessionId: 's' } }).body;
 assert.match(html, /불러오고 있어요/); assert.match(html, /aria-expanded="false"/);
 assert.doesNotMatch(html, /관찰된 에이전트가 없습니다|실행 중 0개|현재 세션에서 관찰한/);
});

test('history output shows reported failure and empty output honestly', () => {
 const history = { kind: 'tool', key: 1, id: 'x', status: 'completed', content: [{ type: 'text', text: 'exit: 2\nfailed' }] };
 const failed = render(Command, { props: { history, now: 0, onclose() {} } }).body;
 assert.match(failed, /알 수 없음/); assert.doesNotMatch(failed, /종료 코드/); assert.match(failed, /failed/);
 const empty = render(Command, { props: { history: { ...history, status: 'pending', content: [] }, now: 0, onclose() {} } }).body;
 assert.match(empty, /대기/); assert.match(empty, /출력이 없어요/);
});

 test('command status reflects structured exit and stop signals', () => {
 const cases = [
  [{ exitCode: 0, signal: null }, '완료'],
  [{ exitCode: 2, signal: null }, '실패'],
  [{ exitCode: null, signal: 'SIGTERM' }, '중지됨'],
  [{ exitCode: -1, signal: 'signal 9' }, '중지됨'],
  [{ exitCode: null, signal: 'timeout' }, '중지됨'],
  [{ exitCode: null, signal: 'gateway-lost' }, '알 수 없음'],
  [{ exitCode: null, signal: null }, '알 수 없음'],
  [{ exitCode: -1, signal: null }, '알 수 없음']
 ];
 for (const [result, expected] of cases) {
  const item = { ...task, ...result };
  assert.equal(commandStatus(item, true).label, expected);
  const html = render(Command, { props: { task: item, fresh: true, now: 10000, onclose() {} } }).body;
  assert.match(html, new RegExp(`>${expected}<`));
 }
 });
