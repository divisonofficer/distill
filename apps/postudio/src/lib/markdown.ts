import DOMPurify from 'dompurify';
import { Marked, type MarkedExtension } from 'marked';
import markedCjkFriendly from 'marked-cjk-friendly';
import { escapeHtml as esc, mathExtension } from './markdown-math';
import { renderCodeBlock } from './markdown-code';

const options: MarkedExtension = {
	gfm: true,
	breaks: true,
	renderer: {
		image({ href, text }) {
			if (!/^arXiv 원문 그림 \d{1,3}$/.test(text)) return false;
			try {
				const url = new URL(href);
				if (url.origin !== 'https://arxiv.org' || url.username || url.password || url.search || url.hash || !/^\/html\/[A-Za-z0-9._/-]+\.(?:png|jpe?g|webp)$/i.test(url.pathname)) return '';
				return `<img src="${esc(url.href)}" alt="${esc(text)}" loading="lazy" decoding="async" referrerpolicy="no-referrer">`;
			} catch { return ''; }
		},
		table({ header, rows, align }) {
			const numeric = header.map((_, index) => {
				const values = rows.map((row) => row[index]?.text.trim() ?? '').filter(Boolean);
				return align[index] === 'right' || (align[index] == null && values.length > 0 && values.every((value) => /^[+−-]?(?:\d{1,3}(?:,\d{3})+|\d+)(?:\.\d+)?(?:%|°)?$/.test(value)));
			});
			const rowHtml = (cells: typeof header, head = false) => `<tr>${cells.map((cell, index) => {
				const tag = head ? 'th' : 'td';
				const alignment = numeric[index] ? 'num' : align[index] === 'center' ? 'center' : '';
				return `<${tag}${head ? ' scope="col"' : ''}${alignment ? ` class="${alignment}"` : ''}>${this.parser.parseInline(cell.tokens)}</${tag}>`;
			}).join('')}</tr>`;
			return `<div class="table-wrap" role="region" aria-label="표" tabindex="0"><table><thead>${rowHtml(header, true)}</thead><tbody>${rows.map((row) => rowHtml(row)).join('')}</tbody></table></div>\n`;
		},
		code(token) { return renderCodeBlock(token); },
		html({ text }) { return esc(text); },
		link({ href, title, tokens }) {
			return `<a href="${esc(href)}"${title ? ` title="${esc(title)}"` : ''} rel="noopener noreferrer">${this.parser.parseInline(tokens)}</a>`;
		}
	}
};
const md = new Marked(options, markedCjkFriendly(), mathExtension());

export function citationUrlKey(value: string): string | undefined {
	try {
		const url = new URL(value);
		if (!['https:', 'http:'].includes(url.protocol) || url.username || url.password) return;
		if (['arxiv.org', 'www.arxiv.org', 'alphaxiv.org', 'www.alphaxiv.org'].includes(url.hostname)) {
			const paper = url.pathname.match(/^\/(?:abs|pdf|overview)\/([\w./-]+)$/)?.[1];
			if (paper) return `arxiv:${paper.replace(/\.pdf$/, '').replace(/v\d+$/, '')}`;
		}
		url.hash = '';
		return url.href;
	} catch { /* Not an absolute public URL. */ }
}

/** Markdown to HTML. Output is always sanitized before it reaches `{@html}`. */
export function renderMarkdown(src: string, citations: readonly { url: string }[] = []): string {
	const indices = new Map(citations.map((source, index) => [citationUrlKey(source.url), index + 1]));
	const parser = citations.length ? new Marked(options, markedCjkFriendly(), mathExtension(), { renderer: {
		link({ href, text, tokens }) {
			const key = citationUrlKey(href);
			const number = key && indices.get(key);
			if (!number) return false;
			const label = /^(?:논문 보기|원문 보기|출처|\[?\d+\]?)$/.test(text.trim()) ? '' : this.parser.parseInline(tokens);
			return `${label}<button type="button" data-citation="${number}" class="chat-citation" aria-label="출처 ${number}">${number}</button>`;
		}
	} }) : md;
	const html = parser.parse(src, { async: false }) as string;
	return DOMPurify.sanitize(html);
}
