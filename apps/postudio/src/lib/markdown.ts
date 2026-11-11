import DOMPurify from 'dompurify';
import { marked } from 'marked';

/** Markdown to HTML. Output is always sanitized before it reaches `{@html}`. */
export function renderMarkdown(src: string): string {
	const html = marked.parse(src, { async: false, gfm: true, breaks: true }) as string;
	return DOMPurify.sanitize(html);
}
