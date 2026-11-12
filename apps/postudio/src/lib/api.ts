import type { ClientMsg } from './types';
import { orderedOrigins } from './fleet';

export function currentApiHost(): string { return typeof location === 'undefined' ? '' : location.origin; }
function endpoint(path: string, host?: string): string { return host ? `${host}${path}` : path; }

const mock = import.meta.env.VITE_MOCK ? await import('./mock') : null;

export interface Socket { send(msg: ClientMsg): boolean | void; close(): void; }
export interface ConnectedSocket extends Socket { send(msg: ClientMsg): boolean; }
export interface SocketHandlers<M> {
 onOpen(): void;
 onClose(): void;
 onMessage(msg: M): void;
 onHost?(origin: string): void;
}

export async function getJson<T>(path: string, host?: string): Promise<T> {
 if (mock) return (await mock.mockFetch('GET', path)) as T;
 const destination = path.startsWith('/api/chats') ? undefined : host;
 const res = await fetch(endpoint(path, destination), { signal: AbortSignal.timeout(2000), credentials: 'include' });
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
  } catch { /* Keep plain-text error responses useful. */ }
  throw new Error(`${path}: ${res.status}${detail.trim() ? ` ${detail.trim().slice(0, 500)}` : ''}`);
 }
 return (await res.json()) as T;
}

export async function postJson<T>(path: string, body: unknown, host?: string): Promise<T> {
 if (mock) return (await mock.mockFetch('POST', path, body)) as T;
 const destination = path.startsWith('/api/chats') ? undefined : host;
 const res = await fetch(endpoint(path, destination), { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body), credentials: 'include' });
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
  } catch { /* Keep plain-text error responses useful. */ }
  detail = detail.trim().slice(0, 500);
  throw new Error(`${path}: ${res.status}${detail ? ` ${detail}` : ''}`);
 }
 const text = await res.text();
 return (text ? JSON.parse(text) : undefined) as T;
}

export function connect<M>(path: string, h: SocketHandlers<M>, options: { host?: string; failover?: boolean } = {}): ConnectedSocket {
 if (mock) {
  const socket = mock.mockSocket<M>(path, h);
  let closed = false;
  return { send(msg) { if (closed) return false; socket.send(msg); return true; }, close() { if (closed) return; closed = true; socket.close(); } };
 }
 let ws: WebSocket | null = null;
 let closed = false;
 let delay = 1000;
 let hostIndex = 0;
 let hosts = options.host ? [options.host] : options.failover ? [...orderedOrigins(), currentApiHost()] : [];
 let timer: ReturnType<typeof setTimeout> | undefined;
 let handshake: ReturnType<typeof setTimeout> | undefined;
 const open = () => {
  if (options.failover) hosts = [...orderedOrigins(), currentApiHost()];
  const selected = hosts.length ? hosts[hostIndex % hosts.length] : undefined;
  const base = selected ? new URL(selected) : location;
  const cur = new WebSocket(`${base.protocol === 'https:' ? 'wss:' : 'ws:'}//${base.host}${path}`);
  ws = cur;
  h.onHost?.(selected ?? location.origin);
  const retry = () => { if (ws !== cur) return; clearTimeout(handshake); ws = null; cur.onopen = cur.onclose = cur.onerror = null; cur.close(); rotate(); };
  const rotate = () => { if (options.failover && hosts.length) hostIndex = (hostIndex + 1) % hosts.length; h.onClose(); if (!closed) { timer = setTimeout(open, delay); delay = Math.min(delay * 2, 10000); } };
  handshake = setTimeout(() => { if (ws === cur && cur.readyState !== WebSocket.OPEN) retry(); }, 2000);
  cur.onopen = () => { if (ws !== cur) return; clearTimeout(handshake); delay = 1000; h.onOpen(); };
  cur.onmessage = (ev) => { if (ws !== cur) return; try { h.onMessage(JSON.parse(ev.data as string) as M); } catch { /* Ignore non-JSON frames. */ } };
  cur.onerror = () => { if (ws === cur && cur.readyState !== WebSocket.OPEN) retry(); };
  cur.onclose = () => { if (ws !== cur) return; clearTimeout(handshake); ws = null; rotate(); };
 };
 open();
 return {
  send(msg) { const cur = ws; if (cur?.readyState !== WebSocket.OPEN) return false; try { cur.send(JSON.stringify(msg)); return true; } catch { return false; } },
  close() { closed = true; clearTimeout(timer); clearTimeout(handshake); const cur = ws; ws = null; cur?.close(); }
 };
}
