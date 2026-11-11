/// <reference lib="webworker" />
import { build, files, version } from '$service-worker';

declare const self: ServiceWorkerGlobalScope;

const CACHE = `postudio-${version}`;
const SHELL = [...build, ...files, '/index.html'];

self.addEventListener('install', (event) => {
	event.waitUntil(caches.open(CACHE).then((c) => c.addAll(SHELL)));
	void self.skipWaiting();
});

self.addEventListener('activate', (event) => {
	event.waitUntil(
		caches.keys().then((keys) => Promise.all(keys.filter((k) => k !== CACHE).map((k) => caches.delete(k))))
	);
	void self.clients.claim();
});

self.addEventListener('fetch', (event) => {
	const url = new URL(event.request.url);
	if (event.request.method !== 'GET' || url.origin !== self.location.origin) return;
	if (url.pathname.startsWith('/api/') || url.pathname.startsWith('/ws/')) return;
	event.respondWith(respond(event.request, url.pathname));
});

async function respond(req: Request, path: string): Promise<Response> {
	const cache = await caches.open(CACHE);
	if (SHELL.includes(path)) {
		const hit = await cache.match(path);
		if (hit) return hit;
	}
	try {
		return await fetch(req);
	} catch {
		// offline SPA navigation: serve the cached app shell
		return (await cache.match('/index.html')) ?? Response.error();
	}
}

self.addEventListener('push', (event) => {
	const data = event.data?.json() as { title?: string; body?: string; url?: string; tag?: string } | undefined;
	event.waitUntil(
		self.registration.showNotification(data?.title ?? 'Postudio', {
			body: data?.body ?? '',
			tag: data?.tag,
			data: { url: data?.url ?? '/' },
			icon: '/icon-192.png'
		})
	);
});

self.addEventListener('notificationclick', (event) => {
	event.notification.close();
	const url = (event.notification.data as { url?: string } | undefined)?.url ?? '/';
	event.waitUntil(
		(async () => {
			const list = await self.clients.matchAll({ type: 'window', includeUncontrolled: true });
			const same = list.find((c) => new URL(c.url).pathname === url);
			if (same) return same.focus();
			const any = list[0];
			if (any) {
				await any.navigate(url);
				return any.focus();
			}
			await self.clients.openWindow(url);
		})()
	);
});
