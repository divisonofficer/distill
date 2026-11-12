import assert from 'node:assert/strict';
import { after, before, test } from 'node:test';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createServer } from 'vite';

const root = fileURLToPath(new URL('..', import.meta.url));
let vite;
let SessionReplay;
let SessionView;
let groupCompletedTools;
let restoredHistoryScrollTop;
let connect;
let postJson;
let attachments;

before(async () => {
	vite = await createServer({
root,
		configFile: resolve(root, 'vite.config.ts'),
		server: { middlewareMode: true },
		appType: 'custom',
		logLevel: 'silent'
	});
	({ SessionReplay, SessionView, groupCompletedTools, restoredHistoryScrollTop } = await vite.ssrLoadModule('/src/lib/session.svelte.ts'));
		({ connect, postJson } = await vite.ssrLoadModule('/src/lib/api.ts'));
	attachments = await vite.ssrLoadModule('/src/lib/attachments.ts');
});

after(async () => {
	await vite?.close();
});

test('initial history page applies before buffered live transcript and keeps live approvals', () => {
const view = new SessionView();
const replay = new SessionReplay(view);
replay.begin();
assert.equal(replay.apply({ t: 'message.delta', text: 'live' }), false);
assert.equal(view.items.length, 0);
replay.apply({ t: 'permission.request', id: 7, tool_call_id: 'tool-1', title: 'Approve', options: [{ id: 'yes', kind: 'allow_once', name: 'Allow' }] });
assert.equal(view.permissions.length, 1);
replay.apply({ t: 'history.page', items: [{ t: 'user.message', text: 'old' }], cursor: null, has_more: false, initial: true });
assert.deepEqual(view.items.map((item) => item.text), ['old', 'live']);
assert.equal(view.permissions.length, 1);
assert.equal(replay.apply({ t: 'replay.end' }), true);
});

test('older history prepends unique stable items and restores scroll anchor', () => {
const view = new SessionView();
view.apply({ t: 'message.delta', text: 'latest' });
view.apply({ t: 'tool', id: 'live-tool', status: 'in_progress' });
const priorKeys = view.items.map((item) => item.key);
view.applyHistoryPage([
{ t: 'user.message', text: 'older' },
{ t: 'tool', id: 'old-tool', status: 'completed' }
], false);
assert.deepEqual(view.items.map((item) => item.kind === 'text' ? item.text : item.id), ['older', 'old-tool', 'latest', 'live-tool']);
assert.deepEqual(view.items.slice(-2).map((item) => item.key), priorKeys);
assert.equal(new Set(view.items.map((item) => item.key)).size, view.items.length);
assert.equal(restoredHistoryScrollTop({ scrollTop: 80, scrollHeight: 1000 }, 1400), 480);
});

test('pasted text threshold and attachment size limits keep inclusive boundaries', async () => {
	const {
		MAX_ATTACHMENTS,
		MAX_IMAGE_BYTES,
		MAX_TEXT_BYTES,
		createTextDraft,
		isLongPaste,
		readAttachmentFiles
	} = attachments;
	assert.equal(isLongPaste('x'.repeat(4000)), false);
	assert.equal(isLongPaste('x'.repeat(4001)), true);
	assert.equal(createTextDraft('boundary.txt', 'x'.repeat(MAX_TEXT_BYTES), 1).size, MAX_TEXT_BYTES);
	assert.throws(() => createTextDraft('too-large.txt', 'x'.repeat(MAX_TEXT_BYTES + 1), 1), /1 MiB/);

	class StubFileReader {
		readAsDataURL(file) {
			this.result = `data:${file.type};base64,AA==`;
			this.onload();
		}
	}
	const restore = overrideGlobals({ FileReader: StubFileReader });
	try {
		const accepted = await readAttachmentFiles(
			[{ name: 'boundary.png', type: 'image/png', size: MAX_IMAGE_BYTES }],
			[],
			0
		);
		assert.equal(accepted.attachments.length, 1);
		const rejected = await readAttachmentFiles(
			[{ name: 'too-large.png', type: 'image/png', size: MAX_IMAGE_BYTES + 1 }],
			[],
			0
		);
		assert.equal(rejected.attachments.length, 0);
		assert.match(rejected.error, /10 MiB/);
		const full = Array.from({ length: MAX_ATTACHMENTS }, (_, key) => ({ key, attachment: { kind: 'file', name: `${key}` }, size: 0 }));
		const limited = await readAttachmentFiles([{ name: 'extra.txt', size: 0 }], full, MAX_ATTACHMENTS);
		assert.equal(limited.attachments.length, MAX_ATTACHMENTS);
		assert.match(limited.error, /10개/);
	} finally {
		restore();
	}
});

