<script lang="ts">
import type { Tone } from '$lib/format';

let { label, tone }: { label: string; tone: Tone } = $props();

const tones: Record<Tone, string> = {
	run: 'text-sky-700 dark:text-sky-300',
	wait: 'text-amber-700 dark:text-amber-300',
	idle: 'text-neutral-500 dark:text-neutral-400',
	err: 'text-red-700 dark:text-red-300',
	off: 'text-neutral-500 dark:text-neutral-400',
	sleep: 'text-violet-700 dark:text-violet-300'
};

const dots: Record<Tone, string> = {
	run: 'bg-sky-500',
	wait: 'bg-amber-500',
	idle: 'bg-neutral-400',
	err: 'bg-red-500',
	off: 'bg-neutral-400',
	sleep: 'bg-violet-500'
};
</script>

<span class={['inline-flex items-center gap-1.5 text-xs whitespace-nowrap', tones[tone]]}>
	<span class={['size-1.5 rounded-full', dots[tone]]} aria-hidden="true"></span>
	{label}
</span>
