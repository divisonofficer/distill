import { ApiError, getJson, postJson } from './api';
import type { ToolItem } from './session.svelte';
import type { BoardRow } from './types';

export interface AgentExecution {
 subagentId: string;
 parentSessionId: string;
 childSessionId: string;
 toolCallId?: string;
 subagentType: string;
 description: string;
 startedAtEpochMs: number;
 durationMs: number;
 status: string;
 turnCount?: number;
 toolCallCount?: number;
 tokensUsed?: number;
 contextWindowTokens?: number;
 toolsUsed?: string[];
 errorCount?: number;
 output?: string;
 toolCalls?: number;
 turns?: number;
 worktreePath?: string;
 failureError?: string;
 cancelReason?: string;
}

export interface CommandExecution {
 taskId: string;
 command: string;
 description?: string;
 canStop?: boolean;
 cwd: string;
 startedAtUnixMs: number;
 endedAtUnixMs: number | null;
 output: string;
 outputFile: string;
 truncated: boolean;
 exitCode: number | null;
 signal: string | null;
 completed: boolean;
 kind: 'bash' | 'monitor';
 outputTotalBytes: number;
}

type StopOutcome = 'cancelled' | 'already_finished' | 'killed' | 'already_exited' | 'not_found';
const active = (status: string) => status === 'running' || status === 'initializing';
const statuses = new Set(['running', 'initializing', 'completed', 'failed', 'cancelled']);

export type ChildExecutionState = 'running' | 'blocked' | 'done' | 'failed' | 'stopped' | 'unknown';

export function childNeedsInput(agent: AgentExecution, childSessions: BoardRow[], parentSessionId: string): boolean {
 return agent.parentSessionId === parentSessionId && childSessions.some(row => row.sessionId === agent.childSessionId && row.parentSessionId === parentSessionId && row.activity === 'needs_input');
}

export function childExecutionState(status: unknown, fresh: boolean, needsInput = false): ChildExecutionState {
 if (status === 'completed') return 'done';
 if (status === 'failed') return 'failed';
 if (status === 'cancelled') return 'stopped';
 if (status === 'running' || status === 'initializing') return needsInput ? 'blocked' : fresh ? 'running' : 'unknown';
 return 'unknown';
}

export function matchChildExecution(tool: ToolItem, agents: AgentExecution[], parentSessionId: string): AgentExecution | null {
 if (!/(^|[_:/-])spawn_subagent$/.test((tool.name ?? '').toLowerCase())) return null;
 const matches = agents.filter(agent => agent.parentSessionId === parentSessionId && agent.toolCallId === tool.id);
 return matches.length === 1 ? matches[0] : null;
}
const number = (value: unknown): value is number => typeof value === 'number' && Number.isFinite(value) && value >= 0;
const text = (value: unknown): value is string => typeof value === 'string';
const id = (value: unknown): value is string => text(value) && value.length > 0;

function agentValid(value: AgentExecution, parent: string): boolean {
 return !!value && id(value.subagentId) && value.parentSessionId === parent && id(value.childSessionId)
  && text(value.subagentType) && text(value.description) && statuses.has(value.status)
  && (value.toolCallId === undefined || id(value.toolCallId))
  && number(value.startedAtEpochMs) && number(value.durationMs);
}
function taskValid(value: CommandExecution): boolean {
 return !!value && id(value.taskId) && text(value.command) && text(value.cwd) && number(value.startedAtUnixMs)
  && (value.endedAtUnixMs === null || number(value.endedAtUnixMs)) && typeof value.completed === 'boolean'
  && text(value.output) && text(value.outputFile) && typeof value.truncated === 'boolean'
  && (value.exitCode === null || (typeof value.exitCode === 'number' && Number.isFinite(value.exitCode)))
  && (value.signal === null || text(value.signal)) && (value.kind === 'bash' || value.kind === 'monitor') && number(value.outputTotalBytes)
  && (value.canStop === undefined || typeof value.canStop === 'boolean') && (value.description === undefined || text(value.description));
}
function outcome(value: unknown, allowed: string[]): StopOutcome {
 const raw = (value as { outcome?: unknown } | null)?.outcome;
 const result = typeof raw === 'string' ? raw : (raw as { kind?: unknown } | null)?.kind;
 if (typeof result !== 'string' || !allowed.includes(result)) throw new Error('Invalid stop acknowledgment');
 return result as StopOutcome;
}

