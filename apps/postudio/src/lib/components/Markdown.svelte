<script lang="ts">
import { renderMarkdown } from '$lib/markdown';
import { markdownControls } from '$lib/markdown-controls';
import 'katex/dist/katex.min.css';

let { text, lineHeight = '1.75', citations = [], oncitation }: { text: string; lineHeight?: string; citations?: readonly { url: string }[]; oncitation?: (number: number) => void } = $props();
const html = $derived(renderMarkdown(text, citations));

function copyOnClick(node: HTMLElement) {
	return markdownControls(node, (number) => {
		if (Number.isInteger(number) && number > 0 && number <= citations.length) oncitation?.(number);
	});
}
</script>

<div
	class="md min-w-0 [overflow-wrap:anywhere] [&>*:first-child]:mt-0 [&>*:last-child]:mb-0"
	style:line-height={lineHeight}
	{@attach copyOnClick}
>
	{@html html}
</div>

<style>
	.md { word-break: keep-all; font-size: 15.5px; }
	.md :global(p) { margin: 0; }
	.md :global(strong) { font-weight: 700; }
	.md :global(h1), .md :global(h2), .md :global(h3), .md :global(h4) { margin: 0; font-weight: 700; line-height: 1.4; letter-spacing: -.01em; }
	.md :global(h1) { font-size: 20px; }
	.md :global(h2) { font-size: 17.5px; }
	.md :global(h3) { font-size: 15.5px; }
	.md :global(h4) { font-size: 14px; color: #55555a; }
	.md :global(* + h1), .md :global(* + h2) { margin-top: 28px; }
	.md :global(* + h3), .md :global(* + h4) { margin-top: 22px; }
	.md :global(h1 + *), .md :global(h2 + *), .md :global(h3 + *), .md :global(h4 + *) { margin-top: 8px; }
	.md :global(ul), .md :global(ol) { margin: 0; padding-left: 1.4em; }
	.md :global(ul) { list-style: disc; }
	.md :global(ol) { list-style: decimal; }
	.md :global(li) { padding-left: .2em; }
	.md :global(li + li), .md :global(li > ul), .md :global(li > ol) { margin-top: 6px; }
	.md :global(ol > li::marker) { font-weight: 600; color: #55555a; font-variant-numeric: tabular-nums; }
	.md :global(ul > li::marker) { color: #8a8a90; }
	.md :global(:not(pre) > code) { font: .86em ui-monospace, monospace; padding: 1px 5px; border: 1px solid #e8e8eb; border-radius: 5px; background: #f6f6f7; white-space: break-spaces; }
	.md :global(img) { max-width: 100%; border: 1px solid #ececef; border-radius: 10px; }
	.md :global(> * + *) { margin-top: 14px; }
	.md :global(p) { margin-bottom: 0; text-wrap: pretty; }
	.md :global(em) { font-style: normal; font-weight: 600; }
	.md :global(a) { color: #1f48a3; text-decoration: none; border-bottom: 1px solid #d7e2fa; }
	.md :global(a:hover) { border-bottom-color: currentColor; }
	:global(.dark) .md :global(a) { color: #a9c2ff; border-bottom-color: #425582; }
	.md :global(blockquote) { border-left: 3px solid #dcdce0; padding: 2px 0 2px 14px; }
	.md :global(hr) { border: 0; border-top: 1px solid #ececef; margin: 26px 0; }
	:global(.dark) .md :global(hr) { border-top-color: #3f3f46; }
	.md :global(.chat-citation) { margin-inline: 3px; min-width: 18px; height: 18px; padding: 0 4px; border-radius: 5px; background: #e8eefc; color: #1f48a3; font: 700 11px/1 ui-monospace, monospace; vertical-align: .15em; }
	.md :global(.chat-citation:focus-visible) { outline: 2px solid #2b59c3; outline-offset: 2px; }
	.md :global(p:has(> .chat-citation:first-child)) { color: #66666b; font-size: .8125rem; }
	:global(.dark) .md :global(p:has(> .chat-citation:first-child)) { color: #a1a1aa; }
	.md :global(.table-wrap) {
		max-width: 100%; overflow-x: auto; border: 1px solid #e6e6e8; border-radius: 10px;
	}
	.md :global(.table-wrap:focus-visible) { outline: 2px solid #2b59c3; outline-offset: 2px; }
	.md :global(table) { width: 100%; border-collapse: collapse; font-size: 14px; line-height: 1.55; }

	.md :global(th),
	.md :global(td) {
		border-bottom: 1px solid #ececef;
		padding: 8px 12px;
		text-align: left;
		vertical-align: top;
	}

	.md :global(th) {
		background: #fafafb; font-size: 12.5px; font-weight: 700; white-space: nowrap;
	}
	.md :global(tr:last-child td) { border-bottom: 0; }
	.md :global(.num) { text-align: right; font-family: ui-monospace, monospace; font-variant-numeric: tabular-nums; }
	.md :global(.center) { text-align: center; }
	:global(.dark) .md :global(.table-wrap) { border-color: #3f3f46; }

	:global(.dark) .md :global(th),
	:global(.dark) .md :global(td) {
		border-color: #52525b;
	}

	:global(.dark) .md :global(th) {
		background: #27272a;
	}
	.md :global(.code-block) { --code-bg: #f6f6f7; --code-line: #e8e8eb; border: 1px solid var(--code-line); border-radius: 12px; background: var(--code-bg); overflow: hidden; color: #1a1a1c; }
	.md :global(.code__head) { height: 36px; display: flex; align-items: center; gap: 8px; padding: 0 6px 0 12px; border-bottom: 1px solid var(--code-line); color: #66666b; font-size: 12px; }
	.md :global(.code__lang) { font-family: ui-monospace, monospace; font-weight: 600; }
	.md :global(.code__file) { font-family: ui-monospace, monospace; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0; }
	.md :global(.code__spacer) { flex: 1; }
	.md :global(.code__btn), .md :global(.math-copy) { flex-shrink: 0; height: 26px; border: 0; border-radius: 6px; background: transparent; padding: 0 8px; font-family: inherit; font-size: 12px; font-weight: 600; line-height: 1; cursor: pointer; white-space: nowrap; }
	.md :global(.code__btn:hover), .md :global(.math-copy:hover) { background: #ebebee; }
	.md :global(.code__btn:disabled) { opacity: .45; cursor: default; }
	.md :global(button:focus-visible), .md :global(pre:focus-visible) { outline: 2px solid #2b59c3; outline-offset: -2px; }
	.md :global(button[data-done="true"]) { color: #17693f; }
	.md :global(.code__body) { position: relative; }
	.md :global(pre) { margin: 0; padding: 12px 14px; overflow-x: auto; font: 13px/1.65 ui-monospace, monospace; tab-size: 4; white-space: pre; }
	.md :global(pre code) { font: inherit; flex: 1; min-width: 0; }
	.md :global(.code__body pre) { display: flex; }
	.md :global(.code__numbers) { flex-shrink: 0; padding-right: 12px; min-width: 2.2em; text-align: right; color: #8a8a90; user-select: none; }
	.md :global(.code__numbers > span) { display: block; }
	.md :global(.code--wrap .code__numbers) { display: none; }
	.md :global(.code--wrap pre) { white-space: pre-wrap; overflow-wrap: anywhere; word-break: break-all; }
	.md :global(.code--clamp .code__body) { max-height: 300px; overflow: hidden; }
	.md :global(.code--clamp .code__body::after) { content: ''; position: absolute; left: 0; right: 0; bottom: 0; height: 64px; background: linear-gradient(transparent, var(--code-bg)); pointer-events: none; }
	.md :global(.code__more) { width: 100%; height: 36px; border: 0; border-top: 1px solid var(--code-line); background: var(--code-bg); color: #1f48a3; font-size: 12.5px; font-weight: 600; cursor: pointer; }
	.md :global(.hljs-keyword), .md :global(.hljs-literal), .md :global(.hljs-selector-tag) { color: #8a3fb8; }
	.md :global(.hljs-string), .md :global(.hljs-attr) { color: #17693f; }
	.md :global(.hljs-number) { color: #a35a00; }
	.md :global(.hljs-title), .md :global(.hljs-built_in) { color: #1f48a3; }
	.md :global(.hljs-comment) { color: #8a8a90; font-style: italic; }
	.md :global(.diff-add) { background: #e6f4ec; }
	.md :global(.diff-delete) { background: #fbe9e7; }
	.md :global(.diff-add), .md :global(.diff-delete) { display: inline-block; min-width: 100%; }
	.md :global(.code--term) { --code-bg: #141417; --code-line: #26262b; color: #e6e6ea; }
	.md :global(.code--term .code__head) { color: #c4c4ca; }
	.md :global(.code--term .code__btn:hover) { background: #26262b; }
	.md :global(.code--term .hljs-string) { color: #8dd5a5; }
	.md :global(.code--term .hljs-built_in) { color: #a9c2ff; }
	.md :global(.math-block) { position: relative; margin-block: 14px; padding: 32px 16px 14px; border: 1px solid #f0f0f2; border-radius: 12px; background: #fafafb; overflow-x: auto; overflow-y: hidden; text-align: center; }
	.md :global(.math-block .katex-display) { margin: 0; }
	.md :global(.katex) { font-size: 1.05em; }
	.md :global(.math-inline) { display: inline-block; max-width: 100%; overflow-x: auto; overflow-y: hidden; vertical-align: middle; }
	.md :global(.math-copy) { position: absolute; right: 6px; top: 6px; height: 24px; font-size: 11.5px; color: #66666b; opacity: 0; }
	.md :global(.math-block:hover .math-copy), .md :global(.math-copy:focus-visible) { opacity: 1; }
	.md :global(.math-pending), .md :global(.math-error) { font: .86em ui-monospace, monospace; white-space: pre-wrap; }
	.md :global(.math-pending) { color: #8a8a90; }
	.md :global(.math-error) { color: #a52f22; text-decoration: underline dashed; text-underline-offset: 4px; cursor: help; }
	:global(.dark) .md :global(.code-block) { --code-bg: #202024; --code-line: #3f3f46; color: #e6e6ea; }
	:global(.dark) .md :global(.code__head), :global(.dark) .md :global(h4) { color: #b4b4ba; }
	:global(.dark) .md :global(.code__more) { color: #a9c2ff; }
	:global(.dark) .md :global(.code__btn:hover), :global(.dark) .md :global(.math-copy:hover) { background: #3f3f46; }
	:global(.dark) .md :global(.hljs-keyword), :global(.dark) .md :global(.hljs-literal) { color: #c992eb; }
	:global(.dark) .md :global(.hljs-string), :global(.dark) .md :global(.hljs-attr) { color: #8dd5a5; }
	:global(.dark) .md :global(.hljs-number) { color: #e6b673; }
	:global(.dark) .md :global(.hljs-title), :global(.dark) .md :global(.hljs-built_in) { color: #a9c2ff; }
	:global(.dark) .md :global(.diff-add) { background: #173f2c; }
	:global(.dark) .md :global(.diff-delete) { background: #542523; }
	:global(.dark) .md :global(.math-block), :global(.dark) .md :global(:not(pre) > code) { background: #202024; border-color: #3f3f46; }
	:global(.dark) .md :global(.math-error) { color: #ffa99d; }
	@media (hover: none) { .md :global(.math-copy) { opacity: 1; } }
</style>
