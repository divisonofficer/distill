import type { ClientMsg } from './types';

// Dev-only mock transport. Vite replaces the flag with a constant, so the
// dynamic import and mock.ts are dropped from production builds.
const mock = import.meta.env.VITE_MOCK ? await import('./mock') : null;

export interface Socket {
	send(msg: ClientMsg): void;
	close(): void;
}

export interface SocketHandlers<M> {
	onOpen(): void;
	onClose(): void;
	onMessage(msg: M): void;
}

export async function getJson<T>(path: string): Promise<T> {
	if (mock) return (await mock.mockFetch('GET', path)) as T;
	const res = await fetch(path);
	if (!res.ok) throw new Error(`${path}: ${res.status}`);
	return (await res.json()) as T;
}

export async function postJson<T>(path: string, body: unknown): Promise<T> {
	if (mock) return (await mock.mockFetch('POST', path, body)) as T;
	const res = await fetch(path, {
		method: 'POST',
		headers: { 'Content-Type': 'application/json' },
		body: JSON.stringify(body)
	});
	if (!res.ok) throw new Error(`${path}: ${res.status}`);
	const text = await res.text();
	return (text ? JSON.parse(text) : undefined) as T;
}

/** WebSocket with reconnect (1 s doubling to 10 s). `onOpen` runs on every (re)connect. */
export function connect<M>(path: string, h: SocketHandlers<M>): Socket {
	if (mock) return mock.mockSocket<M>(path, h);

	const proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
	let ws: WebSocket | null = null;
	let closed = false;
	let delay = 1000;
	let timer: ReturnType<typeof setTimeout> | undefined;

	const open = () => {
		const cur = new WebSocket(`${proto}//${location.host}${path}`);
		ws = cur;
		cur.onopen = () => {
			delay = 1000;
			h.onOpen();
		};
		cur.onmessage = (ev) => {
			try {
				h.onMessage(JSON.parse(ev.data as string) as M);
			} catch {
				// ignore frames that are not JSON
			}
		};
		cur.onclose = () => {
			ws = null;
			h.onClose();
			if (closed) return;
			timer = setTimeout(open, delay);
			delay = Math.min(delay * 2, 10000);
		};
	};
	open();

	return {
		send(msg) {
			const cur = ws;
			if (cur?.readyState === WebSocket.OPEN) cur.send(JSON.stringify(msg));
		},
		close() {
			closed = true;
			clearTimeout(timer);
			ws?.close();
		}
	};
}