export class Executions {
 agents = $state<AgentExecution[]>([]);
 tasks = $state<CommandExecution[]>([]);
 agentUnavailable = $state(false);
 taskUnavailable = $state(false);
 agentLoaded = $state(false);
 taskLoaded = $state(false);
 agentError = $state<string | null>(null);
 taskError = $state<string | null>(null);
 agentFresh = $state<Record<string, boolean>>({});
 agentSnapshotErrors = $state<Record<string, string>>({});
 tasksFresh = $state(false);
 pendingAgentStops = $state<string[]>([]);
 pendingTaskStops = $state<string[]>([]);
 agentStopErrors = $state<Record<string, string>>({});
 taskStopErrors = $state<Record<string, string>>({});
 #host: string;
 #sessionId: string;
 #generation = 0;
 #timer: ReturnType<typeof setInterval> | undefined;
 #agentPoll: Promise<void> | undefined;
 #taskPoll: Promise<void> | undefined;

 constructor(host: string, sessionId: string) { this.#host = host; this.#sessionId = sessionId; }
 start(intervalMs = 2000): void {
  this.stop();
  const generation = this.#generation;
  this.#timer = setInterval(() => { void this.refresh(generation); }, intervalMs);
  void this.refresh(generation);
 }
 stop(): void {
  this.#generation++;
  if (this.#timer !== undefined) clearInterval(this.#timer);
  this.#timer = undefined;
  this.#agentPoll = undefined;
  this.#taskPoll = undefined;
  this.agentFresh = {};
  this.tasksFresh = false;
  this.pendingAgentStops = [];
  this.pendingTaskStops = [];
 }
 async refresh(generation = this.#generation): Promise<void> {
  if (generation !== this.#generation) return;
  if (!this.agentUnavailable && !this.#agentPoll) {
   const poll = this.#refreshAgents(generation);
   this.#agentPoll = poll;
   void poll.finally(() => { if (generation === this.#generation && this.#agentPoll === poll) this.#agentPoll = undefined; });
  }
  if (!this.taskUnavailable && !this.#taskPoll) {
   const poll = this.#refreshTasks(generation);
   this.#taskPoll = poll;
   void poll.finally(() => { if (generation === this.#generation && this.#taskPoll === poll) this.#taskPoll = undefined; });
  }
  await Promise.all([this.#agentPoll, this.#taskPoll]);
 }
 async retry(): Promise<void> {
  this.agentUnavailable = false; this.taskUnavailable = false;
  await this.refresh();
 }
 async #refreshAgents(generation: number): Promise<void> {
  const root = `/api/sessions/${encodeURIComponent(this.#sessionId)}`;
  this.agentFresh = {};
  try {
   const result = await getJson<{ subagents: AgentExecution[] }>(`${root}/subagents`, this.#host);
   if (generation !== this.#generation) return;
   if (!result || !Array.isArray(result.subagents) || !result.subagents.every((a) => agentValid(a, this.#sessionId))
    || new Set(result.subagents.map((a) => a.subagentId)).size !== result.subagents.length) throw new Error('Invalid subagent registry');
   const byId = new Map(this.agents.map((a) => [a.subagentId, a]));
   const fresh: Record<string, boolean> = {};
   const errors: Record<string, string> = {};
   for (const a of result.subagents) {
    const previous = byId.get(a.subagentId);
    byId.set(a.subagentId, previous && !active(previous.status) ? previous : a);
    fresh[a.subagentId] = true;
   }
   const present = new Set(result.subagents.map((a) => a.subagentId));
   await Promise.all([...byId.values()].filter((a) => !present.has(a.subagentId) && active(a.status)).map(async (previous) => {
    try {
     const response = await getJson<{ snapshot: AgentExecution }>(`${root}/subagents/${encodeURIComponent(previous.subagentId)}`, this.#host);
     if (generation !== this.#generation) return;
     if (!response || !agentValid(response.snapshot, this.#sessionId) || response.snapshot.subagentId !== previous.subagentId) throw new Error('Invalid subagent snapshot');
     byId.set(previous.subagentId, response.snapshot);
     fresh[previous.subagentId] = true;
    } catch (error) { if (generation === this.#generation) errors[previous.subagentId] = String(error); }
   }));
   if (generation !== this.#generation) return;
   this.agentLoaded = true; this.agentUnavailable = false;
   this.agents = [...byId.values()]; this.agentFresh = fresh; this.agentSnapshotErrors = errors;
   this.agentError = Object.keys(errors).length ? 'One or more agent snapshots are unavailable' : null;
  } catch (error) { if (generation === this.#generation) { this.agentLoaded = true; this.agentUnavailable = error instanceof ApiError && error.status === 404 && error.detail === '' && error.body === ''; this.agentError = this.agentUnavailable ? null : String(error); } }
 }
 async #refreshTasks(generation: number): Promise<void> {
  this.tasksFresh = false;
  try {
   const result = await getJson<{ tasks: CommandExecution[] }>(`/api/sessions/${encodeURIComponent(this.#sessionId)}/tasks`, this.#host);
   if (generation !== this.#generation) return;
   if (!result || !Array.isArray(result.tasks) || !result.tasks.every(taskValid)
    || new Set(result.tasks.map((t) => t.taskId)).size !== result.tasks.length) throw new Error('Invalid task registry');
   this.taskLoaded = true; this.taskUnavailable = false;
   this.tasks = result.tasks; this.tasksFresh = true; this.taskError = null;
  } catch (error) { if (generation === this.#generation) { this.taskLoaded = true; this.taskUnavailable = error instanceof ApiError && error.status === 404 && error.detail === '' && error.body === ''; this.taskError = this.taskUnavailable ? null : String(error); } }
 }
 async stopAgent(id: string): Promise<StopOutcome | null> {
  if (!this.agentFresh[id] || !this.agents.some((a) => a.subagentId === id && active(a.status)) || this.pendingAgentStops.includes(id)) return null;
  const generation = this.#generation;
  this.pendingAgentStops = [...this.pendingAgentStops, id];
  this.agentStopErrors = { ...this.agentStopErrors, [id]: '' };
  try {
   const result = await postJson<unknown>(`/api/sessions/${encodeURIComponent(this.#sessionId)}/subagents/${encodeURIComponent(id)}/cancel`, {}, this.#host);
   if (generation !== this.#generation) return null;
   const ack = outcome(result, ['cancelled', 'already_finished', 'not_found']);
   if (ack !== 'cancelled') this.agentStopErrors = { ...this.agentStopErrors, [id]: ack };
   const pending = this.#agentPoll;
   if (pending) await pending;
   if (generation !== this.#generation) return null;
   await this.refresh(generation);
   return generation === this.#generation ? ack : null;
  } catch (error) { if (generation === this.#generation) this.agentStopErrors = { ...this.agentStopErrors, [id]: String(error) }; return null; }
  finally { if (generation === this.#generation) this.pendingAgentStops = this.pendingAgentStops.filter((value) => value !== id); }
 }
 async stopTask(id: string): Promise<StopOutcome | null> {
  if (!this.tasksFresh || !this.tasks.some((t) => t.taskId === id && !t.completed && t.canStop === true) || this.pendingTaskStops.includes(id)) return null;
  const generation = this.#generation;
  this.pendingTaskStops = [...this.pendingTaskStops, id];
  this.taskStopErrors = { ...this.taskStopErrors, [id]: '' };
  try {
   const result = await postJson<unknown>(`/api/sessions/${encodeURIComponent(this.#sessionId)}/tasks/${encodeURIComponent(id)}/stop`, {}, this.#host);
   if (generation !== this.#generation) return null;
   const ack = outcome(result, ['killed', 'already_exited', 'not_found']);
   if (ack !== 'killed') this.taskStopErrors = { ...this.taskStopErrors, [id]: ack };
   const pending = this.#taskPoll;
   if (pending) await pending;
   if (generation !== this.#generation) return null;
   await this.refresh(generation);
   return generation === this.#generation ? ack : null;
  } catch (error) { if (generation === this.#generation) this.taskStopErrors = { ...this.taskStopErrors, [id]: String(error) }; return null; }
  finally { if (generation === this.#generation) this.pendingTaskStops = this.pendingTaskStops.filter((value) => value !== id); }
 }
}