test('SessionView replays separate attachment items without parsing image placeholder prose', () => {
	const view = new SessionView();
	view.apply({ t: 'user.message', text: '[Image #1]' });
	view.apply({ t: 'user.attachment', attachment: { kind: 'text', name: 'pasted-text-1.txt', text: 'original text' } });
	view.apply({ t: 'user.attachment', attachment: { kind: 'file', name: 'unavailable.pdf', uri: 'file:///private/file.pdf' } });
	assert.equal(view.items.length, 3);
	assert.deepEqual(view.items.map((item) => item.kind), ['text', 'attachment', 'attachment']);
	assert.deepEqual(view.items[1].attachment, { kind: 'text', name: 'pasted-text-1.txt', text: 'original text' });
	assert.deepEqual(view.items[2].attachment, { kind: 'file', name: 'unavailable.pdf', uri: 'file:///private/file.pdf' });
});

test('message deltas start fresh groups after turn end', () => {
	const view = new SessionView();
	view.apply({ t: 'message.delta', text: 'answer' });
	view.apply({ t: 'message.delta', text: ' continued' });
	view.apply({ t: 'turn.end', stop_reason: 'end_turn' });
	view.apply({ t: 'message.delta', text: 'next turn' });
	view.apply({ t: 'turn.end', stop_reason: 'end_turn' });
	view.apply({ t: 'reasoning.delta', text: 'thought' });
	view.apply({ t: 'turn.end', stop_reason: 'end_turn' });
	view.apply({ t: 'reasoning.delta', text: 'later thought' });

	assert.deepEqual(
		view.items.map(({ role, text }) => [role, text]),
		[
			['assistant', 'answer continued'],
			['assistant', 'next turn'],
			['reasoning', 'thought'],
			['reasoning', 'later thought']
		]
	);
});

test('terminal events clear permission cards without blocking later replay requests', () => {
	const request = {
		t: 'permission.request',
		id: 1,
		tool_call_id: 'call-1',
		title: 'Allow command',
		options: [{ id: 'allow', kind: 'allow_once', name: 'Allow' }]
	};
	const terminals = [
		{ t: 'turn.end', stop_reason: 'end_turn' },
		{ t: 'turn.end', stop_reason: 'error', error: 'request failed' },
				{ t: 'session.state', status: 'error' }
	];

	for (const terminal of terminals) {
		const view = new SessionView();
		view.apply(request);
		assert.equal(view.permissions.length, 1);
		view.apply(terminal);
		assert.equal(view.permissions.length, 0);
		view.apply(request);
		assert.equal(view.permissions.length, 1);
	}
});

test('postJson includes useful server error details', async () => {
	const originalFetch = globalThis.fetch;
	globalThis.fetch = async () =>
		new Response(JSON.stringify({ error: 'worktree is not supported yet' }), {
			status: 501,
			headers: { 'Content-Type': 'application/json' }
		});

	try {
		await assert.rejects(postJson('/api/sessions', {}), /\/api\/sessions: 501 worktree is not supported yet/);
	} finally {
		globalThis.fetch = originalFetch;
	}
});

test('new session page enables worktree creation with a required safe name', async () => {
	const restore = overrideGlobals({ localStorage: { getItem: () => null, setItem: () => {} } });
	try {
		const [{ default: NewPage }, { render }] = await Promise.all([
			vite.ssrLoadModule('/src/routes/new/+page.svelte'),
			vite.ssrLoadModule('svelte/server')
		]);
		const { body } = render(NewPage);
		const checkbox = body.match(/<input type="checkbox"[^>]*>/)?.[0];
		const menu = body.match(/<details\b[\s\S]*?<\/details>/)?.[0];
		assert.ok(checkbox);
		assert.doesNotMatch(checkbox, /checked=/);
		assert.ok(menu);
		const source = readFileSync(resolve(root, 'src/routes/new/+page.svelte'), 'utf8');
		assert.match(source, /worktree 이름/);
		assert.match(source, /required maxlength="64" pattern=\{'\[A-Za-z0-9_\\\\-\]\+'\}/);
		const patternSource = source.match(/pattern=\{'((?:\\\\.|[^'])*)'\}/)?.[1];
		assert.ok(patternSource);
		const pattern = new Function(`return '${patternSource}'`)();
		const worktreeName = new RegExp(`^(?:${pattern})$`, 'v');
		assert.match('feature-update_1', worktreeName);
		assert.doesNotMatch('bad/name', worktreeName);
		assert.doesNotMatch('bad name', worktreeName);
		assert.doesNotMatch('café', worktreeName);
		assert.doesNotMatch(menu, /서버가 아직 worktree 격리를 지원하지 않습니다/);
	} finally {
		restore();
	}
});

