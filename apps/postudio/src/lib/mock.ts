// Dev-only. Loaded only when VITE_MOCK=1 (see api.ts).
import type { BoardRow, ClientMsg, Mode, NewSession, Sep } from './types';
import type { Socket, SocketHandlers } from './api';

const fixtures = import.meta.glob('../../../../crates/custom/distill-node/tests/fixtures/sep/*.json', {
	import: 'default'
}) as Record<string, () => Promise<Sep[]>>;

const PUBLIC_KEY = 'BFNrxpMZ64KkLGGnifDUlXnBag_E_RUuz6jogN-VOAEI3gsyP7sRX0TdTAfi7T8SZEWulbsbjgh5uwgatdHeSBQ';
const REPLY = [
	'Plan:',
	'',
	'- read `main.rs`',
	'- edit the parser',
	'- run the tests',
	'',
	'```ts',
	'const total: number = 1 + 2;',
	'```',
	'',
	'Done. **All green.**'
].join('\n');

interface Pending {
	id: unknown;
	toolCallId: string;
	/** resolves when the client answers */
	resolve: () => void;
}

interface MockSession {
	row: BoardRow;
	mode: Mode;
	events: Sep[];
	answered: Set<string>;
	pending: Map<string, Pending>;
	turn: number;
}

const sessions = new Map<string, MockSession>();
const boardListeners = new Set<(sessions: BoardRow[]) => void>();
let created = 0;
let loaded: Promise<void> | null = null;

const sleep = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));
const keyOf = (id: unknown) => JSON.stringify(id);

function newSession(id: string, events: Sep[], cwd = '/sandbox/proj'): MockSession {
	const title = events.find((e): e is Extract<Sep, { t: 'session.title' }> => e.t === 'session.title');
	return {
		row: {
			sessionId: id,
			cwd,
			title: title?.title ?? '',
			activity: 'idle',
			resident: true,
			yolo: false,
			modelId: 'mock-model',
			isWorktree: false,
			lastChangeUnixMs: Date.now()
		},
		mode: 'ask',
		events,
		answered: new Set(),
		pending: new Map(),
		turn: 0
	};
}

