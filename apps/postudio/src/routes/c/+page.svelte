<script lang="ts">
import { afterNavigate, goto } from '$app/navigation';
import { onMount } from 'svelte';
import { getJson, postJson } from '$lib/api';
import { chatDisplayTitle, chatPreviewTitles, chatTitleOverrides, parseChatList, parseCreatedChat, type ChatSummary } from '$lib/chats';
import Icon from '$lib/components/Icon.svelte';

let chats = $state.raw<ChatSummary[]>([]);
let loading = $state(true);
let loaded = $state(false);
let creating = $state(false);
let error = $state('');
let requestId = 0;
let search = $state('');
const filteredChats = $derived(chats.filter((chat) => chatDisplayTitle(chat, [], $chatPreviewTitles, $chatTitleOverrides).toLocaleLowerCase().includes(search.trim().toLocaleLowerCase())));

async function refreshChats() {
const request = ++requestId;
if (!loaded) loading = true;
try {
const next = parseChatList(await getJson<unknown>('/api/chats'));
if (request !== requestId) return;
chats = next;
loaded = true;
error = '';
} catch (cause) {
if (request === requestId) error = cause instanceof Error ? cause.message : '알 수 없는 오류';
} finally {
if (request === requestId) loading = false;
}
}

async function createChat() {
if (creating) return;
creating = true;
error = '';
try {
const chat = parseCreatedChat(await postJson<unknown>('/api/chats', {}));
await goto(`/c/${encodeURIComponent(chat.chatId)}`);
} catch (cause) {
error = cause instanceof Error ? cause.message : '알 수 없는 오류';
} finally {
creating = false;
}
}

afterNavigate(() => void refreshChats());
onMount(() => {
const timer = window.setInterval(() => void refreshChats(), 15000);
return () => window.clearInterval(timer);
});
</script>

<svelte:head>
<title>채팅 · Postudio</title>
</svelte:head>

<svelte:window onfocus={() => void refreshChats()} />

<section class="mx-auto flex min-h-0 w-full max-w-[920px] flex-1 flex-col gap-5 overflow-y-auto px-4 py-5 md:px-6 md:py-8">
<header class="flex flex-wrap items-center justify-between gap-3">
<div>
<h1 class="text-[28px] font-bold tracking-tight">채팅</h1>
<p class="mt-1 text-sm text-neutral-600 dark:text-neutral-300">메모와 아이디어를 모으고, 분기로 이어가세요.</p>
</div>
<button
 type="button"
 disabled={creating}
 class="flex min-h-11 shrink-0 items-center gap-2 rounded-xl bg-neutral-900 px-4 text-sm font-medium text-white hover:bg-neutral-700 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-neutral-500 disabled:opacity-50 dark:bg-neutral-100 dark:text-neutral-900 dark:hover:bg-neutral-300"
 onclick={createChat}
>
<Icon name="new" class="size-4" />
{creating ? '채팅 만드는 중…' : '새 채팅'}
</button>
</header>

<label class="sr-only" for="chat-search">채팅 검색</label><input id="chat-search" type="search" bind:value={search} placeholder="채팅 검색" class="min-h-12 rounded-xl border border-neutral-300 bg-white px-4 text-sm dark:border-neutral-700 dark:bg-neutral-900" />
{#if error}
<div role="alert" class="flex flex-wrap items-center justify-between gap-3 rounded-xl border border-red-300 bg-red-50 px-4 py-3 text-sm text-red-900 dark:border-red-800 dark:bg-red-950 dark:text-red-100">
<span>채팅 목록을 불러오지 못했습니다. {error}</span>
<button type="button" class="min-h-10 rounded-lg px-3 font-medium underline underline-offset-4" onclick={() => void refreshChats()}>다시 시도</button>
</div>
{/if}

{#if loading && !loaded}
<p role="status" class="text-sm text-neutral-600 dark:text-neutral-300">채팅 목록을 불러오는 중입니다.</p>
{:else if loaded && chats.length === 0}
<div class="rounded-2xl border border-dashed border-neutral-300 px-5 py-10 text-center dark:border-neutral-700">
<Icon name="chat" class="mx-auto mb-3 size-11 rounded-xl bg-neutral-100 p-3 text-neutral-500 dark:bg-neutral-800" />
<p class="text-[15px] font-semibold">아직 대화가 없어요</p>
<p class="mx-auto mt-2 max-w-xs text-[13px] leading-relaxed text-neutral-600 dark:text-neutral-300">질문이나 TXT/MD 파일로 첫 대화를 시작해 보세요.</p>
<button type="button" disabled={creating} class="mt-4 min-h-10 rounded-lg bg-neutral-900 px-4 text-[13px] font-semibold text-white disabled:opacity-50 dark:bg-neutral-100 dark:text-neutral-900" onclick={createChat}>{creating ? '채팅 만드는 중…' : '새 채팅'}</button>
</div>
{:else if filteredChats.length === 0}
<div role="status" class="rounded-xl border border-neutral-200 px-5 py-9 text-center dark:border-neutral-700">
<Icon name="chat" class="mx-auto mb-3 size-11 rounded-xl bg-neutral-100 p-3 text-neutral-500 dark:bg-neutral-800" />
<p class="break-words text-[15px] font-semibold">“{search.trim()}”와 맞는 대화가 없어요</p>
<p class="mx-auto mt-2 max-w-xs text-[13px] leading-relaxed text-neutral-600 dark:text-neutral-300">현재는 대화 제목에서 찾아요. 다른 단어로 검색해 보세요.</p>
<button type="button" class="mt-3 min-h-10 rounded-lg border border-neutral-300 px-3 text-[13px] font-semibold dark:border-neutral-700" onclick={() => { search = ''; document.getElementById('chat-search')?.focus(); }}>검색어 지우기</button>
</div>
{:else if filteredChats.length > 0}
<ul class="flex flex-col gap-2" aria-label="저장된 채팅">
{#each filteredChats as chat (chat.chatId)}
<li>
<a
 href="/c/{encodeURIComponent(chat.chatId)}"
 class="flex min-h-[68px] items-center gap-3 rounded-xl bg-[#f6f6f7] px-3 py-3 hover:bg-neutral-200/70 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-neutral-500 dark:bg-neutral-900 dark:hover:bg-neutral-800"
>
<Icon name="chat" class="size-10 shrink-0 rounded-xl bg-neutral-100 p-3 text-neutral-600 dark:text-neutral-300" />
<span class="min-w-0 flex-1 truncate text-sm font-medium">{chatDisplayTitle(chat, [], $chatPreviewTitles, $chatTitleOverrides)}</span>
<Icon name="chevron" class="size-4 shrink-0 -rotate-90 text-neutral-400" />
</a>
</li>
{/each}
</ul>
{/if}
</section>
