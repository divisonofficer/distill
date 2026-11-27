import assert from 'node:assert/strict';
import { test } from 'node:test';
import { Marked } from 'marked';
import cjk from 'marked-cjk-friendly';
import { createServer } from 'vite';

test('CJK emphasis closes next to Korean particles without changing inline code', () => {
 const parser = new Marked(cjk());
 assert.match(parser.parse('**부호 거리장(SDF)**은 **"명시된 가정 아래에서"**입니다'), /<strong>부호 거리장\(SDF\)<\/strong>은 <strong>&quot;명시된 가정 아래에서&quot;<\/strong>입니다/);
 assert.match(parser.parse('`**부호 거리장(SDF)**은`'), /<code>\*\*부호 거리장\(SDF\)\*\*은<\/code>/);
});

test('paper citation URLs match arXiv, PDF, and alphaXiv variants', async () => {
 const vite = await createServer({ server: { middlewareMode: true }, appType: 'custom', logLevel: 'silent' });
 try {
  const { citationUrlKey } = await vite.ssrLoadModule('/src/lib/markdown.ts');
  assert.equal(citationUrlKey('https://arxiv.org/abs/2611.12345v2'), citationUrlKey('https://www.alphaxiv.org/overview/2611.12345'));
  assert.equal(citationUrlKey('https://arxiv.org/pdf/2611.12345v1.pdf'), 'arxiv:2611.12345');
  assert.equal(citationUrlKey('javascript:alert(1)'), undefined);
  assert.equal(citationUrlKey('https://example.org/doc#anchor'), 'https://example.org/doc');
 } finally { await vite.close(); }
});
