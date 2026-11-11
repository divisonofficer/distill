<script lang="ts">
	import { postJson, getJson } from '$lib/api';
	import { loadDefaultMode, MODES, MODE_LABEL, saveDefaultMode } from '$lib/prefs';
	import type { Mode } from '$lib/types';

	type PushState = 'checking' | 'unsupported' | 'denied' | 'off' | 'on';

	let pushState = $state<PushState>('checking');
	let busy = $state(false);
	let error = $state('');
	let defaultMode = $state<Mode>(loadDefaultMode());

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
		} catch {
			error = '알림을 켜지 못했습니다.';
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
		} catch {
			error = '알림을 끄지 못했습니다.';
		} finally {
			busy = false;
		}
	}

	function setDefaultMode(m: Mode) {
		defaultMode = m;
		saveDefaultMode(m);
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

<div class="flex flex-col gap-6 p-4">
	<h1 class="text-base font-semibold">설정</h1>

	<section class="flex flex-col gap-3" aria-labelledby="push-h">
		<h2 id="push-h" class="text-sm font-semibold">푸시 알림</h2>
		<p class="text-sm">상태: {pushLabel[pushState]}</p>
		{#if pushState === 'off' || pushState === 'on'}
			<button
				type="button"
				disabled={busy}
				class="min-h-11 rounded-lg border border-neutral-400 px-4 text-sm font-medium disabled:opacity-40"
				onclick={pushState === 'on' ? disable : enable}
			>
				{pushState === 'on' ? '알림 끄기' : '알림 켜기'}
			</button>
		{/if}
		{#if error}
			<p role="alert" class="text-sm text-red-700 dark:text-red-300">{error}</p>
		{/if}
	</section>

	<section class="flex flex-col gap-3" aria-labelledby="mode-h">
		<h2 id="mode-h" class="text-sm font-semibold">새 작업 기본 권한 모드</h2>
		<div class="flex flex-wrap gap-2" role="radiogroup" aria-labelledby="mode-h">
			{#each MODES as m (m)}
				<label
					class={[
						'flex min-h-11 cursor-pointer items-center rounded-lg border px-4 text-sm',
						defaultMode === m ? 'border-blue-600 bg-blue-50 dark:bg-blue-950' : 'border-neutral-300 dark:border-neutral-700'
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
