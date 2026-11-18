import { sveltekit } from '@sveltejs/kit/vite';
import tailwindcss from '@tailwindcss/vite';
import { defineConfig } from 'vite';

const backend = 'http://127.0.0.1:7410';
const headers = { 'Tailscale-User-Login': 'me@example.com' };

export default defineConfig({
	plugins: [tailwindcss(), sveltekit()],
	server: {
		host: '127.0.0.1',
		port: 5173,
		strictPort: true,
		fs: { allow: ['../..'] },
		proxy: {
			'/api': { target: backend, headers },
			'/ws': { target: backend, ws: true, headers }
		}
	}
});
