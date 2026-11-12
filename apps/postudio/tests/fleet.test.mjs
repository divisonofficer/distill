import assert from 'node:assert/strict';
import { after, before, test } from 'node:test';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createServer } from 'vite';

const root = fileURLToPath(new URL('..', import.meta.url));
let vite;
let fleet;
let api;

before(async () => {
  globalThis.location = { origin: 'https://local.example' };
  const values = new Map();
  globalThis.localStorage = { getItem: (key) => values.get(key) ?? null, setItem: (key, value) => values.set(key, value) };
  vite = await createServer({ root, configFile: resolve(root, 'vite.config.ts'), server: { middlewareMode: true }, appType: 'custom', logLevel: 'silent' });
  fleet = await vite.ssrLoadModule('/src/lib/fleet.ts');
  api = await vite.ssrLoadModule('/src/lib/api.ts');
});

after(async () => { await vite?.close(); });

test('host list splits newlines and commas without splitting hostname letters', () => {
  assert.deepEqual(fleet.parseHostList('https://node.example\nhttps://peer.example,https://another.example'), [
    'https://node.example', 'https://peer.example', 'https://another.example'
  ]);
});

test('ordinary GET stays on current origin while explicit host pins destination', async () => {
  fleet.saveHosts(['https://peer-a.example', 'https://peer-b.example']);
  const calls = [];
  globalThis.fetch = async (url) => { calls.push(String(url)); return { ok: true, json: async () => ({}) }; };
  await api.getJson('/api/node');
  await api.getJson('/api/node', 'https://peer-b.example');
  await api.getJson('/api/chats', 'https://peer-b.example');
  assert.deepEqual(calls, ['/api/node', 'https://peer-b.example/api/node', '/api/chats']);
  assert.equal(fleet.orderedOrigins()[0], 'https://peer-a.example');
});

test('board socket rotates configured origins in user priority order', async () => {
  fleet.saveHosts(['https://peer-a.example', 'https://peer-b.example']);
  const urls = [];
  class FakeSocket {
    static OPEN = 1;
    readyState = 0;
    onopen = null;
    onclose = null;
    onmessage = null;
    constructor(url) { urls.push(String(url)); }
    close() { this.onclose?.(); }
    send() {}
  }
  globalThis.WebSocket = FakeSocket;
  const socket = api.connect('/ws/board', { onOpen() {}, onClose() {}, onMessage() {} }, { failover: true });
  globalThis.WebSocket.instances;
  await new Promise((resolve) => setTimeout(resolve, 20));
  // Trigger failure on first peer, then allow reconnect timer to select next peer.
  socket.close();
  assert.deepEqual(urls, ['wss://peer-a.example/ws/board']);
});

test('pinned session destination never fails over to another host', async () => {
  fleet.saveHosts(['https://peer-a.example', 'https://peer-b.example']);
  assert.equal(fleet.sessionOrigin('?host=https%3A%2F%2Fpeer-b.example'), 'https://peer-b.example');
  assert.throws(() => fleet.sessionOrigin('?host=https%3A%2F%2Fevil.example'), /Invalid/);
  const calls = [];
  globalThis.fetch = async (url) => { calls.push(String(url)); throw new Error('offline'); };
  await assert.rejects(api.getJson('/api/sessions', 'https://peer-b.example'), /offline/);
  assert.deepEqual(calls, ['https://peer-b.example/api/sessions']);
});

test('board provenance never uses backend session origin metadata', async () => {
 const { board } = await vite.ssrLoadModule('/src/lib/board.svelte.ts');
 const originalSocket = globalThis.WebSocket;
 let socket;
 class BoardSocket {
  static OPEN = 1;
  readyState = 1;
  constructor() { socket = this; }
  close() {}
  send() {}
 }
 globalThis.WebSocket = BoardSocket;
 fleet.saveHosts([]);
 board.start();
 socket.onmessage({ data: JSON.stringify({ t: 'board.upsert', sessions: [{ sessionId: 'local-row', origin: { kind: 'local' }, cwd: '/project' }] }) });
 const row = board.find('local-row');
 assert.equal(row.apiOrigin, location.origin);
 assert.equal(fleet.sessionHref(row.sessionId, row.apiOrigin), '/s/local-row');
 socket.onmessage({ data: JSON.stringify({ t: 'board.upsert', sessions: [{ sessionId: 'local-row', origin: { kind: 'local' }, cwd: '/project', title: 'updated' }] }) });
 assert.equal(board.find('local-row').apiOrigin, location.origin);
 globalThis.WebSocket = originalSocket;
});