test('new session page selects environment before prompt and combines board and recent workspaces', async () => {
	const recent = ['/host/shared', '/host/recent', '/host/shared'];
	const restore = overrideGlobals({
		localStorage: {
			getItem: (key) => key === 'postudio.recentCwds' ? JSON.stringify(recent) : null,
			setItem: () => {}
		}
	});
	const { board } = await vite.ssrLoadModule('/src/lib/board.svelte.ts');
	const previousRows = board.rows;
	board.rows = [{ sessionId: 'board-only', cwd: '/host/from-board' }, { sessionId: 'shared', cwd: '/host/shared' }];
	try {
		const [{ default: NewPage }, { render }] = await Promise.all([
			vite.ssrLoadModule('/src/routes/new/+page.svelte'),
			vite.ssrLoadModule('svelte/server')
		]);
		const { body } = render(NewPage);
		const menu = body.match(/<details\b[\s\S]*?<\/details>/)?.[0];
		assert.ok(menu);
		assert.ok(body.indexOf('aria-label="호스트 선택"') < body.indexOf('id="prompt"'));
		assert.ok(body.indexOf('aria-label="워크스페이스 선택"') < body.indexOf('id="prompt"'));
		assert.match(body, /from-board/);
		assert.match(body, /\/host\/recent/);
		assert.equal((body.match(/title="\/host\/shared"/g) ?? []).length, 1);
		assert.equal((body.match(/title="\/host\/from-board"/g) ?? []).length, 1);
		assert.match(menu, /호스트의 절대 경로/);
		assert.match(menu, /연결된 호스트에서 사용합니다/);
		assert.doesNotMatch(body, /<datalist/);
		assert.doesNotMatch(menu, /<button[^>]*type="submit"/);
		assert.match(menu, /type="checkbox"/);
	} finally {
		board.rows = previousRows;
		restore();
	}
});

function overrideGlobals(values) {
	const previous = new Map();
	for (const [name, value] of Object.entries(values)) {
		previous.set(name, Object.getOwnPropertyDescriptor(globalThis, name));
		Object.defineProperty(globalThis, name, { configurable: true, writable: true, value });
	}
	return () => {
		for (const [name, descriptor] of previous) {
			if (descriptor) Object.defineProperty(globalThis, name, descriptor);
			else delete globalThis[name];
		}
	};
}

class StubWebSocket {
	static OPEN = 1;
	static instances = [];
	readyState = 0;
	onopen;
	onmessage;
	onclose;
	sent = [];

	constructor(url) {
		this.url = url;
		StubWebSocket.instances.push(this);
	}

	send(value) {
		if (this.readyState !== StubWebSocket.OPEN) throw new Error('socket is not open');
		this.sent.push(value);
	}

	close() {
		this.readyState = 3;
	}
}

test('socket send reports connection state and reconnect drops stale frames', () => {
	StubWebSocket.instances = [];
	const timers = [];
	const restore = overrideGlobals({
		WebSocket: StubWebSocket,
		location: { protocol: 'http:', host: 'localhost:5173' },
		setTimeout: (callback, delay) => {
			timers.push({ callback, delay });
			return timers.length;
		},
		clearTimeout: () => {}
	});
	let socket;
	const opened = [];
	const closed = [];
	const messages = [];

	try {
		socket = connect('/ws/session/test', {
			onOpen: () => opened.push(true),
			onClose: () => closed.push(true),
			onMessage: (message) => messages.push(message)
		});
		const first = StubWebSocket.instances[0];
		assert.equal(socket.send({ t: 'cancel' }), false);
		first.readyState = StubWebSocket.OPEN;
		first.onopen();
		assert.equal(socket.send({ t: 'cancel' }), true);
		assert.deepEqual(JSON.parse(first.sent[0]), { t: 'cancel' });

		first.readyState = 3;
		first.onclose();
		assert.equal(socket.send({ t: 'cancel' }), false);
		timers.find((timer) => timer.delay === 1000).callback();
		const second = StubWebSocket.instances[1];
		first.onmessage({ data: '{"stale":true}' });
		second.readyState = StubWebSocket.OPEN;
		second.onopen();
		second.onmessage({ data: '{"fresh":true}' });

		assert.deepEqual(messages, [{ fresh: true }]);
		assert.equal(opened.length, 2);
		assert.equal(closed.length, 1);
	} finally {
		socket?.close();
		restore();
	}
});

 test('permission mode mapping uses authoritative flags', async () => {
 const { modeFromRow } = await vite.ssrLoadModule('/src/lib/types.ts');
 assert.equal(modeFromRow(undefined), undefined);
 assert.equal(modeFromRow({yolo:false}), undefined);
 assert.equal(modeFromRow({yolo:false,autoMode:true}), 'auto');
 assert.equal(modeFromRow({yolo:false,autoMode:false}), 'ask');
 assert.equal(modeFromRow({yolo:true,autoMode:false}), 'always-approve');
 });
 test('recoverable reply errors preserve pending permission cards', () => {
 const view = new SessionView();
 view.apply({t:'permission.request',id:1,tool_call_id:'t',title:'Edit',options:[]});
 view.apply({t:'error',message:'invalid reply'});
 assert.equal(view.permissions.length, 1);
 view.apply({t:'session.state',status:'error'});
 assert.equal(view.permissions.length, 0);
 });

