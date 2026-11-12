<script module lang="ts">
export type LoginStatus = 'starting' | 'pending' | 'completed' | 'failed' | 'cancelled' | 'expired';
export type LoginStart = { jobId: string; status: 'starting'; callbackMode: 'host_loopback' };
export type LoginPoll = { status: LoginStatus; authorizationUrl?: string };

const LOGIN_STATUSES: LoginStatus[] = ['starting', 'pending', 'completed', 'failed', 'cancelled', 'expired'];

export function isLocalLoginHost(hostname: string): boolean {
	const host = hostname.toLowerCase();
	return host === 'localhost' || host === '127.0.0.1' || host === '::1' || host === '[::1]';
}

export function safeAuthorizationHref(value?: string): string | null {
	if (!value) return null;
	try {
		const url = new URL(value);
		return url.protocol === 'https:' && !url.username && !url.password ? url.href : null;
	} catch {
		return null;
	}
}

export function parseLoginStart(value: unknown): LoginStart {
	if (!value || typeof value !== 'object') throw new Error('Invalid login response.');
	const response = value as Record<string, unknown>;
	if (typeof response.jobId !== 'string' || !response.jobId || response.status !== 'starting' || response.callbackMode !== 'host_loopback') {
		throw new Error('Invalid login response.');
	}
	return { jobId: response.jobId, status: 'starting', callbackMode: 'host_loopback' };
}

export function parseLoginPoll(value: unknown): LoginPoll {
	if (!value || typeof value !== 'object') throw new Error('Invalid login status response.');
	const response = value as Record<string, unknown>;
	if (!LOGIN_STATUSES.includes(response.status as LoginStatus) || (response.authorizationUrl !== undefined && typeof response.authorizationUrl !== 'string')) {
		throw new Error('Invalid login status response.');
	}
	return {
		status: response.status as LoginStatus,
		...(response.authorizationUrl === undefined ? {} : { authorizationUrl: response.authorizationUrl })
	};
}
</script>

<script lang="ts">
import { onMount } from 'svelte';
import { getJson, postJson } from '$lib/api';
import { ui } from '$lib/ui.svelte';
import AccountRouting from '$lib/components/AccountRouting.svelte';

type Provider = 'chatgpt' | 'claude' | 'grok' | 'openrouter';
type Quota = {
account_id: string;
checked_at: string;
fetched_at: string | null;
status: 'available' | 'unavailable' | 'unsupported';
remaining_percent: number | null;
};
type Account = {
id: string;
provider: Provider;
label: string;
legacy: boolean;
connected: boolean;
email: string | null;
connection_error: 'unreadable' | 'configuration_required' | null;
quota: Quota | null;
};
type AccountLogin = {
jobId: string;
status: LoginStatus;
authorizationUrl?: string;
message?: string;
canceling?: boolean;
startedAt: number;
};
const LOGIN_TIMEOUT_MS = 5 * 60 * 1000;
const WEB_LOGIN_PROVIDERS: Provider[] = ['chatgpt', 'claude', 'openrouter'];

const PROVIDERS: Provider[] = ['chatgpt', 'claude', 'grok', 'openrouter'];
const PROVIDER_LABEL: Record<Provider, string> = {
chatgpt: 'ChatGPT',
claude: 'Claude',
grok: 'Grok',
openrouter: 'OpenRouter'
};

let accounts = $state<Account[]>([]);
let accountId = $state('');
let provider = $state<Provider>('claude');
let label = $state('');
let editingId = $state('');
let renameLabel = $state('');
let usageById = $state<Record<string, Quota>>({});
let usageOpen = $state<Record<string, boolean>>({});
let loading = $state(false);
let busy = $state(false);
let error = $state('');
let localBrowser = $state(false);
let localLoginConfirmed = $state(false);
let loginJobs = $state<Record<string, AccountLogin>>({});
let loginTimers = new Map<string, ReturnType<typeof setTimeout>>();
let mounted = false;

onMount(() => {
mounted = true;
localBrowser = isLocalLoginHost(window.location.hostname);
void refresh();
return () => {
mounted = false;
for (const timer of loginTimers.values()) clearTimeout(timer);
loginTimers.clear();
};
});

async function loadAccounts(): Promise<boolean> {
loading = true;
try {
accounts = await getJson<Account[]>('/api/accounts');
return true;
} catch {
accounts = [];
error = '이 서버에서 계정 관리 API를 사용할 수 없습니다. 지원되는 Postudio node에서 다시 열어 주세요.';
return false;
} finally {
loading = false;
}
}

