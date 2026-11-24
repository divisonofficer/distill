import type { ClientMsg } from './types';
import { orderedOrigins } from './fleet';

export function currentApiHost(): string { return typeof location === 'undefined' ? '' : location.origin; }
function endpoint(path: string, host?: string): string { return host ? `${host}${path}` : path; }

const mock = import.meta.env.VITE_MOCK ? await import('./mock') : null;

export interface Socket { send(msg: ClientMsg): boolean | void; close(): void; }
export interface ConnectedSocket extends Socket { send(msg: ClientMsg): boolean; }
export interface SocketHandlers<M> { onOpen(): void; onClose(): void; onMessage(msg: M): void; onHost?(origin: string): void; }

export class ApiError extends Error {
 constructor(public status: number, public path: string, public detail: string, public body = detail) {
  super(`${path}: ${status}${detail.trim() ? ` ${detail.trim().slice(0, 500)}` : ''}`);
  this.name = 'Error';
 }
}

async function requestJson<T>(path: string, host: string | undefined, method: string, body: unknown, timeoutMs = 20_000): Promise<T> {
 if (mock) return (await mock.mockFetch(method, path, body)) as T;
 const destination = path.startsWith('/api/chats') ? undefined : host;
 const res = await fetch(endpoint(path, destination), { method, headers: body === undefined ? undefined : { 'Content-Type': 'application/json' }, body: body === undefined ? undefined : JSON.stringify(body), credentials: 'include', signal: AbortSignal.timeout(timeoutMs) });
 if (!res.ok) {
  const text = await res.text(); let detail = text;
  try { const data: unknown = JSON.parse(text); if (data && typeof data === 'object') { const item = data as Record<string, unknown>; if (typeof item.error === 'string') detail = item.error; else if (typeof item.message === 'string') detail = item.message; } } catch { /* Keep plain-text error responses useful. */ }
  throw new ApiError(res.status, path, detail, text);
 }
 const text = await res.text(); return (text ? JSON.parse(text) : undefined) as T;
}
export async function getJson<T>(path: string, host?: string, timeoutMs = 2000): Promise<T> { return requestJson<T>(path, host, 'GET', undefined, timeoutMs); }
export async function postJson<T>(path: string, body: unknown, host?: string): Promise<T> { return requestJson<T>(path, host, 'POST', body); }
export async function putJson<T>(path: string, body: unknown, host?: string): Promise<T> { return requestJson<T>(path, host, 'PUT', body); }
export async function deleteJson<T>(path: string, host?: string): Promise<T> { return requestJson<T>(path, host, 'DELETE', undefined); }

export function connect<M>(path: string, h: SocketHandlers<M>, options: { host?: string; failover?: boolean } = {}): ConnectedSocket {
 if (mock) { const socket = mock.mockSocket<M>(path, h); let closed = false; return { send(msg) { if (closed) return false; socket.send(msg); return true; }, close() { if (closed) return; closed = true; socket.close(); } }; }
 let ws: WebSocket | null = null; let closed = false; let delay = 1000; let hostIndex = 0; let hosts = options.host ? [options.host] : options.failover ? [...orderedOrigins(), currentApiHost()] : []; let timer: ReturnType<typeof setTimeout> | undefined; let handshake: ReturnType<typeof setTimeout> | undefined;
 const open = () => { if (options.failover) hosts = [...orderedOrigins(), currentApiHost()]; const selected = hosts.length ? hosts[hostIndex % hosts.length] : undefined; const base = selected ? new URL(selected) : location; const cur = new WebSocket(`${base.protocol === 'https:' ? 'wss:' : 'ws:'}//${base.host}${path}`); ws = cur; h.onHost?.(selected ?? location.origin); const retry = () => { if (ws !== cur) return; clearTimeout(handshake); ws = null; cur.onopen = cur.onclose = cur.onerror = null; cur.close(); rotate(); }; const rotate = () => { if (options.failover && hosts.length) hostIndex = (hostIndex + 1) % hosts.length; h.onClose(); if (!closed) { timer = setTimeout(open, delay); delay = Math.min(delay * 2, 10000); } }; handshake = setTimeout(() => { if (ws === cur && cur.readyState !== WebSocket.OPEN) retry(); }, 2000); cur.onopen = () => { if (ws !== cur) return; clearTimeout(handshake); delay = 1000; h.onOpen(); }; cur.onmessage = (ev) => { if (ws !== cur) return; try { h.onMessage(JSON.parse(ev.data as string) as M); } catch { /* Ignore non-JSON frames. */ } }; cur.onerror = () => { if (ws === cur && cur.readyState !== WebSocket.OPEN) retry(); }; cur.onclose = () => { if (ws !== cur) return; clearTimeout(handshake); ws = null; rotate(); }; };
 open(); return { send(msg) { const cur = ws; if (cur?.readyState !== WebSocket.OPEN) return false; try { cur.send(JSON.stringify(msg)); return true; } catch { return false; } }, close() { closed = true; clearTimeout(timer); clearTimeout(handshake); const cur = ws; ws = null; cur?.close(); } };
}
