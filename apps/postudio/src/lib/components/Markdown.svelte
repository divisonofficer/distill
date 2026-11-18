<script lang="ts">
import { renderMarkdown } from '$lib/markdown';

let { text, lineHeight = '1.75rem' }: { text: string; lineHeight?: string } = $props();
const html = $derived(renderMarkdown(text));

// Copy buttons come from the sanitized HTML, so one listener on the root handles them all.
function copyOnClick(node: HTMLElement) {
	const onclick = (e: MouseEvent) => {
		const btn = (e.target as HTMLElement).closest<HTMLButtonElement>('button[data-copy]');
		const code = btn?.closest('.code-block')?.querySelector('code');
		if (!btn || !code) return;
		void navigator.clipboard
			.writeText(code.textContent ?? '')
			.then(() => {
				btn.textContent = '복사됨';
				setTimeout(() => (btn.textContent = '복사'), 1500);
			})
			.catch(() => {});
	};
	node.addEventListener('click', onclick);
	return () => node.removeEventListener('click', onclick);
}
</script>

<div
	class="md min-w-0 [overflow-wrap:anywhere] leading-7 [&>*:first-child]:mt-0 [&>*:last-child]:mb-0 [&_a]:underline [&_h1]:mt-4 [&_h1]:text-xl [&_h1]:font-semibold [&_h2]:mt-4 [&_h2]:text-lg [&_h2]:font-semibold [&_h3]:mt-3 [&_h3]:font-semibold [&_li]:my-0.5 [&_ol]:list-decimal [&_ol]:pl-6 [&_p+p]:mt-3 [&_p]:my-1 [&_ul]:list-disc [&_ul]:pl-6 [&_:not(pre)>code]:rounded [&_:not(pre)>code]:bg-neutral-200/70 [&_:not(pre)>code]:px-1 [&_:not(pre)>code]:py-0.5 [&_:not(pre)>code]:text-[0.9em] dark:[&_:not(pre)>code]:bg-neutral-700/60"
	style:line-height={lineHeight}
	{@attach copyOnClick}
>
	{@html html}
</div>

<style>
	.md :global(table) {
		display: block;
		width: max-content;
		max-width: 100%;
		overflow-x: auto;
		border-collapse: collapse;
		white-space: normal;
	}

	.md :global(th),
	.md :global(td) {
		border: 1px solid #d4d4d8;
		padding: 0.375rem 0.625rem;
		text-align: left;
		vertical-align: top;
	}

	.md :global(th) {
		background: #f4f4f5;
		font-weight: 600;
	}

	:global(.dark) .md :global(th),
	:global(.dark) .md :global(td) {
		border-color: #52525b;
	}

	:global(.dark) .md :global(th) {
		background: #27272a;
	}
</style>