async function refresh(announce = false) {
if (busy) return;
busy = true;
error = '';
const loaded = await loadAccounts();
if (announce) ui.notify(loaded ? '계정 목록을 새로고침했습니다.' : '계정 목록을 새로고침하지 못했습니다.', loaded ? 'success' : 'error');
busy = false;
}

function setLoginJob(accountId: string, job: AccountLogin) {
loginJobs = { ...loginJobs, [accountId]: job };
}

function stopLoginTimer(accountId: string) {
const timer = loginTimers.get(accountId);
if (timer !== undefined) clearTimeout(timer);
loginTimers.delete(accountId);
}

function isActiveLogin(accountId: string): boolean {
const status = loginJobs[accountId]?.status;
return status === 'starting' || status === 'pending';
}

function canStartWebLogin(account: Account): boolean {
return localBrowser && localLoginConfirmed && WEB_LOGIN_PROVIDERS.includes(account.provider);
}

function scheduleLoginPoll(accountId: string, jobId: string, delay: number) {
stopLoginTimer(accountId);
loginTimers.set(accountId, setTimeout(() => {
loginTimers.delete(accountId);
void pollLogin(accountId, jobId);
}, delay));
}

async function startLogin(account: Account) {
if (!canStartWebLogin(account) || isActiveLogin(account.id)) return;
stopLoginTimer(account.id);
const startedAt = Date.now();
try {
const response = parseLoginStart(await postJson<unknown>('/api/accounts/login', { id: account.id }));
if (!mounted) return;
setLoginJob(account.id, { jobId: response.jobId, status: response.status, startedAt });
scheduleLoginPoll(account.id, response.jobId, 1000);
} catch (cause) {
if (!mounted) return;
ui.notify('로그인 요청을 시작하지 못했습니다.', 'error');
setLoginJob(account.id, {
jobId: '',
status: 'failed',
message: cause instanceof Error ? cause.message : '로그인 요청을 시작하지 못했습니다.',
startedAt
});
}
}

async function pollLogin(accountId: string, jobId: string) {
const current = loginJobs[accountId];
if (!mounted || !current || current.jobId !== jobId || !isActiveLogin(accountId)) return;
if (Date.now() - current.startedAt >= LOGIN_TIMEOUT_MS) {
setLoginJob(accountId, { ...current, status: 'expired', message: '로그인 대기 시간이 초과되었습니다. 다시 시도해 주세요.' });
ui.notify('로그인이 만료되었습니다.', 'error');
return;
}
try {
const response = parseLoginPoll(await getJson<unknown>(`/api/accounts/login/${encodeURIComponent(jobId)}`));
if (!mounted) return;
const latest = loginJobs[accountId];
if (!latest || latest.jobId !== jobId || !isActiveLogin(accountId)) return;
const next = { ...latest, status: response.status, authorizationUrl: response.authorizationUrl, message: undefined };
setLoginJob(accountId, next);
if (response.status !== latest.status) {
if (response.status === 'completed') ui.notify('로그인이 완료되었습니다.');
else if (response.status === 'failed') ui.notify('로그인에 실패했습니다.', 'error');
else if (response.status === 'expired') ui.notify('로그인이 만료되었습니다.', 'error');
else if (response.status === 'cancelled') ui.notify('로그인이 취소되었습니다.', 'error');
}
if (response.status === 'completed') {
stopLoginTimer(accountId);
await loadAccounts();
} else if (response.status === 'starting' || response.status === 'pending') {
scheduleLoginPoll(accountId, jobId, 1500);
} else {
stopLoginTimer(accountId);
}
} catch (cause) {
if (!mounted) return;
const latest = loginJobs[accountId];
if (!latest || latest.jobId !== jobId || !isActiveLogin(accountId)) return;
setLoginJob(accountId, { ...latest, message: cause instanceof Error ? `로그인 상태를 확인하지 못했습니다. ${cause.message}` : '로그인 상태를 확인하지 못했습니다.' });
scheduleLoginPoll(accountId, jobId, 3000);
}
}

