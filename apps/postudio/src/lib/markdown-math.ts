import katex from 'katex';
import type { MarkedExtension } from 'marked';

export const escapeHtml = (s: string) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');

/** Delimiter parsing is lexical: code spans/fences are consumed by Marked first. */
function mathAt(src: string, block: boolean) {
 const open = block ? (src.startsWith('$$') ? '$$' : src.startsWith('\\[') ? '\\[' : '') : (src.startsWith('\\(') ? '\\(' : src.startsWith('$') && !src.startsWith('$$') ? '$' : '');
 if (!open) return;
 if (open === '$' && /\s/.test(src[1] ?? '')) return;
 const close = open === '\\[' ? '\\]' : open === '\\(' ? '\\)' : open;
 for (let i = open.length; i < src.length; i++) {
  if (src[i] === '\\' && !src.startsWith(close, i)) { i++; continue; }
  if (!block && src[i] === '\n') break;
  if (!src.startsWith(close, i)) continue;
  if (open === '$' && /\d/.test(src[i + 1] ?? '')) return;
  if (open === '$' && /\s/.test(src[i - 1])) continue;
  const raw = src.slice(0, i + close.length);
  return { raw, text: src.slice(open.length, i), pending: false };
 }
 // A standalone monetary amount is prose, not an unfinished formula.
 if (open === '$' && /^\$\d/.test(src)) return;
 // Keep an unfinished formula readable and escaped instead of attempting TeX recovery.
 const end = block ? src.length : (src.indexOf('\n') < 0 ? src.length : src.indexOf('\n'));
 return { raw: src.slice(0, end), text: src.slice(open.length, end), pending: true };
}

function renderMath(text: string, raw: string, block: boolean, pending: boolean) {
 const tag = block ? 'div' : 'span';
 if (pending) return `<${tag} class="${block ? 'math-block ' : ''}math-pending">${escapeHtml(raw)}</${tag}>`;
 let html: string;
 try {
  if (text.length > 16_384) throw new Error('수식이 너무 길어 원문으로 표시해요');
  // No URLs, HTML commands, shared macros, or unbounded macro expansion.
  html = katex.renderToString(text, { displayMode: block, throwOnError: true, trust: false, strict: 'error', output: 'html', maxExpand: 500, maxSize: 20 });
 } catch (error) {
  return `<${tag} class="${block ? 'math-block ' : ''}math-error" title="${escapeHtml(String(error))}">${escapeHtml(raw)}</${tag}>`;
 }
 const accessible = `<span role="math" aria-label="${escapeHtml(text)}">${html}</span>`;
 return block ? `<div class="math-block" data-tex="${escapeHtml(text)}"><button type="button" class="math-copy" data-math-copy>LaTeX 복사</button>${accessible}</div>` : `<span class="math-inline">${accessible}</span>`;
}

export function mathExtension(): MarkedExtension {
 return { extensions: [true, false].map((block) => ({
  name: block ? 'mathBlock' : 'mathInline',
  level: block ? 'block' as const : 'inline' as const,
  start(src: string) { const index = src.search(block ? /\$\$|\\\[/ : /\$|\\\(/); return index < 0 ? undefined : index; },
  tokenizer(src: string) {
   const match = mathAt(src, block);
   if (match) return { type: block ? 'mathBlock' : 'mathInline', ...match };
  },
  renderer(token) { return renderMath(token.text, token.raw, block, token.pending); }
 })) };
}
