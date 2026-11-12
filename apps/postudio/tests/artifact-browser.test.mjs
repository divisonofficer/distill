import assert from 'node:assert/strict';
import { after, before, test } from 'node:test';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createServer } from 'vite';

const root = fileURLToPath(new URL('..', import.meta.url));
function overrideLocation(origin) {
 const original = Object.getOwnPropertyDescriptor(globalThis, 'location');
 Object.defineProperty(globalThis, 'location', { configurable: true, value: { origin } });
 return () => original ? Object.defineProperty(globalThis, 'location', original) : delete globalThis.location;
}
let vite;
let browser;
before(async () => {
 vite = await createServer({ root, configFile: resolve(root, 'vite.config.ts'), server: { middlewareMode: true }, appType: 'custom', logLevel: 'silent' });
 browser = await vite.ssrLoadModule('/src/lib/artifact-browser.ts');
});
after(async () => { await vite?.close(); });

test('untrusted origin helper rejects malformed and trusted origins', () => {
 const { configuredUntrustedOrigin, artifactUrl } = browser;
 assert.equal(configuredUntrustedOrigin({ untrustedOrigin: 'https://separate.example' }, 'https://trusted.example'), 'https://separate.example');
 assert.equal(configuredUntrustedOrigin({ untrustedOrigin: 'https://trusted.example' }, 'https://trusted.example'), null);
 const restoreLocation = overrideLocation('https://ui.example');
 try {
  assert.equal(configuredUntrustedOrigin({ untrustedOrigin: 'https://ui.example' }, 'https://remote.example'), null);
 } finally { restoreLocation(); }
 for (const untrustedOrigin of [undefined, 'https://x.test/path', 'https://u:p@x.test', 'javascript:alert(1)', 'https://x.test?q=1', 'https://x.test/#fragment']) {
  assert.equal(configuredUntrustedOrigin({ untrustedOrigin }, 'https://trusted.example'), null);
 }
 assert.equal(artifactUrl('https://separate.example', 'sid', 'plots/a b.png'), 'https://separate.example/f/sid/plots/a%20b.png');
});
