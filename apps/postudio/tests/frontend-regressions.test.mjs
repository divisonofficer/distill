import assert from 'node:assert/strict';
import { after, before, test } from 'node:test';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createServer } from 'vite';

const root = fileURLToPath(new URL('..', import.meta.url));
let vite;
let SessionView;
let connect;
let postJson;

before(async () => {
	vite = await createServer({
root,
		configFile: resolve(root, 'vite.config.ts'),
		server: { middlewareMode: true },
		appType: 'custom',
		logLevel: 'silent'
	});
	({ SessionView } = await vite.ssrLoadModule('/src/lib/session.svelte.ts'));
	({ connect, postJson } = await vite.ssrLoadModule('/src/lib/api.ts'));
});

after(async () => {
	await vite?.close();
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

test('new session page disables unsupported worktree creation', async () => {
	const restore = overrideGlobals({ localStorage: { getItem: () => null, setItem: () => {} } });
	try {
		const [{ default: NewPage }, { render }] = await Promise.all([
			vite.ssrLoadModule('/src/routes/new/+page.svelte'),
			vite.ssrLoadModule('svelte/server')
		]);
		const { body } = render(NewPage);
		const checkbox = body.match(/<input type="checkbox"[^>]*>/)?.[0];
		assert.ok(checkbox);
		assert.match(checkbox, /disabled=""/);
		assert.doesNotMatch(checkbox, /checked=/);
		assert.match(body, /서버가 아직 worktree 격리를 지원하지 않습니다/);
	} finally {
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
		timers[0].callback();
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
