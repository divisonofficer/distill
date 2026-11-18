import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import test from 'node:test';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const fontsDir = path.join(root, 'static', 'fonts');

test('self-hosted fonts have valid local assets and licenses', async () => {
  const appHtml = await readFile(path.join(root, 'src', 'app.html'), 'utf8');
  const link = appHtml.match(/<link\b(?=[^>]*\brel=["']stylesheet["'])[^>]*\bhref=["']([^"']*fonts\/fonts\.css)["'][^>]*>/i);
  assert.ok(link, 'app.html must link local fonts CSS');
  assert.ok(!/^https?:\/\//i.test(link[1]), 'font CSS link must be local');

  const css = await readFile(path.join(fontsDir, 'fonts.css'), 'utf8');
  assert.doesNotMatch(css, /(?:https?:)?\/\//i, 'font CSS must not contain remote URLs');
  assert.match(css, /font-family:\s*'Inter'/);
  assert.match(css, /font-family:\s*'Noto Sans KR'/);
  assert.match(css, /font-weight:\s*400\s+700/);
  assert.match(css, /font-display:\s*swap/);

  const urls = [...css.matchAll(/url\(([^)]+)\)/g)].map((match) => match[1].replace(/^['"]|['"]$/g, ''));
  assert.ok(urls.length > 0, 'font CSS must reference font files');
  for (const url of urls) {
    assert.ok(!/^https?:\/\//i.test(url), `font URL must be local: ${url}`);
    const filename = decodeURIComponent(url);
    assert.ok(!filename.includes('..'), `font URL must stay in fonts directory: ${url}`);
    const font = await readFile(path.join(fontsDir, filename));
    assert.equal(font.subarray(0, 4).toString('ascii'), 'wOF2', `${url} must be WOFF2`);
  }

  await Promise.all(['OFL-Inter.txt', 'OFL-Noto-Sans-KR.txt'].map(async (license) => {
    const contents = await readFile(path.join(fontsDir, license), 'utf8');
    assert.ok(contents.trim(), `${license} must not be empty`);
  }));
});