function loadAll(): Promise<void> {
	loaded ??= (async () => {
		for (const [path, load] of Object.entries(fixtures)) {
			const id = path.replace(/^.*\//, '').replace(/\.json$/, '');
			sessions.set(id, newSession(id, await load()));
		}
	})();
	return loaded;
}

function publishRow(s: MockSession): void {
	const update = [rowOf(s)];
	for (const listener of boardListeners) listener(update);
}

function rowOf(s: MockSession): BoardRow {
	const last = [...s.events].reverse().find((e) => e.t === 'session.state');
	const awaiting = s.events.some((e) => e.t === 'permission.request' && !s.answered.has(e.tool_call_id));
	let activity = 'idle';
	if (awaiting) activity = 'needs_input';
	else if (last?.t === 'session.state') {
		const map: Record<string, string> = { running: 'working', idle: 'idle', error: 'error' };
		activity = map[last.status] ?? 'idle';
	}
	return { ...s.row, activity, yolo: s.mode === 'always-approve', lastChangeUnixMs: Date.now() };
}

function turnEvents(text: string): Sep[] {
	const out: Sep[] = [
		{ t: 'session.state', status: 'running' },
		{ t: 'user.message', text },
		{ t: 'usage', used: 2700, size: 256000 }
	];
	for (let i = 0; i < REPLY.length; i += 24) out.push({ t: 'message.delta', text: REPLY.slice(i, i + 24) });
	out.push({ t: 'turn.end', stop_reason: 'end_turn' }, { t: 'session.state', status: 'idle' });
	return out;
}

export async function mockFetch(method: string, path: string, body?: unknown): Promise<unknown> {
	await loadAll();
	if (method === 'GET' && path === '/api/node') return { host: 'mock-host', version: '0.0.0-mock' };
	if (method === 'GET' && path === '/api/sessions') return { sessions: [...sessions.values()].map(rowOf) };
	if (method === 'GET' && path === '/api/push/key') return { publicKey: PUBLIC_KEY };
	if (method === 'POST' && (path === '/api/push/subscribe' || path === '/api/push/unsubscribe')) return undefined;
	if (method === 'POST' && path === '/api/sessions') {
		const b = body as NewSession;
		const id = `mock-${++created}`;
		const s = newSession(id, [], b.cwd);
		s.row.isWorktree = !!b.worktree;
		s.mode = b.mode ?? 'ask';
		if (b.prompt) s.events.push(...turnEvents(b.prompt));
		sessions.set(id, s);
		return { sessionId: id };
	}
	throw new Error(`mock: no route ${method} ${path}`);
}

/** Replays stored events; a pending permission stops replay until the client answers it. */
async function replay(s: MockSession, emit: (m: Sep) => void, alive: () => boolean): Promise<void> {
	const events = [...s.events];
	let ended = false;
	for (const ev of events) {
		if (!alive()) return;
		if (ev.t === 'session.state' && ev.status === 'waiting_permission') continue;
		if (ev.t === 'permission.resolved') {
			if (s.answered.has(ev.tool_call_id)) emit(ev);
			publishRow(s);
			continue;
		}
		if (ev.t === 'permission.request') {
			emit(ev);
			publishRow(s);
			if (s.answered.has(ev.tool_call_id)) continue;
			if (!ended) {
				ended = true;
				emit({ t: 'replay.end' });
			}
			emit({ t: 'session.state', status: 'waiting_permission' });
			await new Promise<void>((res) => {
				s.pending.set(keyOf(ev.id), { id: ev.id, toolCallId: ev.tool_call_id, resolve: res });
			});
			if (!alive()) return;
			continue;
		}
		emit(ev);
		await sleep(60);
	}
	if (!ended) emit({ t: 'replay.end' });
}

async function runTurn(s: MockSession, emit: (m: Sep) => void, text: string, alive: () => boolean): Promise<void> {
	const turn = ++s.turn;
	for (const ev of turnEvents(text)) {
		if (!alive() || s.turn !== turn) return;
		s.events.push(ev);
		emit(ev);
		if (ev.t === 'session.state') publishRow(s);
		await sleep(40);
	}
}

export function mockSocket<M>(path: string, h: SocketHandlers<M>): Socket {
	let closed = false;
	const alive = () => !closed;
	const emit = (m: unknown) => {
		if (!closed) h.onMessage(m as M);
	};

	if (path === '/ws/board') {
		const publish = (sessions: BoardRow[]) => emit({ t: 'board.upsert', sessions });
		boardListeners.add(publish);
		void (async () => {
			await loadAll();
			await sleep(50);
			if (closed) return;
			h.onOpen();
			emit({ t: 'board.upsert', sessions: [...sessions.values()].map(rowOf) });
		})();
		return {
			send() {},
			close: () => {
				closed = true;
				boardListeners.delete(publish);
			}
		};
	}

	const id = path.replace('/ws/session/', '');
	const emitSep = (m: Sep) => emit(m);
	void (async () => {
		await loadAll();
		await sleep(50);
		if (closed) return;
		h.onOpen();
		const s = sessions.get(id);
		if (!s) {
			emit({ t: 'error', message: '세션을 찾을 수 없습니다.' });
			emit({ t: 'replay.end' });
			return;
		}
		await replay(s, emitSep, alive);
	})();

	return {
		send(msg: ClientMsg) {
			const s = sessions.get(id);
			if (!s || closed) return;
			switch (msg.t) {
				case 'prompt':
					void runTurn(s, emitSep, msg.text, alive);
					break;
				case 'cancel':
					s.turn++;
					emitSep({ t: 'turn.end', stop_reason: 'cancelled' });
					emitSep({ t: 'session.state', status: 'idle' });
					break;
				case 'permission': {
					// replay() emits the fixture's permission.resolved and running states after this
					const p = s.pending.get(keyOf(msg.id));
					if (!p) break;
					s.pending.delete(keyOf(msg.id));
					s.answered.add(p.toolCallId);
					publishRow(s);
					p.resolve();
					break;
				}
				case 'mode':
					s.mode = msg.mode;
					break;
			}
		},
		close() {
			closed = true;
		}
	};
}