async function cancelLogin(accountId: string) {
const current = loginJobs[accountId];
if (!current || !isActiveLogin(accountId) || current.canceling) return;
stopLoginTimer(accountId);
setLoginJob(accountId, { ...current, canceling: true, message: undefined });
try {
await postJson<unknown>(`/api/accounts/login/${encodeURIComponent(current.jobId)}/cancel`, {});
if (!mounted) return;
const latest = loginJobs[accountId];
if (latest?.jobId === current.jobId) {
setLoginJob(accountId, { ...latest, status: 'cancelled', canceling: false });
if (latest.status !== 'cancelled') ui.notify('로그인이 취소되었습니다.', 'error');
}
} catch (cause) {
if (!mounted) return;
const latest = loginJobs[accountId];
if (latest?.jobId === current.jobId) {
setLoginJob(accountId, { ...latest, canceling: false, message: cause instanceof Error ? `취소 요청에 실패했습니다. ${cause.message}` : '취소 요청에 실패했습니다.' });
ui.notify('로그인 취소 요청에 실패했습니다.', 'error');
scheduleLoginPoll(accountId, current.jobId, 1500);
}
}
}

async function mutate(path: string, body: unknown, operation: string) {
if (busy) return;
busy = true;
error = '';
try {
await postJson<{ ok: boolean }>(path, body);
await loadAccounts();
if (!error) ui.notify(`${operation} 완료`);
} catch {
error = '계정 작업을 완료하지 못했습니다. 호스트의 계정 상태와 입력을 확인해 주세요.';
ui.notify(`${operation} 실패`, 'error');
} finally {
busy = false;
}
}

async function addAccount(event: SubmitEvent) {
event.preventDefault();
await mutate('/api/accounts/add', { id: accountId, provider, label }, '계정 추가');
if (!error) {
accountId = '';
label = '';
}
}

function beginRename(account: Account) {
editingId = account.id;
renameLabel = account.label;
}

async function saveRename(event: SubmitEvent, id: string) {
event.preventDefault();
await mutate('/api/accounts/rename', { id, label: renameLabel }, '계정 이름 변경');
if (!error) editingId = '';
}

async function logout(account: Account) {
if (!window.confirm(`“${account.label}” 계정에서 로그아웃할까요?`)) return;
await mutate('/api/accounts/logout', { id: account.id }, '로그아웃');
}

async function remove(account: Account) {
if (!window.confirm(`“${account.label}” 계정을 제거할까요? 이 작업은 되돌릴 수 없습니다.`)) return;
await mutate('/api/accounts/remove', { id: account.id }, '계정 제거');
}

function loginStatusText(status: LoginStatus): string {
switch (status) {
case 'starting': return '로그인 요청을 시작했습니다.';
case 'pending': return '인증을 기다리는 중입니다.';
case 'completed': return '로그인이 완료되었습니다.';
case 'failed': return '로그인에 실패했습니다.';
case 'cancelled': return '로그인이 취소되었습니다.';
case 'expired': return '로그인이 만료되었습니다.';
}
}

async function toggleUsage(id: string) {
if (Object.hasOwn(usageOpen, id) && usageOpen[id]) {
usageOpen = { ...usageOpen, [id]: false };
return;
}
if (busy) return;
busy = true;
error = '';
try {
usageById = { ...usageById, [id]: await postJson<Quota>('/api/accounts/usage', { id }) };
usageOpen = { ...usageOpen, [id]: true };
} catch {
error = '사용량 정보를 불러오지 못했습니다. 호스트 계정 API를 확인해 주세요.';
} finally {
busy = false;
}
}
</script>

<section id="accounts-h" class="settings-card flex flex-col gap-4 rounded-2xl border border-neutral-200 bg-white p-4 dark:border-neutral-800 dark:bg-neutral-950 sm:p-5" aria-labelledby="accounts-title">
<div class="flex flex-wrap items-end justify-between gap-3">
<div class="min-w-0 flex-1"><h2 id="accounts-title" class="text-[22px] font-semibold tracking-tight">계정</h2><p class="mt-1 text-[13.5px] leading-relaxed text-neutral-600 dark:text-neutral-400">모델 제공자 계정을 연결하고 주간 사용량을 확인해요.</p></div>
<button
 type="button"
 disabled={busy}
 class="min-h-10 rounded-xl border border-neutral-300 px-3 text-sm hover:bg-neutral-100 disabled:opacity-40 dark:border-neutral-700 dark:hover:bg-neutral-800"
 onclick={() => void refresh(true)}
