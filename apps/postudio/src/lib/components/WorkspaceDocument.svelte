<script lang="ts">
import { untrack } from 'svelte';
import { getJson } from '$lib/api';
import { artifactEntries, artifactUrl, configuredUntrustedOrigin, type ArtifactEntry } from '$lib/artifact-browser';
import type { WorkbenchTab } from '$lib/workbench-layout';
import type { ToolItem } from '$lib/session.svelte';
import type { NodeInfo } from '$lib/types';

let { tab, host, blocked = false, tools = [], onOpenFile, onOpenPreview, onOpenTool }: {
 tab: WorkbenchTab; host: string; blocked?: boolean; tools?: ToolItem[];
 onOpenFile?: (entry: ArtifactEntry, ownerSessionId: string) => void;
 onOpenPreview?: (port: number) => void;
 onOpenTool?: (tool: ToolItem, view: 'details' | 'diffs' | 'logs') => void;
} = $props();
let folder = $state('');
let entries = $state<ArtifactEntry[]>([]);
let ports = $state<number[]>([]);
let node = $state<NodeInfo | null>(null);
let loading = $state(false);
let error = $state('');
let refresh = $state(0);
let generation = 0;
let identity = '';
let readyIdentity = $state('');
const selectedTool = $derived(tab.kind === 'tool' ? tools.find((tool) => tool.id === tab.toolId) : undefined);
const origin = $derived(node ? configuredUntrustedOrigin(node, host) : null);
const fileUrl = $derived(tab.kind === 'file' && origin ? artifactUrl(origin, tab.sessionId, tab.path) : null);
const allowedPort = $derived(tab.kind === 'preview' && tab.port !== undefined && ports.includes(tab.port) ? tab.port : null);
const previewUrl = $derived(origin && allowedPort !== null ? `${origin}/p/${allowedPort}/` : null);
const identityKey = $derived(`${tab.kind}\u0000${tab.sessionId}\u0000${host}\u0000${tab.kind === 'file' ? tab.path : tab.kind === 'tool' ? tab.toolId + ':' + tab.view : tab.kind === 'preview' ? tab.port ?? '' : ''}`);
const commandTools = $derived(tools.filter((tool) => tool.toolKind === 'execute' || /command|execute|shell|terminal|bash/i.test(tool.name ?? '')));
const toolChoices = $derived(tab.kind === 'tool' && tab.view === 'logs' ? commandTools : tools);

$effect(() => {
 const sid = tab.sessionId, destination = host, kind = tab.kind, key = identityKey;
 void refresh;
 if (key !== identity) {
  untrack(() => { identity = key; entries = []; ports = []; node = null; error = ''; readyIdentity = ''; if (kind === 'explorer') folder = ''; });
 }
 const dir = folder;
 const token = ++generation;
 loading = false; error = '';
 if (blocked) return () => { if (token === generation) generation++; };
 const needsInfo = kind === 'file' || kind === 'preview';
 const needsEntries = kind === 'explorer';
 const needsPorts = kind === 'preview';
 if (!needsInfo && !needsEntries) return () => { if (token === generation) generation++; };
 loading = true;
 void (async () => {
  try {
   const infoPromise = (needsInfo || needsEntries ? getJson<NodeInfo>('/api/node', destination) : Promise.resolve(null)).catch(() => null);
   const entriesPromise = needsEntries ? artifactEntries(sid, dir, destination) : Promise.resolve([] as ArtifactEntry[]);
   const portsPromise = needsPorts ? getJson<{ ports: Array<{ port: number }> }>('/api/ports', destination) : Promise.resolve({ ports: [] });
   const [info, listed, available] = await Promise.all([infoPromise, entriesPromise, portsPromise]);
   if (token !== generation) return;
   node = info;
   entries = listed;
   ports = [...new Set(available.ports.map((item) => item.port).filter((value) => Number.isInteger(value) && value >= 1 && value <= 65535))];
   readyIdentity = key;
  } catch (cause) {
   if (token === generation) { readyIdentity = key; error = cause instanceof Error ? cause.message : '워크스페이스를 불러오지 못했습니다.'; }
  } finally { if (token === generation) loading = false; }
 })();
 return () => { if (token === generation) generation++; };
});

