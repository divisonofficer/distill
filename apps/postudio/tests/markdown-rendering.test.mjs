import assert from 'node:assert/strict';
import { test } from 'node:test';
import { JSDOM } from 'jsdom';
import { createServer } from 'vite';

test('sanitized math, code, citations and interactive controls', async (t) => {
 const dom = new JSDOM('<!doctype html><body></body>', { url: 'https://postudio.test' });
 const saved = Object.getOwnPropertyDescriptors(globalThis);
 for (const key of ['window', 'document', 'Element', 'HTMLElement', 'navigator']) Object.defineProperty(globalThis, key, { configurable: true, value: key === 'window' ? dom.window : dom.window[key] });
 const vite = await createServer({ server: { middlewareMode: true }, appType: 'custom', logLevel: 'silent' });
 try {
  const { renderMarkdown } = await vite.ssrLoadModule('/src/lib/markdown.ts');
  const { markdownControls } = await vite.ssrLoadModule('/src/lib/markdown-controls.ts');
  const render = (src, citations = []) => { const root = document.createElement('div'); root.innerHTML = renderMarkdown(src, citations); document.body.replaceChildren(root); return root; };
  await t.test('all four math delimiters render locally and retain layout', () => {
   for (const src of ['$2x$', '$x^2$', '\\(x^2\\)', '$$\\frac{a}{b}$$', '\\[\\sqrt{x}\\]']) {
    const root = render(src); assert.ok(root.querySelector('.katex'), src); assert.equal(root.querySelector('.math-error'), null);
   }
   assert.ok(render('$$x\\tag{1}$$').querySelector('.katex-tag'));
  });
  await t.test('unfinished/invalid formulas are escaped readable source', () => {
   assert.equal(render('$\\frac{a}').querySelector('.math-pending').textContent, '$\\frac{a}');
   assert.equal(render('$$\\frac{a}').querySelector('.math-pending').textContent, '$$\\frac{a}');
   const invalid = render('$\\notACommand{x}$').querySelector('.math-error');
   assert.equal(invalid.textContent, '$\\notACommand{x}$'); assert.match(invalid.title, /Undefined control sequence/);
   assert.ok(render('$x$').querySelector('.katex'));
   assert.ok(render('$' + 'x'.repeat(16_385) + '$').querySelector('.math-error'));
   assert.ok(render('$\\def\\a{\\a}\\a$').querySelector('.math-error'));
  });
  await t.test('currency, escaped dollars and code never become math', () => {
   for (const src of ['$5', '$5 and $10', '$ x $', '$x$2', '\\$x\\$', '`$x$`', '```latex\n$x$\n```']) assert.equal(render(src).querySelector('.katex'), null, src);
  });
  await t.test('HTML and unsafe TeX cannot introduce actions or scripts', () => {
   for (const src of ['<img src=x onerror="alert(1)">', '<button data-copy>fake</button>', '$\\href{javascript:alert(1)}{x}$', '$\\htmlClass{evil}{x}$', '```js title="<img src=x>"\n<script>alert(1)</script>\n```']) {
    const root = render(src); assert.equal(root.querySelector('script,img,[onerror],a[href^="javascript:"],.evil'), null, src);
   }
   assert.equal(render('[bad](javascript:alert%281%29)').querySelector('a')?.hasAttribute('href'), false);
  });
  await t.test('CJK, source receipts, numeric table alignment and ordered starts survive', () => {
   const root = render('**부호 거리장(SDF)**은 **"명시된 가정 아래에서"**입니다\n\n[원문 보기](https://arxiv.org/pdf/2611.12345v1.pdf)\n\n| 실험 | 수치 |\n|---|---|\n| A | 27.4 |\n\n3. 셋\n4. 넷', [{ url: 'https://alphaxiv.org/overview/2611.12345' }]);
   const linkRoot = render('[external](https://example.org)'); const link = linkRoot.querySelector('a');
   const stop = markdownControls(linkRoot, () => {}); link.addEventListener('click', (event) => event.preventDefault()); link.click();
   assert.equal(link.target, '_blank'); assert.match(link.rel, /noopener/); stop();
   assert.equal(root.querySelectorAll('strong').length, 2); assert.equal(root.querySelector('[data-citation]').textContent, '1');
   assert.equal(root.querySelector('.table-wrap').tabIndex, 0); assert.ok(root.querySelector('td.num')); assert.equal(root.querySelector('ol').start, 3);
  });
  await t.test('code metadata, highlighting and incomplete copy guard', () => {
   const root = render('```python title="a.py"\ndef run():\n    return 1\n```');
   assert.equal(root.querySelector('.code__file').textContent, 'a.py'); assert.ok(root.querySelector('.hljs-keyword')); assert.equal(root.querySelector('[data-copy]').disabled, false);
   assert.equal(render('```py\nprint(1)').querySelector('[data-copy]').disabled, true);
   assert.equal(render('~~~sh\necho ok\n~~~').querySelector('[data-copy]').disabled, false);
   assert.ok(render('```unknown\n<x>\n```').querySelector('code').textContent.includes('<x>'));
  });
  await t.test('copy/wrap/expand/math controls are keyboard-native and do not execute source', async () => {
   const source = Array.from({ length: 21 }, (_, i) => `line ${i}`).join('\n');
   const root = render('```text\n' + source + '\n```\n\n$$x^2$$');
   assert.equal(root.querySelectorAll('.code__numbers > span').length, 21);
   assert.equal(root.querySelector('.code__numbers').getAttribute('aria-hidden'), 'true');
   const copied = []; Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText: async (text) => copied.push(text) } });
   const dispose = markdownControls(root, () => {});
   try {
    root.querySelector('[data-wrap]').click(); assert.equal(root.querySelector('[data-wrap]').getAttribute('aria-pressed'), 'true'); assert.ok(root.querySelector('.code--wrap'));
    root.querySelector('[data-more]').click(); assert.equal(root.querySelector('[data-more]').getAttribute('aria-expanded'), 'true'); assert.equal(root.querySelector('.code--clamp'), null);
    root.querySelector('[data-more]').click(); assert.ok(root.querySelector('.code--clamp'));
    root.querySelector('[data-copy]').click(); await Promise.resolve(); assert.equal(copied[0], source); assert.equal(root.querySelector('[data-copy]').textContent, '복사했어요');
    root.querySelector('[data-math-copy]').click(); await Promise.resolve(); assert.equal(copied[1], 'x^2');
   } finally { dispose(); }
   const diff = render('```diff\n-old\n+new\n@@ block @@\n```'); const stop = markdownControls(diff, () => {});
   diff.querySelector('[data-copy]').click(); await Promise.resolve(); assert.equal(copied[2], 'old\nnew\n@@ block @@'); stop();
  });
  await t.test('clipboard rejection is visible rather than false success', async () => {
   const root = render('```js\n1\n```'); Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText: async () => { throw new Error('denied'); } } });
   const dispose = markdownControls(root, () => {}); root.querySelector('[data-copy]').click(); await Promise.resolve(); assert.equal(root.querySelector('[data-copy]').textContent, '복사 실패'); dispose();
  });
 } finally {
  await vite.close(); dom.window.close();
  for (const key of ['window', 'document', 'Element', 'HTMLElement', 'navigator']) { if (saved[key]) Object.defineProperty(globalThis, key, saved[key]); else delete globalThis[key]; }
 }
});
