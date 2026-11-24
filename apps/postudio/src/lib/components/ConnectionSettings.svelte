<script lang="ts">
import { onMount } from 'svelte';
import { getJson, postJson } from '$lib/api';
import { configuredHosts, invitationExpiry, invitationPreview, pairingOrigin, safeOrigin, savePairedHost } from '$lib/fleet';

let { onpaired }: { onpaired: () => void } = $props();
type Status = { identity: { nodeId: string; publicKey: string }; peers: { nodeId: string; origin: string }[]; sync: { available: boolean; peerCount: number; lastAttemptUnixSeconds: number | null; lastSuccessUnixSeconds: number | null; lastError: string | null } };
type Candidate = { url: string; online: boolean };
let status = $state<Status | null>(null);
let statusError = $state('');
let error = $state('');
let notice = $state('');
let pending = $state('');
let invitation = $state('');
let pasted = $state('');
let preview = $derived(invitationPreview(pasted));
let expires = $state(0);
let candidates = $state<Candidate[] | null>(null);
let discoveryError = $state('');
let mounted = false;
let expiryTimer: ReturnType<typeof setTimeout> | undefined;

function clearInvitation() { invitation = ''; expires = 0; clearTimeout(expiryTimer); }
async function refresh() {
 try {
  const data = await getJson<Status>('/api/connections');
  if (!mounted) return;
  if (invitation && status && data.peers.some((peer) => !status?.peers.some((old) => old.nodeId === peer.nodeId))) clearInvitation();
  status = data; statusError = '';
  let added = false;
  for (const peer of data.peers) {
   if (!configuredHosts().some((host) => host.origin === peer.origin)) {
    try { savePairedHost(peer.origin); added = true; }
    catch { error = '연결된 기기의 호스트 주소를 저장하지 못했어요.'; }
   }
  }
  if (added) onpaired();
 }
 catch { if (mounted) statusError = '연결 상태를 불러오지 못했어요.'; }
}
onMount(() => {
 mounted = true;
 void refresh();
 const timer = setInterval(() => void refresh(), 10000);
 return () => { mounted = false; clearInterval(timer); clearInvitation(); pasted = ''; };
});
const warning = '연결하면 기존 및 앞으로의 일반 채팅을 공유하고, 서로의 연결된 모델 계정으로 일반 채팅 응답을 요청할 수 있어요. 모델 사용량은 계정이 연결된 호스트에 반영됩니다. 신뢰하는 기기와만 연결하세요.';
async function generate() {
 error = ''; notice = '';
 const origin = pairingOrigin(location.origin);
 if (!origin) { error = '초대하려면 HTTPS로 배포하세요. 로컬 루프백 주소만 HTTP를 허용해요.'; return; }
 if (!confirm(`${warning}\n초대를 생성할까요?`)) return;
 clearInvitation(); pending = 'invite';
 try {
  const result = await postJson<{ invitation: string; expiresAt: number }>('/api/connections/invite', { origin });
  if (!mounted) return;
  expires = invitationExpiry(result.expiresAt);
  if (!Number.isFinite(expires) || expires <= Date.now()) { error = '초대가 만료됐어요. 다시 생성해 주세요.'; return; }
  invitation = result.invitation;
  expiryTimer = setTimeout(() => { clearInvitation(); notice = '초대가 만료됐어요.'; }, Math.min(expires - Date.now(), 600000));
 } catch { if (mounted) error = '초대를 생성하지 못했어요.'; }
 finally { pending = ''; }
}
async function copy() {
 try { await navigator.clipboard.writeText(invitation); notice = '초대를 복사했어요.'; }
 catch { error = '복사하지 못했어요. 초대를 직접 선택해 복사해 주세요.'; }
}
async function join() {
 error = ''; notice = '';
 if (!pasted.trim()) { error = '상대 기기의 초대를 붙여 넣어 주세요.'; return; }
 const origin = pairingOrigin(location.origin);
 if (!origin) { error = '연결하려면 HTTPS로 배포하세요. 로컬 루프백 주소만 HTTP를 허용해요.'; return; }
 const remote = invitationPreview(pasted);
 if (!remote) { error = '초대가 잘못됐거나 만료됐어요. 새 초대를 받아 주세요.'; return; }
 if (!confirm(`${warning}\n상대 웹 주소: ${remote.origin}\n상대 기기 지문: ${remote.nodeId}\n이 기기와 연결할까요?`)) return;
 pending = 'join';
 try {
  const result = await postJson<{ nodeId: string; origin: string }>('/api/connections/join', { invitation: pasted.trim(), origin });
  if (!mounted) return;
  pasted = ''; clearInvitation();
  try { savePairedHost(result.origin); onpaired(); notice = '기기를 연결하고 호스트 목록에 저장했어요.'; }
  catch { error = '연결했지만 호스트 주소를 저장하지 못했어요. 호스트 설정을 확인해 주세요.'; }
  await refresh();
 } catch { if (mounted) error = '연결을 완료하지 못했어요. 한쪽만 설정됐을 수 있으니 두 기기에서 연결 상태를 확인하고 새 초대를 생성해 주세요.'; }
 finally { pending = ''; }
}
async function remove(nodeId: string) {
 if (!confirm('연결을 해제하면 채팅 동기화와 브라우저 접근 권한이 취소돼요. 이미 복사된 채팅 기록과 호스트 목록의 주소는 유지돼요. 계속할까요?')) return;
 pending = nodeId; error = ''; notice = '';
 try { await postJson('/api/connections/remove', { nodeId }); notice = '연결을 해제했어요.'; await refresh(); }
 catch { if (mounted) error = '연결을 해제하지 못했어요.'; }
 finally { pending = ''; }
}
async function discover() {
 pending = 'discover'; discoveryError = ''; candidates = null;
 try { candidates = await getJson<Candidate[]>('/api/fleet'); }
 catch { discoveryError = 'Tailscale 검색을 사용할 수 없거나 실패했어요.'; }
 finally { pending = ''; }
}
function displayTime(value: number) { return new Date(value * 1000).toLocaleString(); }
</script>

