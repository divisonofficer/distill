import hljs from 'highlight.js/lib/core';
import javascript from 'highlight.js/lib/languages/javascript';
import typescript from 'highlight.js/lib/languages/typescript';
import python from 'highlight.js/lib/languages/python';
import bash from 'highlight.js/lib/languages/bash';
import json from 'highlight.js/lib/languages/json';
import yaml from 'highlight.js/lib/languages/yaml';
import css from 'highlight.js/lib/languages/css';
import xml from 'highlight.js/lib/languages/xml';
import rust from 'highlight.js/lib/languages/rust';
import { escapeHtml as esc } from './markdown-math';

for (const [name, language] of Object.entries({ javascript, typescript, python, bash, json, yaml, css, xml, rust })) hljs.registerLanguage(name, language);
const aliases: Record<string, string> = { js: 'javascript', ts: 'typescript', py: 'python', sh: 'bash', zsh: 'bash', console: 'bash', html: 'xml', yml: 'yaml', rs: 'rust' };

/** Copy preserves source whitespace but omits diff display markers (not file headers). */
export function codeCopyText(text: string, language: string): string {
 return language === 'diff' ? text.split('\n').map((line) => /^[+-](?![+-])/.test(line) ? line.slice(1) : line).join('\n') : text;
}

export function renderCodeBlock({ text, lang, raw }: { text: string; lang?: string; raw: string }): string {
 const language = (lang ?? '').trim().split(/\s+/)[0].toLowerCase();
 const title = (lang ?? '').match(/\btitle=(?:"([^"]*)"|'([^']*)')/);
 const filename = title?.[1] ?? title?.[2] ?? '';
 const lines = text.split('\n').length;
 const fence = raw.match(/^ {0,3}(`{3,}|~{3,})/);
 const closed = !fence || new RegExp(`(?:^|\\n) {0,3}${fence[1][0]}{${fence[1].length},}[ \\t]*(?:\\n)?$`).test(raw.slice(fence[0].length));
 const highlightLanguage = aliases[language] ?? language;
 const numbers = lines >= 10 ? `<span class="code__numbers" aria-hidden="true">${Array.from({ length: lines }, (_, index) => `<span>${index + 1}</span>`).join('')}</span>` : '';
 let body = esc(text);
 // Large/unknown blocks remain cheap, legible plain text; no auto-detection.
 if (text.length <= 100_000 && hljs.getLanguage(highlightLanguage)) {
  try { body = hljs.highlight(text, { language: highlightLanguage, ignoreIllegals: true }).value; } catch { /* escaped source is the fallback */ }
 }
 if (language === 'diff') body = text.split('\n').map((line) => `<span class="${line.startsWith('@@') ? 'hljs-comment' : /^\+(?!\+\+)/.test(line) ? 'diff-add' : /^-(?!--)/.test(line) ? 'diff-delete' : 'diff-line'}">${esc(line)}</span>`).join('\n');
 return `<div class="code-block${lines > 20 ? ' code--clamp' : ''}${['bash', 'sh', 'zsh', 'console', 'shell'].includes(language) ? ' code--term' : ''}" data-language="${esc(language)}" data-code-source="${esc(text)}" data-lines="${lines}">` +
  `<div class="code__head"><span class="code__lang">${esc(language || '코드')}</span>${filename ? `<span class="code__file">${esc(filename)}</span>` : ''}<span class="code__spacer"></span><button type="button" class="code__btn" data-wrap aria-pressed="false">줄바꿈</button><button type="button" class="code__btn" data-copy${closed ? '' : ' disabled title="코드가 완성되면 복사할 수 있어요"'}>복사</button></div>` +
  `<div class="code__body"><pre tabindex="0" aria-label="${esc(language || '코드')} 코드">${numbers}<code>${body}</code></pre></div>` +
  (lines > 20 ? `<button type="button" class="code__more" data-more aria-expanded="false">전체 보기 · ${lines}줄</button>` : '') + '</div>';
}