>
새로고침
</button>
</div>
<p class="text-sm text-neutral-600 dark:text-neutral-400">계정 목록과 연결 상태는 현재 Postudio node 호스트에서 가져옵니다. 로그인 정보는 브라우저에 저장하지 않습니다. 원격 호스트의 브라우저 로그인은 지원하지 않으며, 해당 node 호스트에서 <code>distill accounts login &lt;id&gt;</code>를 실행하세요.</p>
{#if localBrowser}
<div class="flex flex-col gap-2 rounded-xl border border-neutral-200 p-4 dark:border-neutral-800">
	<p class="text-sm">이 로컬 페이지는 node 호스트에서 OAuth 로그인을 시작합니다. 인증 링크를 직접 열어 완료하세요. 자동으로 새 창을 열지 않습니다.</p>
	<label class="flex min-h-11 items-center gap-3 text-sm">
		<input type="checkbox" bind:checked={localLoginConfirmed} />
		이 브라우저가 node와 같은 컴퓨터에서 실행 중
	</label>
</div>
{/if}
{#if !localBrowser}
<p class="text-sm text-amber-700 dark:text-amber-300">이 페이지 주소가 localhost가 아니므로 브라우저 로그인은 사용할 수 없습니다. node 호스트에서 CLI 로그인을 실행하세요.</p>
{/if}

{#if error}
<p role="alert" class="text-sm text-red-700 dark:text-red-300">{error}</p>
{/if}
{#if loading}
<p role="status" class="text-sm text-neutral-500">계정 목록을 불러오는 중입니다.</p>
{/if}

<form class="grid gap-3 rounded-xl border border-neutral-200 p-4 dark:border-neutral-800 sm:grid-cols-2" onsubmit={addAccount}>
<h3 class="text-sm font-medium sm:col-span-2">계정 추가</h3>
<label class="flex flex-col gap-1 text-sm">
<span>계정 ID</span>
<input
 bind:value={accountId}
 required
 maxlength="64"
 pattern={'[A-Za-z0-9_][A-Za-z0-9_-]{0,63}'}
 autocomplete="off"
 class="min-h-10 rounded-lg border border-neutral-300 bg-transparent px-3 dark:border-neutral-700"
/>
</label>
<label class="flex flex-col gap-1 text-sm">
<span>제공자</span>
<select bind:value={provider} class="min-h-10 rounded-lg border border-neutral-300 bg-transparent px-3 dark:border-neutral-700">
{#each PROVIDERS as item (item)}
<option value={item}>{PROVIDER_LABEL[item]}</option>
{/each}
</select>
</label>
<label class="flex flex-col gap-1 text-sm sm:col-span-2">
<span>표시 이름</span>
<input
 bind:value={label}
 required
 maxlength="120"
 autocomplete="off"
 class="min-h-10 rounded-lg border border-neutral-300 bg-transparent px-3 dark:border-neutral-700"
/>
</label>
<button
 type="submit"
 disabled={busy}
 class="min-h-11 justify-self-start rounded-xl bg-neutral-900 px-4 text-sm font-medium text-white disabled:opacity-40 dark:bg-neutral-100 dark:text-neutral-900"
>
계정 추가
</button>
</form>

{#if accounts.length === 0 && !loading && !error}
<p class="text-sm text-neutral-500">등록된 계정이 없습니다.</p>
{/if}
<ul class="flex flex-col gap-3">
{#each accounts as account (account.id)}
<li class="flex flex-col gap-3 rounded-2xl border border-neutral-200 bg-white p-4 sm:p-5 dark:border-neutral-800 dark:bg-neutral-950">
<div class="flex flex-wrap items-start justify-between gap-3">
<div class="min-w-0 flex-1">
<p class="break-words text-sm font-medium">{account.label} <span class="font-normal text-neutral-500">({PROVIDER_LABEL[account.provider]})</span></p>
<p class="break-all font-mono text-xs text-neutral-500">{account.id}</p>
<p class="text-sm">{account.connected ? '연결됨' : '연결 안 됨'}{account.email ? ` · ${account.email}` : ''}</p>
{#if account.connection_error === 'unreadable'}
<p class="text-xs text-amber-700 dark:text-amber-300">저장된 연결 정보를 읽을 수 없습니다.</p>
{:else if account.connection_error === 'configuration_required'}
<p class="text-xs text-amber-700 dark:text-amber-300">호스트에 계정 설정이 필요합니다.</p>
{/if}
{#if account.quota?.remaining_percent != null}
<p class="text-xs text-neutral-500">남은 사용량 {Math.round(account.quota.remaining_percent)}%</p>
{/if}
{#if account.provider === 'grok'}
<p class="text-xs text-amber-700 dark:text-amber-300">Grok 웹 로그인은 지원하지 않습니다. node 호스트에서 <code>distill accounts login {account.id}</code>를 실행하세요.</p>
{/if}
</div>
<div class="flex flex-wrap gap-2">
<button type="button" disabled={busy || !canStartWebLogin(account) || isActiveLogin(account.id)} title={account.provider === 'grok' ? 'Grok은 브라우저 로그인을 지원하지 않습니다. CLI를 사용하세요.' : undefined} class="min-h-10 rounded-lg border border-neutral-300 px-3 text-xs disabled:opacity-40 dark:border-neutral-700" onclick={() => void startLogin(account)}>{account.provider === 'grok' ? 'CLI 로그인' : account.connected ? '다시 로그인' : '연결'}</button>
<button type="button" disabled={busy} class="min-h-10 rounded-lg border border-neutral-300 px-3 text-xs disabled:opacity-40 dark:border-neutral-700" onclick={() => beginRename(account)}>이름 변경</button>
<button type="button" disabled={busy} class="min-h-10 rounded-lg border border-neutral-300 px-3 text-xs disabled:opacity-40 dark:border-neutral-700" onclick={() => toggleUsage(account.id)}>{Object.hasOwn(usageOpen, account.id) && usageOpen[account.id] ? '사용량 숨기기' : '사용량'}</button>
<button type="button" disabled={busy} class="min-h-10 rounded-lg border border-neutral-300 px-3 text-xs disabled:opacity-40 dark:border-neutral-700" onclick={() => logout(account)}>로그아웃</button>
<button
 type="button"
 disabled={busy || account.legacy}
 title={account.legacy ? '기본 계정은 제거할 수 없습니다.' : undefined}
 class="min-h-10 rounded-lg border border-red-300 px-3 text-xs text-red-700 disabled:opacity-40 dark:border-red-900 dark:text-red-300"
 onclick={() => remove(account)}
>
제거
</button>
</div>
</div>
{#if loginJobs[account.id]}
	{@const job = loginJobs[account.id]!}
	<div class="flex flex-col gap-2 rounded-lg bg-neutral-100 p-3 dark:bg-neutral-900">
		<p role="status" class="text-sm">{loginStatusText(job.status)} {job.message ?? ''}</p>
		{#if isActiveLogin(account.id)}
			{@const authorizationHref = safeAuthorizationHref(job.authorizationUrl)}
			{#if authorizationHref}
				<a href={authorizationHref} target="_blank" rel="noopener noreferrer" class="min-h-10 self-start content-center rounded-lg border border-neutral-300 px-3 text-sm underline dark:border-neutral-700">OAuth 인증 링크 열기</a>
			{:else if job.status === 'pending'}
				<p class="text-sm text-neutral-600 dark:text-neutral-400">안전한 HTTPS 인증 링크를 기다리는 중입니다.</p>
			{/if}
			<button type="button" disabled={job.canceling} class="min-h-10 self-start rounded-lg border border-neutral-300 px-3 text-sm disabled:opacity-40 dark:border-neutral-700" onclick={() => void cancelLogin(account.id)}>{job.canceling ? '취소 중…' : '로그인 취소'}</button>
		{/if}
	</div>
{/if}
{#if editingId === account.id}
<form class="flex flex-wrap gap-2" onsubmit={(event) => saveRename(event, account.id)}>
<label class="sr-only" for="rename-{account.id}">표시 이름</label>
<input id="rename-{account.id}" bind:value={renameLabel} required maxlength="120" class="min-h-10 min-w-0 flex-1 rounded-lg border border-neutral-300 bg-transparent px-3 text-sm dark:border-neutral-700" />
<button type="submit" disabled={busy} class="min-h-10 rounded-lg bg-neutral-900 px-3 text-xs text-white disabled:opacity-40 dark:bg-neutral-100 dark:text-neutral-900">저장</button>
<button type="button" disabled={busy} class="min-h-10 rounded-lg border border-neutral-300 px-3 text-xs disabled:opacity-40 dark:border-neutral-700" onclick={() => (editingId = '')}>취소</button>
</form>
{/if}
{#if Object.hasOwn(usageOpen, account.id) && usageOpen[account.id] && Object.hasOwn(usageById, account.id)}
<pre class="overflow-x-auto rounded-lg bg-neutral-100 p-3 text-xs dark:bg-neutral-900">{JSON.stringify(usageById[account.id], null, 2)}</pre>
{/if}
</li>
{/each}
</ul>
</section>
<AccountRouting {accounts} />
