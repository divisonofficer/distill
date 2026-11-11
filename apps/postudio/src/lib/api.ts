import type { ClientMsg } from './types';

// Dev-only mock transport. Vite replaces the flag with a constant, so the
// dynamic import and mock.ts are dropped from production builds.
const mock = import.meta.env.VITE_MOCK ? await import('./mock') : null;

// The mock transport retains a void send; connect normalizes sends to boolean.
export interface Socket {
	send(msg: ClientMsg): boolean | void;
	close(): void;
}

export interface ConnectedSocket extends Socket {
	send(msg: ClientMsg): boolean;
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
	if (!res.ok) {
		const text = await res.text();
		let detail = text;
		try {
			const body: unknown = JSON.parse(text);
			if (body && typeof body === 'object') {
				const data = body as Record<string, unknown>;
				if (typeof data.error === 'string') detail = data.error;
				else if (typeof data.message === 'string') detail = data.message;
			}
		} catch {
			// Keep plain-text error responses useful.
		}
		detail = detail.trim().slice(0, 500);
		throw new Error(`${path}: ${res.status}${detail ? ` ${detail}` : ''}`);
	}
	const text = await res.text();
	return (text ? JSON.parse(text) : undefined) as T;
}

/** WebSocket with reconnect (1 s doubling to 10 s). `onOpen` runs on every (re)connect. */
export function connect<M>(path: string, h: SocketHandlers<M>): ConnectedSocket {
	if (mock) {
		const socket = mock.mockSocket<M>(path, h);
		let closed = false;
		return {
			send(msg) {
				if (closed) return false;
				socket.send(msg);
				return true;
			},
			close() {
				if (closed) return;
				closed = true;
				socket.close();
			}
		};
	}

	const proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
	let ws: WebSocket | null = null;
	let closed = false;
	let delay = 1000;
	let timer: ReturnType<typeof setTimeout> | undefined;

	const open = () => {
		const cur = new WebSocket(`${proto}//${location.host}${path}`);
		ws = cur;
		// Frames from a superseded socket are dropped so one view never mixes two streams.
		cur.onopen = () => {
			if (ws !== cur) return;
			delay = 1000;
			h.onOpen();
		};
		cur.onmessage = (ev) => {
			if (ws !== cur) return;
			try {
				h.onMessage(JSON.parse(ev.data as string) as M);
			} catch {
				// ignore frames that are not JSON
			}
		};
		cur.onclose = () => {
			if (ws !== cur) return;
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
			if (cur?.readyState !== WebSocket.OPEN) return false;
			try {
				cur.send(JSON.stringify(msg));
				return true;
			} catch {
				return false;
			}
		},
		close() {
			closed = true;
			clearTimeout(timer);
			// Detach first so frames still queued on this socket are dropped.
			const cur = ws;
			ws = null;
			cur?.close();
		}
	};
}