function openFolder(path: string) { folder = path; refresh++; }
function retry() { refresh++; }
function openTool(tool: ToolItem) { if (tab.kind === 'tool') onOpenTool?.(tool, tab.view); }
</script>

<div class="document">
 {#if blocked}<p class="message" role="alert">호스트를 사용할 수 없어 작업 공간 요청을 차단했습니다.</p>
 {:else if tab.kind === 'conversation'}
  <p class="message">대화 탭은 여기서 표시하지 않습니다.</p>
 {:else if tab.kind === 'explorer'}
  <section class="pane" aria-label="파일 탐색기">
   <div class="toolbar"><button type="button" aria-label="상위 폴더" onclick={() => openFolder(folder.split('/').slice(0, -1).join('/'))} disabled={!folder}>상위 폴더</button><button type="button" onclick={() => openFolder('')}>루트</button><span class="crumb" title={folder}>/{folder}</span><button type="button" onclick={retry}>새로고침</button></div>
   {#if error}<p class="message" role="alert">{error} <button type="button" onclick={retry}>다시 시도</button></p>
   {:else if loading}<p class="message" role="status">파일 목록을 불러오는 중…</p>
   {:else}<nav class="files" aria-label="파일 목록">{#each entries as entry (entry.path)}<button class="entry" type="button" onclick={() => entry.kind === 'directory' ? openFolder(entry.path) : onOpenFile?.(entry, tab.sessionId)}><span>{entry.kind === 'directory' ? '▸' : '·'}</span><span class="filename">{entry.name}</span></button>{:else}<p class="message">이 폴더에 파일이 없습니다.</p>{/each}</nav>{/if}
  </section>
 {:else if tab.kind === 'file'}
  <section class="pane" aria-label={tab.title}>
   <div class="toolbar"><strong>{tab.title}</strong><span class="readonly">읽기 전용</span><button type="button" onclick={retry}>새로고침</button></div>
   {#if error}<p class="message" role="alert">{error} <button type="button" onclick={retry}>다시 시도</button></p>
   {:else if loading}<p class="message" role="status">파일을 불러오는 중…</p>
   {:else if readyIdentity !== identityKey}<p class="message" role="status">파일 출처를 확인하는 중…</p>
   {:else if !origin}<p class="message">안전한 파일 출처를 사용할 수 없습니다.</p>
   {:else if /^(image\/(png|jpeg|gif|webp|avif))$/.test(tab.mime ?? '')}<img class="image" src={fileUrl!} alt={tab.title} />
   {:else}<iframe class="frame" title={tab.title} src={fileUrl!} sandbox={tab.mime === 'text/html' ? 'allow-scripts' : ''}></iframe>{/if}
  </section>
 {:else if tab.kind === 'preview'}
  <section class="pane" aria-label="미리보기">
   <div class="toolbar"><span>{tab.port === undefined ? '사용 가능한 포트' : `포트 ${tab.port}`}</span><button type="button" onclick={retry}>새로고침</button></div>
   {#if error}<p class="message" role="alert">{error} <button type="button" onclick={retry}>다시 시도</button></p>
   {:else if loading}<p class="message" role="status">포트 목록을 불러오는 중…</p>
   {:else if tab.port === undefined}{#each ports as port (port)}<button class="entry" type="button" onclick={() => onOpenPreview?.(port)}>포트 열기 {port}</button>{:else}<p class="message">사용 가능한 미리보기 포트가 없습니다.</p>{/each}
   {:else if readyIdentity !== identityKey}<p class="message" role="status">포트 출처를 확인하는 중…</p>
   {:else if !ports.includes(tab.port)}<p class="message">포트 {tab.port}가 허용된 포트 목록에 없습니다.</p>
   {:else if !previewUrl}<p class="message">안전한 미리보기 출처를 사용할 수 없습니다.</p>
   {:else}<iframe class="frame" title={`미리보기 포트 ${tab.port}`} src={previewUrl} sandbox="allow-scripts"></iframe>{/if}
  </section>
 {:else if tab.kind === 'tool'}
  <section class="pane" aria-label="도구">
   {#if !tab.toolId}<div class="toolbar"><strong>도구</strong></div>{#each toolChoices as tool (tool.id)}<button class="entry" type="button" onclick={() => openTool(tool)}><span class="filename">{tool.title ?? tool.name ?? tool.id}</span></button>{:else}<p class="message">사용 가능한 도구가 없습니다.</p>{/each}
   {:else if !selectedTool}<p class="message">도구 데이터가 아직 로드되지 않았습니다.</p>
   {:else}<div class="toolbar"><strong>{selectedTool.title ?? selectedTool.name ?? selectedTool.id}</strong><span class="readonly">읽기 전용</span></div>
    {#if tab.view === 'logs'}<p class="notice">명령 실행 기록 · 대화형 셸이 아닙니다.</p>{#each (selectedTool.content ?? []).filter((item) => item.type === 'text') as item, index (index)}<pre>{item.text}</pre>{:else}<p class="message">명령 출력이 없습니다.</p>{/each}
    {:else if tab.view === 'diffs'}{#each (selectedTool.content ?? []).filter((item) => item.type === 'diff') as diff (diff.path)}<article><h2>{diff.path}</h2>{#if diff.old !== undefined}<h3>변경 전</h3><pre>{diff.old}</pre>{/if}<h3>변경 후</h3><pre>{diff.new}</pre></article>{:else}<p class="message">변경 사항이 없습니다.</p>{/each}
    {:else}<p class="notice">{selectedTool.status ?? '상태 정보 없음'}</p>{#if selectedTool.input !== undefined}<h3>입력</h3><pre>{JSON.stringify(selectedTool.input, null, 2)}</pre>{/if}{#each (selectedTool.content ?? []).filter((item) => item.type === 'text') as item, index (index)}<h3>출력</h3><pre>{item.text}</pre>{/each}{/if}
   {/if}
  </section>
 {/if}
</div>

<style>
.document { display:flex; flex:1; flex-direction:column; width:100%; min-width:0; min-height:0; overflow:hidden; font:inherit; }
.pane { display:flex; flex:1; flex-direction:column; min-width:0; min-height:0; overflow:auto; }
.toolbar { display:flex; flex-wrap:wrap; align-items:center; gap:8px; min-height:40px; padding:4px 8px; border-bottom:1px solid #ececef; }
.toolbar button, .entry, .message button { min-height:32px; padding:5px 8px; border:1px solid #e6e6e8; border-radius:6px; background:inherit; color:inherit; font:inherit; }
.crumb { flex:1; min-width:0; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.files { min-width:0; overflow:auto; }
.entry { display:flex; width:100%; gap:8px; align-items:center; border-color:transparent; text-align:left; overflow:hidden; }
.entry:hover { background:#e8e8eb; }
.filename { min-width:0; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.readonly { margin-left:auto; color:#66666b; font-size:11px; }
.message, .notice { padding:12px; overflow-wrap:anywhere; color:#66666b; }
.image { max-width:100%; max-height:100%; object-fit:contain; }
.frame { width:100%; flex:1; min-height:200px; border:0; }
pre { margin:0; padding:12px; white-space:pre-wrap; overflow-wrap:anywhere; background:#1b1b1f; color:#e6e6ea; font:12px/1.7 ui-monospace,monospace; }
article { padding:12px; }
h2,h3 { margin:10px; overflow-wrap:anywhere; }
button:focus-visible { outline:2px solid #2b59c3; outline-offset:-2px; }
</style>
