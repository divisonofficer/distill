<script lang="ts">
import { goto } from '$app/navigation';
import { postJson } from '$lib/api';
import Icon from '$lib/components/Icon.svelte';
import ModelSelect from '$lib/components/ModelSelect.svelte';
import { loadDefaultMode, loadRecent, MODES, MODE_LABEL, rememberCwd } from '$lib/prefs';
import type { Mode, NewSession } from '$lib/types';

let cwd = $state('');
let worktree = $state(false);
let wtName = $state('');
let mode = $state<Mode>(loadDefaultMode());
let modelId = $state('');
let prompt = $state('');
let error = $state('');
let busy = $state(false);
let recent = $state<string[]>(loadRecent());

async function submit(e: SubmitEvent) {
e.preventDefault();
error = '';
const dir = cwd.trim();
if (!dir) {
error = '작업 경로를 입력하세요.';
return;
}
const body: NewSession = { cwd: dir, mode };
if (modelId) body.modelId = modelId;
if (worktree) body.worktree = { name: wtName.trim() };
if (prompt.trim()) body.prompt = prompt.trim();
busy = true;
try {
const res = await postJson<{ sessionId: string }>('/api/sessions', body);
rememberCwd(dir);
await goto(`/s/${encodeURIComponent(res.sessionId)}`);
} catch (cause) {
const detail = cause instanceof Error ? cause.message : '알 수 없는 오류';
error = `세션을 만들지 못했습니다. ${detail}`;
} finally {
busy = false;
}
}

const field =
'min-h-11 w-full rounded-xl border border-neutral-300 bg-white px-3 text-sm outline-none focus:border-neutral-500 dark:border-neutral-700 dark:bg-neutral-900 dark:focus:border-neutral-500';
const label = 'text-sm font-medium';
</script>

<svelte:head>
<title>새 작업 · Postudio</title>
</svelte:head>

<div class="mx-auto w-full max-w-2xl p-4 md:p-8">
<form class="flex flex-col gap-6" onsubmit={submit}>
<h1 class="text-2xl font-semibold tracking-tight">새 작업</h1>

<div class="flex flex-col gap-2">
<label for="cwd" class={label}>작업 경로</label>
<input
id="cwd"
bind:value={cwd}
list="recent-cwds"
autocomplete="off"
autocapitalize="off"
spellcheck="false"
placeholder="/home/user/project"
class="{field} font-mono"
/>
<datalist id="recent-cwds">
{#each recent as r (r)}
<option value={r}></option>
{/each}
</datalist>
{#if recent.length > 0}
<div class="flex flex-wrap gap-2">
{#each recent as r (r)}
<button
type="button"
class="min-h-9 rounded-full border border-neutral-300 px-3 font-mono text-xs hover:bg-neutral-100 dark:border-neutral-700 dark:hover:bg-neutral-800"
onclick={() => (cwd = r)}
>
{r}
</button>
{/each}
</div>
{/if}
</div>

<div class="flex flex-col gap-3">
<label class="flex min-h-11 items-center gap-3 text-sm font-medium">
<input
 type="checkbox"
 bind:checked={worktree}
 disabled
 aria-describedby="worktree-unsupported"
 class="size-5 accent-neutral-900 disabled:cursor-not-allowed disabled:opacity-50 dark:accent-neutral-100"
/>
worktree로 격리
</label>
<p id="worktree-unsupported" class="-mt-1 text-sm text-neutral-500 dark:text-neutral-400">
서버가 아직 worktree 격리를 지원하지 않습니다. 지원될 때까지 사용할 수 없습니다.
</p>
{#if worktree}
<label for="wt-name" class={label}>worktree 이름</label>
<input id="wt-name" bind:value={wtName} autocomplete="off" placeholder="비우면 자동 이름" class={field} />
{/if}
</div>

<ModelSelect id="new-model" bind:value={modelId} initialFromCatalog />

<div class="flex flex-col gap-2">
<label for="mode" class={label}>권한 모드</label>
<select id="mode" bind:value={mode} class={field}>
{#each MODES as m (m)}
<option value={m}>{MODE_LABEL[m]}</option>
{/each}
</select>
</div>

<div class="flex flex-col gap-2">
<label for="prompt" class={label}>프롬프트</label>
<textarea
id="prompt"
bind:value={prompt}
rows="5"
class="w-full rounded-2xl border border-neutral-300 bg-white p-3 text-[15px] leading-6 outline-none focus:border-neutral-500 dark:border-neutral-700 dark:bg-neutral-900"
></textarea>
</div>

{#if error}
<p role="alert" class="text-sm text-red-700 dark:text-red-300">{error}</p>
{/if}

<button
type="submit"
disabled={busy}
class="flex min-h-11 items-center justify-center gap-2 self-start rounded-xl bg-neutral-900 px-5 text-sm font-medium text-white disabled:opacity-40 dark:bg-neutral-100 dark:text-neutral-900"
>
<Icon name="send" class="size-4" />
시작
</button>
</form>
</div>
