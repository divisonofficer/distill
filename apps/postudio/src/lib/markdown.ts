import DOMPurify from 'dompurify';
import { Marked } from 'marked';

const esc = (s: string) =>
	s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');

const md = new Marked({
	gfm: true,
	breaks: true,
	renderer: {
		code({ text, lang }) {
			const label = esc((lang ?? '').trim().split(/\s+/)[0] || 'text');
			return (
				`<div class="code-block my-3 overflow-hidden rounded-xl border border-neutral-200 dark:border-neutral-800">` +
				`<div class="flex items-center justify-between bg-neutral-100 px-3 py-1.5 text-xs text-neutral-600 dark:bg-neutral-900 dark:text-neutral-400">` +
				`<span class="font-mono">${label}</span>` +
				`<button type="button" data-copy class="min-h-7 rounded-md px-2 hover:bg-neutral-200 dark:hover:bg-neutral-800">복사</button>` +
				`</div>` +
				`<pre class="overflow-x-auto bg-neutral-50 p-3 text-[13px] leading-relaxed dark:bg-neutral-950"><code class="font-mono">${esc(text)}</code></pre>` +
				`</div>`
			);
		}
	}
});

/** Markdown to HTML. Output is always sanitized before it reaches `{@html}`. */
export function renderMarkdown(src: string): string {
	const html = md.parse(src, { async: false }) as string;
	return DOMPurify.sanitize(html);
}
