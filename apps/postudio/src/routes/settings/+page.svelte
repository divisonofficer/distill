<script lang="ts">
	import { onMount } from 'svelte';
	import { page } from '$app/state';
	import AccountSettings from '$lib/components/AccountSettings.svelte';
	import { postJson, getJson } from '$lib/api';
	import { loadDefaultMode, MODES, MODE_LABEL, saveDefaultMode } from '$lib/prefs';
	import type { Mode } from '$lib/types';
import { ui } from '$lib/ui.svelte';
import { configuredHosts, parseHostList, probeHosts, saveHosts, type FleetHost } from '$lib/fleet';

	type PushState = 'checking' | 'unsupported' | 'denied' | 'off' | 'on';

	let pushState = $state<PushState>('checking');
	let busy = $state(false);
	let error = $state('');
	let defaultMode = $state<Mode>(loadDefaultMode());
let fleetText = $state('');
let fleetHosts = $state<FleetHost[]>([]);
let fleetBusy = $state(false);
let fleetError = $state('');

onMount(() => {
  fleetHosts = configuredHosts();
  fleetText = fleetHosts.map((host) => host.origin).join('\n');
  void refreshFleet();
});

async function refreshFleet(announce = false) {
  fleetBusy = true;
  try {
    fleetHosts = await probeHosts();
    if (announce) ui.notify(`호스트 상태 확인 완료: ${fleetHosts.filter((host) => host.online).length}/${fleetHosts.length}개 온라인`);
  } catch {
    if (announce) ui.notify('호스트 상태를 확인하지 못했습니다.', 'error');
  } finally { fleetBusy = false; }
}
function saveFleet() {
  fleetError = '';
  const origins = parseHostList(fleetText);
  const invalid = origins.filter((origin) => !/^https?:\/\//i.test(origin) || (() => { try { const url = new URL(origin); return url.pathname !== '/' || !!url.search || !!url.hash || !!url.username || !!url.password; } catch { return true; } })());
  if (invalid.length) { fleetError = `유효하지 않은 호스트: ${invalid.join(', ')}`; return; }
  try {
    fleetHosts = saveHosts(origins);
    fleetText = fleetHosts.map((host) => host.origin).join('\n');
    ui.notify('호스트 설정을 저장했습니다.');
    void refreshFleet();
  } catch {
    fleetError = '호스트 설정을 저장하지 못했습니다.';
    ui.notify('호스트 설정을 저장하지 못했습니다.', 'error');
  }
}

	$effect(() => {
		void refresh();
	});

	async function registration(): Promise<ServiceWorkerRegistration> {
		const reg = await navigator.serviceWorker.getRegistration();
		if (!reg) throw new Error('service worker not registered');
		return reg;
	}

	async function currentSub(): Promise<PushSubscription | null> {
		const reg = await navigator.serviceWorker.getRegistration();
		return reg ? reg.pushManager.getSubscription() : null;
	}

	async function refresh() {
		if (!('serviceWorker' in navigator) || !('PushManager' in window) || !('Notification' in window)) {
			pushState = 'unsupported';
			return;
		}
		if (Notification.permission === 'denied') {
			pushState = 'denied';
			return;
		}
		pushState = (await currentSub()) ? 'on' : 'off';
	}

	function b64ToBytes(b64url: string): Uint8Array<ArrayBuffer> {
		const pad = '='.repeat((4 - (b64url.length % 4)) % 4);
		const bin = atob((b64url + pad).replace(/-/g, '+').replace(/_/g, '/'));
		const out = new Uint8Array(new ArrayBuffer(bin.length));
		for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
		return out;
	}

	async function enable() {
		busy = true;
		error = '';
		try {
			const perm = await Notification.requestPermission();
			if (perm !== 'granted') {
				pushState = perm === 'denied' ? 'denied' : 'off';
				return;
			}
			const { publicKey } = await getJson<{ publicKey: string }>('/api/push/key');
			const reg = await registration();
			const sub = await reg.pushManager.subscribe({
				userVisibleOnly: true,
				applicationServerKey: b64ToBytes(publicKey)
			});
			await postJson('/api/push/subscribe', sub.toJSON());
			pushState = 'on';
			ui.notify('알림을 켰습니다.');
		} catch {
			error = '알림을 켜지 못했습니다.';
			ui.notify(error, 'error');
		} finally {
			busy = false;
		}
	}

	async function disable() {
		busy = true;
		error = '';
		try {
			const sub = await currentSub();
			if (sub) {
				await postJson('/api/push/unsubscribe', { endpoint: sub.endpoint });
				await sub.unsubscribe();
			}
			pushState = 'off';
			ui.notify('알림을 껐습니다.');
		} catch {
			error = '알림을 끄지 못했습니다.';
			ui.notify(error, 'error');
		} finally {
			busy = false;
		}
	}

	function setDefaultMode(m: Mode) {
		try {
			saveDefaultMode(m);
			defaultMode = m;
			ui.notify('기본 작업 모드를 저장했습니다.');
		} catch {
			ui.notify('기본 작업 모드를 저장하지 못했습니다.', 'error');
		}
	}

	const pushLabel: Record<PushState, string> = {
		checking: '확인 중',
		unsupported: '이 브라우저에서는 지원하지 않습니다',
		denied: '브라우저에서 알림이 차단되어 있습니다',
		off: '꺼짐',
		on: '켜짐'
	};
</script>

<svelte:head>
	<title>설정 · Postudio</title>
</svelte:head>

<div class="settings-shell flex min-h-0 flex-1 flex-col">
	<h1 class="sr-only">설정</h1>
	<div class="settings-body flex min-h-0 flex-1">
		<nav aria-label="설정 메뉴" class="settings-nav">
			{#each [{ id: 'accounts-h', label: '계정', href: '#accounts-h' }, { id: 'routing-h', label: '모델 라우팅', href: '#routing-h' }, { id: 'push-h', label: '알림', href: '#push-h' }, { id: 'fleet-h', label: '호스트', href: '#fleet-h' }, { id: 'mode-h', label: '작업 기본값', href: '#mode-h' }] as item}
				<a href={item.href} aria-current={page.url.hash === item.href || (!page.url.hash && item.id === 'accounts-h') ? 'location' : undefined}>{item.label}</a>
			{/each}
		</nav>
		<div class="settings-content relative min-h-0 min-w-0 flex-1 overflow-y-auto overscroll-contain">
			<div class="settings-page mx-auto flex w-full max-w-[780px] flex-col gap-[18px] px-5 py-6 md:px-7 md:py-7">
	<AccountSettings />

	<section id="push-h" class="settings-card flex flex-col gap-3" aria-labelledby="push-title">
		<h2 id="push-title" class="text-[22px] font-semibold tracking-tight">알림</h2>
		<p class="text-sm">상태: {pushLabel[pushState]}</p>
		{#if pushState === 'off' || pushState === 'on'}
			<button
				type="button"
				disabled={busy}
				class="min-h-11 self-start rounded-xl border border-neutral-300 px-4 text-sm font-medium hover:bg-neutral-100 disabled:opacity-40 dark:border-neutral-700 dark:hover:bg-neutral-800"
				onclick={pushState === 'on' ? disable : enable}
			>
				{pushState === 'on' ? '알림 끄기' : '알림 켜기'}
			</button>
		{/if}
		{#if error}
			<p role="alert" class="text-sm text-red-700 dark:text-red-300">{error}</p>
		{/if}
	</section>

	<section id="fleet-h" class="settings-card flex flex-col gap-3" aria-labelledby="fleet-title">
<h2 id="fleet-title" class="text-[22px] font-semibold tracking-tight">호스트</h2>
<p class="text-sm text-neutral-600 dark:text-neutral-300">호스트 URL을 우선순위 순서대로 입력합니다. 채팅은 현재 호스트에 유지됩니다.</p>
<textarea aria-label="호스트 주소 목록" bind:value={fleetText} rows="3" spellcheck="false" placeholder="https://host.example" class="w-full rounded-xl border border-neutral-300 bg-transparent p-3 text-sm dark:border-neutral-700"></textarea>
{#if fleetError}<p role="alert" class="break-words text-sm text-red-700 dark:text-red-300">{fleetError}</p>{/if}
<div class="flex flex-wrap gap-2">
<button type="button" onclick={saveFleet} class="min-h-10 rounded-xl border border-neutral-300 px-4 text-sm dark:border-neutral-700">호스트 저장</button>
<button type="button" disabled={fleetBusy} onclick={() => void refreshFleet(true)} class="min-h-10 rounded-xl border border-neutral-300 px-4 text-sm disabled:opacity-40 dark:border-neutral-700">상태 확인</button>
</div>
<ul class="flex flex-col gap-1 text-sm" aria-live="polite">
{#each fleetHosts as host (host.origin)}<li>{host.online ? '온라인' : '오프라인'} · {host.origin}</li>{/each}
</ul>
</section>

<section id="mode-h" class="settings-card flex flex-col gap-3" aria-labelledby="mode-title">
		<h2 id="mode-title" class="text-[22px] font-semibold tracking-tight">작업 기본값</h2>
		<div class="flex flex-wrap gap-2" role="radiogroup" aria-labelledby="mode-title">
			{#each MODES as m (m)}
				<label
					class={[
						'flex min-h-11 cursor-pointer items-center rounded-xl border px-4 text-sm',
						defaultMode === m ? 'border-neutral-900 bg-neutral-100 font-medium dark:border-neutral-100 dark:bg-neutral-800' : 'border-neutral-300 hover:bg-neutral-100 dark:border-neutral-700 dark:hover:bg-neutral-800'
					]}
				>
					<input
						type="radio"
						name="default-mode"
						value={m}
						checked={defaultMode === m}
						onchange={() => setDefaultMode(m)}
						class="sr-only"
					/>
					{MODE_LABEL[m]}
				</label>
			{/each}
		</div>
	</section>
			</div>
		</div>
	</div>
</div>

<style>
.settings-nav { width: 220px; flex: none; display: flex; flex-direction: column; gap: 2px; padding: 20px 12px; border-right: 1px solid #ececef; }
.settings-nav a { min-height: 40px; display: flex; align-items: center; border-radius: 10px; padding: 0 12px; color: #55555a; font-size: 13px; text-decoration: none; }
.settings-nav a:hover { background: #f0f0f2; }
.settings-nav a[aria-current="location"] { background: #eef2fc; color: #1f48a3; font-weight: 600; }
.settings-content { scroll-behavior: smooth; }
.settings-content :global(section[id]), .settings-content :global(h2[id]) { scroll-margin-top: 16px; }
.settings-card { padding: 18px 20px; border: 1px solid #e6e6e8; border-radius: 16px; background: #fff; }
:global(.dark) .settings-card { border-color: #262626; background: #0a0a0a; }
@media (max-width: 700px) {
	.settings-body { flex-direction: column; }
	.settings-nav { width: 100%; flex-direction: row; overflow-x: auto; padding: 8px 12px; border-right: 0; border-bottom: 1px solid #ececef; }
	.settings-nav a { min-height: 44px; flex: none; white-space: nowrap; }
	.settings-page { padding-top: 20px; }
}
@media (max-width: 640px) { .settings-card { padding: 16px; } }
@media (prefers-reduced-motion: reduce) { .settings-content { scroll-behavior: auto; } }
</style>
