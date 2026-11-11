<script lang="ts">
	import type { Tone } from '$lib/format';

	let { label, tone }: { label: string; tone: Tone } = $props();

	const tones: Record<Tone, string> = {
		run: 'bg-sky-100 text-sky-800 dark:bg-sky-900 dark:text-sky-100',
		wait: 'bg-amber-100 text-amber-900 dark:bg-amber-900 dark:text-amber-100',
		idle: 'bg-neutral-200 text-neutral-800 dark:bg-neutral-800 dark:text-neutral-200',
		err: 'bg-red-100 text-red-900 dark:bg-red-900 dark:text-red-100',
		off: 'bg-neutral-300 text-neutral-700 dark:bg-neutral-700 dark:text-neutral-300',
		sleep: 'bg-violet-100 text-violet-900 dark:bg-violet-900 dark:text-violet-100'
	};
</script>

<span class={['inline-block rounded-full px-2 py-0.5 text-xs font-medium whitespace-nowrap', tones[tone]]}>
	{label}
</span>
