<script lang="ts">
	import { renderMarkdown } from '$lib/markdown';

	let { text }: { text: string } = $props();
	const html = $derived(renderMarkdown(text));
</script>

<div class="min-w-0 break-words [&_a]:underline [&_code]:font-mono [&_li]:ml-5 [&_li]:list-disc [&_p+p]:mt-2 [&_pre]:my-2 [&_pre]:overflow-x-auto [&_pre]:rounded [&_pre]:bg-neutral-100 [&_pre]:p-2 [&_pre]:text-xs dark:[&_pre]:bg-neutral-800">
	{@html html}
</div>