test('attachment reads merge without resurrecting removed drafts and enforce totals', async () => {
const { mergeDrafts, MAX_TOTAL_BYTES, clipboardFiles } = await vite.ssrLoadModule('/src/lib/attachments.ts');
const pasted = {key:2,attachment:{kind:'text',name:'pasted.txt',text:'keep'},size:4};
const incoming = {key:3,attachment:{kind:'image',mime:'image/png',data:'AA=='},size:8};
assert.deepEqual(mergeDrafts([pasted],[incoming]).attachments.map(a=>a.key),[2,3]);
assert.deepEqual(mergeDrafts([], [incoming]).attachments.map(a=>a.key),[3]);
const full = {...pasted,size:MAX_TOTAL_BYTES};
const rejected = mergeDrafts([full],[incoming]);
assert.equal(rejected.attachments.length,1);
assert.match(rejected.error,/20 MiB/);
const file = {name:'paste.png'};
assert.equal(clipboardFiles({files:[file],items:[{kind:'file',getAsFile:()=>({...file})}]}).length,1);
});

test('permission replays replace stale request IDs and expired replies remove only that card', () => {
const view = new SessionView();
const request = {t:'permission.request',id:1,tool_call_id:'tool',title:'command',options:[{id:'yes',kind:'allow_once',name:'Yes'}]};
view.apply(request);
view.apply({...request,id:2});
assert.equal(view.permissions.length,1);
assert.equal(view.permissions[0].id,2);
view.apply({t:'permission.expired',id:1});
assert.equal(view.permissions.length,1);
view.apply({t:'permission.expired',id:2});
assert.equal(view.permissions.length,0);
});


test('completed adjacent tools collapse together but errors and live tools stay separate', () => {
const view = new SessionView();
view.apply({ t: 'tool', id: 'one', status: 'completed', title: 'Read file' });
view.apply({ t: 'tool', id: 'two', status: 'completed', title: 'Search' });
view.apply({ t: 'tool', id: 'live', status: 'in_progress', title: 'Run command' });
view.apply({ t: 'tool', id: 'failed', status: 'failed', title: 'Write file' });
assert.deepEqual(groupCompletedTools(view.items).map((entry) => entry.kind === 'tools' ? entry.items.map((item) => item.id) : entry.item.kind === 'tool' ? [entry.item.id] : []), [['one', 'two'], ['live'], ['failed']]);
});

test('legacy replay without history.page still restores transcript at replay.end', () => {
const view = new SessionView();
const replay = new SessionReplay(view);
replay.begin();
replay.apply({ t: 'user.message', text: 'previous question' });
replay.apply({ t: 'message.delta', text: 'previous answer' });
assert.equal(replay.apply({ t: 'replay.end' }), true);
assert.deepEqual(view.items.map(item => item.text), ['previous question', 'previous answer']);
replay.apply({ t: 'message.delta', text: ' continuation' });
assert.equal(view.items.at(-1).text, 'previous answer continuation');
});

test('replay preserves turn boundaries and older tools retain historical fields with live status', () => {
const view = new SessionView();
const replay = new SessionReplay(view);
replay.begin();
replay.apply({ t: 'message.delta', text: 'A' });
replay.apply({ t: 'turn.end' });
replay.apply({ t: 'message.delta', text: 'B' });
replay.apply({ t: 'replay.end' });
assert.deepEqual(view.items.map(item => item.text), ['A', 'B']);
view.apply({ t: 'tool', id: 'old', status: 'completed' });
view.applyHistoryPage([{ t: 'tool', id: 'old', title: 'Original', status: 'pending', input: { command: 'pwd' } }], false);
assert.equal(view.items[0].title, 'Original');
assert.equal(view.items[0].status, 'completed');
assert.equal(view.items.filter(item => item.kind === 'tool').length, 1);
});
