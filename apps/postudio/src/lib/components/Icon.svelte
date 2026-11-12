<script lang="ts">
const paths = {
	menu: 'M4 7h16M4 12h16M4 17h16',
	sidebar: 'M4 5h16v14H4zM9 5v14',
	new: 'M12 5H5v14h14v-7M16 4l4 4-9 9H7v-4z',
	sliders: 'M4 7h10M18 7h2M4 17h2M10 17h10M14 5v4M6 15v4',
settings: 'M12 3l1.2 1.9 2.2-.3.9 2 2.1 1-.3 2.2L20 12l-1.9 1.2.3 2.2-2 0.9-1 2.1-2.2-.3L12 20l-1.2-1.9-2.2.3-.9-2-2.1-1 .3-2.2L4 12l1.9-1.2-.3-2.2 2-.9 1-2.1 2.2.3zM12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6z',
list: 'M9 6h11M9 12h11M9 18h11M4 6h.01M4 12h.01M4 18h.01',
	send: 'M12 19V5M6 11l6-6 6 6',
	stop: 'M7 7h10v10H7z',
	image: 'M4 5h16v14H4zM4 16l5-5 4 4 3-3 4 4',
	close: 'M6 6l12 12M18 6L6 18',
	copy: 'M9 9h10v10H9zM5 15V5h10',
	edit: 'M4 16.5V20h3.5L19 8.5 15.5 5 4 16.5zM14.5 6l3.5 3.5',
	fork: 'M6 4v16M6 9a6 6 0 0 0 6 6h5m-3-3 3 3-3 3',
	check: 'M5 12l5 5L20 7',
	chevron: 'M9 6l6 6-6 6',
	shield: 'M12 3l7 3v6c0 4-3 7-7 9-4-2-7-5-7-9V6z',
	chat: 'M4 5h16v11H9l-5 4z',
	calendar: 'M5 5h14v15H5zM8 3v4M16 3v4M5 10h14',
	book: 'M5 4h14v16H5zM8 4v16M11 8h5M11 12h5',
	document: 'M6 3h8l4 4v14H6zM14 3v5h4M9 12h6M9 16h6',
	presentation: 'M3 4h18v13H3zM12 17v4M8 21h8',
	terminal: 'M5 6l5 5-5 5M12 17h7',
	server: 'M4 4h16v6H4zM4 14h16v6H4zM7 7h1M7 17h1',
folder: 'M3 6h7l2 2h9v11H3z',
globe: 'M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zM3 12h18M12 3a15 15 0 0 1 0 18M12 3a15 15 0 0 0 0 18'
} as const;

let { name, class: cls = 'size-5' }: { name: keyof typeof paths; class?: string } = $props();
</script>

<svg class={cls} aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d={paths[name]} /></svg>
