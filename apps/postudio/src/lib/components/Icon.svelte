<script lang="ts">
const paths = {
	menu: 'M4 7h16M4 12h16M4 17h16',
	sidebar: 'M4 5h16v14H4zM9 5v14',
	new: 'M12 5H5v14h14v-7M16 4l4 4-9 9H7v-4z',
	sliders: 'M4 7h10M18 7h2M4 17h2M10 17h10M14 5v4M6 15v4',
	send: 'M12 19V5M6 11l6-6 6 6',
	stop: 'M7 7h10v10H7z',
	image: 'M4 5h16v14H4zM4 16l5-5 4 4 3-3 4 4',
	close: 'M6 6l12 12M18 6L6 18',
	copy: 'M9 9h10v10H9zM5 15V5h10',
	check: 'M5 12l5 5L20 7',
	chevron: 'M9 6l6 6-6 6',
	shield: 'M12 3l7 3v6c0 4-3 7-7 9-4-2-7-5-7-9V6z',
	chat: 'M4 5h16v11H9l-5 4z'
} as const;

let { name, class: cls = 'size-5' }: { name: keyof typeof paths; class?: string } = $props();
</script>

<svg class={cls} aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d={paths[name]} /></svg>
