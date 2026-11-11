<script lang="ts">
import { onMount } from 'svelte';
import { getJson, postJson } from '$lib/api';

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

onMount(() => {
void refresh();
});

async function loadAccounts() {
loading = true;
try {
accounts = await getJson<Account[]>('/api/accounts');
} catch {
accounts = [];
error = '이 서버에서 계정 관리 API를 사용할 수 없습니다. 지원되는 Postudio node에서 다시 열어 주세요.';
} finally {
loading = false;
}
}

async function refresh() {
if (busy) return;
busy = true;
error = '';
await loadAccounts();
busy = false;
}

async function mutate(path: string, body: unknown) {
if (busy) return;
busy = true;
error = '';
try {
await postJson<{ ok: boolean }>(path, body);
await loadAccounts();
} catch {
error = '계정 작업을 완료하지 못했습니다. 호스트의 계정 상태와 입력을 확인해 주세요.';
} finally {
busy = false;
}
}

async function addAccount(event: SubmitEvent) {
event.preventDefault();
await mutate('/api/accounts/add', { id: accountId, provider, label });
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
await mutate('/api/accounts/rename', { id, label: renameLabel });
if (!error) editingId = '';
}

async function logout(account: Account) {
if (!window.confirm(`“${account.label}” 계정에서 로그아웃할까요?`)) return;
await mutate('/api/accounts/logout', { id: account.id });
}

async function remove(account: Account) {
if (!window.confirm(`“${account.label}” 계정을 제거할까요? 이 작업은 되돌릴 수 없습니다.`)) return;
await mutate('/api/accounts/remove', { id: account.id });
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

<section class="flex flex-col gap-4" aria-labelledby="accounts-h">
<div class="flex items-center justify-between gap-3">
<h2 id="accounts-h" class="text-sm font-semibold">호스트 계정</h2>
<button
 type="button"
 disabled={busy}
 class="min-h-10 rounded-xl border border-neutral-300 px-3 text-sm hover:bg-neutral-100 disabled:opacity-40 dark:border-neutral-700 dark:hover:bg-neutral-800"
 onclick={refresh}
>
새로고침
</button>
</div>
<p class="text-sm text-neutral-600 dark:text-neutral-400">계정 목록과 연결 상태는 현재 Postudio node 호스트에서 가져옵니다. 로그인은 웹에서 지원하지 않습니다. 해당 호스트에서 <code>distill accounts login &lt;id&gt;</code>를 실행하세요. 로그인 정보는 브라우저에 저장하지 않습니다.</p>
<p class="text-sm text-neutral-600 dark:text-neutral-400">모델별 계정 할당과 자동 라우팅 설정은 아직 구현되지 않았습니다.</p>

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
<li class="flex flex-col gap-3 rounded-xl border border-neutral-200 p-4 dark:border-neutral-800">
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
</div>
<div class="flex flex-wrap gap-2">
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