<section id="connections-h" class="settings-card flex flex-col gap-3" aria-labelledby="connections-title">
<h2 id="connections-title" class="text-[22px] font-semibold tracking-tight">연결 및 채팅 공유</h2>
<p class="text-sm">코딩 세션은 소유 호스트에서 실행돼요. 연결된 기기는 일반 채팅을 복제하지만 작업 공간 파일은 공유하지 않아요.</p>
<p class="text-sm">각 기기의 설정은 반드시 해당 기기 자신의 웹 주소에서 열어 연결해야 해요. 다른 기기의 설정 화면에서 대신 등록할 수 없어요.</p>
<p class="text-sm">{warning} 초대는 한 번만 사용할 수 있고 10분 후 만료돼요.</p>
{#if statusError}<p role="alert" class="text-sm text-red-700 dark:text-red-300">{statusError}</p>{/if}
{#if status}
<p class="break-all text-sm">내 기기 지문: {status.identity.nodeId}</p>
<p class="text-sm">동기화: {status.sync.available ? '사용 가능' : '사용 불가'} · 연결 {status.sync.peerCount}개{#if status.sync.lastAttemptUnixSeconds} · 최근 시도 {displayTime(status.sync.lastAttemptUnixSeconds)}{/if}{#if status.sync.lastSuccessUnixSeconds} · 최근 성공 {displayTime(status.sync.lastSuccessUnixSeconds)}{/if}</p>
{#if status.sync.lastError}<p class="break-words text-sm">동기화에 실패했어요. 두 기기의 연결 상태를 확인해 주세요.</p>{/if}
<ul class="flex flex-col gap-2 text-sm">
{#each status.peers as peer (peer.nodeId)}
<li class="break-all">기기 지문: {peer.nodeId} · {peer.origin} <a href={`${peer.origin}/`} target="_blank" rel="noopener noreferrer" class="underline">작업 열기</a> <button type="button" disabled={!!pending} onclick={() => remove(peer.nodeId)} class="min-h-10 rounded-xl border border-neutral-300 px-3 disabled:opacity-40 dark:border-neutral-700">{pending === peer.nodeId ? '해제 중…' : '연결 해제'}</button></li>
{/each}
</ul>
{/if}
<button type="button" disabled={!!pending} onclick={generate} class="min-h-10 self-start rounded-xl border border-neutral-300 px-4 text-sm disabled:opacity-40 dark:border-neutral-700">{pending === 'invite' ? '생성 중…' : '초대 생성'}</button>
{#if invitation}
<label class="text-sm" for="connection-invite">일회용 초대</label>
<textarea id="connection-invite" readonly value={invitation} rows="3" class="w-full rounded-xl border border-neutral-300 bg-transparent p-3 text-sm dark:border-neutral-700"></textarea>
<button type="button" onclick={copy} class="min-h-10 self-start rounded-xl border border-neutral-300 px-4 text-sm dark:border-neutral-700">초대 복사</button>
{/if}
<label class="text-sm" for="connection-paste">상대 기기의 초대</label>
<textarea id="connection-paste" bind:value={pasted} maxlength="16384" rows="3" spellcheck="false" autocomplete="off" class="w-full rounded-xl border border-neutral-300 bg-transparent p-3 text-sm dark:border-neutral-700"></textarea>
{#if preview}<p class="break-all text-sm">초대의 웹 주소: {preview.origin} · 기기 지문: {preview.nodeId}</p><p class="text-sm">연결 전에 상대 기기에서 이 정보가 맞는지 확인하세요.</p>{:else if pasted.trim()}<p class="text-sm">초대가 잘못됐거나 만료됐어요.</p>{/if}
<button type="button" disabled={!!pending} onclick={join} class="min-h-10 self-start rounded-xl border border-neutral-300 px-4 text-sm disabled:opacity-40 dark:border-neutral-700">{pending === 'join' ? '연결 중…' : '초대로 연결'}</button>
{#if error}<p role="alert" class="text-sm text-red-700 dark:text-red-300">{error}</p>{/if}
{#if notice}<p role="status" class="text-sm">{notice}</p>{/if}
<h3 class="font-semibold">Tailscale 기기 검색</h3>
<p class="text-sm">검색 결과는 Tailscale에서 접근 가능한 주소이며 Postudio 인증 여부는 확인하지 않아요. 설정 페이지를 열어 직접 확인하세요.</p>
<button type="button" disabled={!!pending} onclick={discover} class="min-h-10 self-start rounded-xl border border-neutral-300 px-4 text-sm disabled:opacity-40 dark:border-neutral-700">{pending === 'discover' ? '검색 중…' : '기기 검색'}</button>
{#if discoveryError}<p role="alert" class="text-sm">{discoveryError}</p>{/if}
{#if candidates}
{#if candidates.length === 0}<p class="text-sm">검색된 기기가 없어요.</p>{/if}
<ul class="flex flex-col gap-2 text-sm">
{#each candidates as candidate}
{@const origin = safeOrigin(candidate.url)}
<li class="break-all">{candidate.online ? 'Tailscale 접근 가능' : 'Tailscale 오프라인'} · {candidate.url}
{#if origin}<a href={`${origin}/settings`} target="_blank" rel="noopener noreferrer" class="underline">설정 열기</a>{:else}<span> · 유효하지 않은 주소예요.</span>{/if}</li>
{/each}
</ul>
{/if}
</section>
