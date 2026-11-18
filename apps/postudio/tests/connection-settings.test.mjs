import assert from 'node:assert/strict';
import { after, before, test } from 'node:test';
import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createServer } from 'vite';

const root = fileURLToPath(new URL('..', import.meta.url));
let vite, fleet, component, render;
before(async () => {
 const values = new Map();
 globalThis.localStorage = { getItem: (key) => values.get(key) ?? null, setItem: (key, value) => values.set(key, value) };
 vite = await createServer({ root, configFile: resolve(root, 'vite.config.ts'), server: { middlewareMode: true }, appType: 'custom', logLevel: 'silent' });
 fleet = await vite.ssrLoadModule('/src/lib/fleet.ts');
 ({ render } = await vite.ssrLoadModule('svelte/server'));
 component = (await vite.ssrLoadModule('/src/lib/components/ConnectionSettings.svelte')).default;
});
after(async () => { await vite?.close(); });

test('origins reject credentials, paths and insecure non-loopback; paired hosts retain priority', () => {
 assert.equal(fleet.pairingOrigin('http://localhost:3000'), 'http://localhost:3000');
 assert.equal(fleet.pairingOrigin('http://127.0.0.1:3000'), 'http://127.0.0.1:3000');
 for (const value of ['http://example.com', 'https://user:pass@example.com', 'https://example.com/path', 'javascript:alert(1)']) assert.equal(fleet.pairingOrigin(value), null);
 fleet.saveHosts(['https://first.example', 'https://second.example']);
 fleet.savePairedHost('https://third.example');
 assert.deepEqual(fleet.orderedOrigins(), ['https://first.example', 'https://second.example', 'https://third.example']);
 assert.throws(() => fleet.savePairedHost('http://evil.example'));
 assert.equal(fleet.invitationExpiry(1000), 1000000);
 assert.equal(fleet.invitationExpiry(1700000000000), 1700000000000);
});

test('initial render hides invitation and exposes sharing and discovery controls', () => {
 const { body } = render(component, { props: { onpaired() {} } });
 assert.match(body, /초대 생성/);
 assert.match(body, /초대로 연결/);
 assert.match(body, /기기 검색/);
 assert.match(body, /작업 공간 파일은 공유하지 않아요/);
 assert.doesNotMatch(body, /id="connection-invite"/);
});

test('pending and error states and guarded API contracts remain present', async () => {
 const source = await readFile(resolve(root, 'src/lib/components/ConnectionSettings.svelte'), 'utf8');
 for (const path of ['/api/connections', '/api/connections/invite', '/api/connections/join', '/api/connections/remove', '/api/fleet']) assert.ok(source.includes(path));
 for (const text of ['생성 중…', '연결 중…', '검색 중…', '연결을 완료하지 못했어요', '검색된 기기가 없어요', 'disabled={!!pending}', 'clearInterval(timer)', 'rel="noopener noreferrer"']) assert.ok(source.includes(text));
 assert.doesNotMatch(source, /localStorage|console\./);
});

test('invitation preview validates bounded opaque payload without exposing token', () => {
 const payload = { version: 1, origin: 'https://remote.example', publicKey: 'key', nodeId: 'a'.repeat(64), token: 'secret', expiresAt: 2000000000 };
 const encode = (value) => Buffer.from(JSON.stringify(value)).toString('base64url');
 assert.deepEqual(fleet.invitationPreview(encode(payload), 1000), { origin: payload.origin, nodeId: payload.nodeId });
 for (const input of ['!', 'a'.repeat(16385), encode({ ...payload, expiresAt: 1 }), encode({ ...payload, origin: 'http://evil.example' }), encode({ ...payload, nodeId: 'bad' })]) assert.equal(fleet.invitationPreview(input, 1000), null);
});

test('join requires local address, confirms preview and avoids post-unmount host mutation', async () => {
 const source = await readFile(resolve(root, 'src/lib/components/ConnectionSettings.svelte'), 'utf8');
 assert.match(source, /invitation: pasted\.trim\(\), origin/);
 assert.match(source, /상대 웹 주소: \$\{remote\.origin\}.*상대 기기 지문: \$\{remote\.nodeId\}/);
 assert.match(source, /if \(!mounted\) return;\s*pasted = ''; clearInvitation\(\);\s*try \{ savePairedHost/);
 assert.match(source, /한쪽만 설정됐을 수/);
 assert.match(source, /호스트 목록의 주소는 유지돼요/);
});
