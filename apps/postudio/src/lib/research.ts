import { getJson, postJson, putJson, deleteJson } from './api';

export type ResearchSource = 'alphaXiv' | 'openAlex' | 'bioRxiv' | 'pubMed';
export interface ResearchCapabilities { approvedSshTarget?:boolean; postudioContract: number; expectedCommit: boolean; workspace: boolean; metrics: boolean; sources: string[]; dashboard: boolean; researchTools: boolean; }
export interface ResearchStatus { available: boolean; connected: boolean; version: string | null; error?: string; capabilities?: ResearchCapabilities; library?: { connected: boolean; reason?: string }; }
export interface ResearchProject { id: string; name: string; slug: string; repoPath: string; runCommand?: string | null; paperId?: string | null; createdAt: number; updatedAt: number; [key: string]: unknown; }
export interface ResearchExperiment { id: string; projectId: string; parentExperimentId: string | null; slug: string; branchName: string; title: string | null; description: string | null; runCommand: string; agentStatus: string; createdAt: number; updatedAt: number; archived: boolean; [key: string]: unknown; }
export interface ResearchRun { id: string; projectId: string; experimentId: string; status: string; command: string; createdAt: number; updatedAt: number; endedAt: number | null; exitCode: number | null; commitSha: string | null; contact?: { type: string; [key: string]: unknown } | null; cancelling?: boolean; cancelError?: string | null; startedAt?: number | null; [key: string]: unknown; }
export interface ResearchPaper { id: string; source: ResearchSource; sourceId: string; title: string; authors?: string[]; year?: number | null; venue?: string | null; url?: string; abstract?: string; }
export interface ResearchFile { name: string; path: string; size?: number; kind?: 'directory' | 'file' | string; mime?: string; }
export interface ResearchLog { text: string; start: number; end: number; ident?: unknown; total?: number; reset?: boolean; }
export interface ResearchSnapshot { headCommit: string; uncommittedCount: number | null; worktreePath?: string | null; branch: string; }
export interface ResearchWorkspace extends ResearchSnapshot { cwd: string; }
export interface ResearchTarget { id: string; label: string; backend: 'local' | 'ssh'; available: boolean; reason?: string; }
export interface ResearchTargetBinding { id: string; postudioHostId: string; orxTargetAlias: string; backend: 'local' | 'ssh'; }
export interface ResearchTargets {targets:ResearchTarget[];bindings?:ResearchTargetBinding[];candidates?:Array<{alias:string;label:string}>;sourceError?:string;}
export interface ResearchMetrics { runId: string; metrics: Record<string, number>; series: Record<string, Array<{ step: number; value: number }>>; source: 'trackio'; sourceUrl?: string | null; errors: string[]; }
export interface ResearchPaperRef { path: string; anchor?: string | null; }
export interface ResearchExperimentRefs { distillSessionId?: string; sessionIds?: string[]; literatureIds: string[]; paperRef?: ResearchPaperRef | null; artifactPaths: string[]; }
export interface ResearchApproval { id: string; projectId: string; experimentId: string; sessionId?: string; expectedCommit: string; targetId: string; status: 'pending' | 'approved' | 'rejected' | 'submitted' | 'failed'; runId?: string; error?: string; }
export interface ProjectResearchRefs { literature: ResearchPaper[]; experiments: Record<string, ResearchExperimentRefs>; settings: { primaryMetric: string | null; metricDirections: Record<string, 'higher' | 'lower'>; metricUnits: Record<string, string> }; approvals?: ResearchApproval[]; }
export interface ResearchLinks { literatureIds?: string[]; paperRef?: ResearchPaperRef | null; artifactPaths?: string[]; }
export interface ResearchPaperSearch { papers: ResearchPaper[]; sourceErrors?: Record<string, string>; }

export class ResearchMutationGate {
 #active = new Set<string>();
 enter(key: string): (() => void) | null {
  if (this.#active.has(key)) return null;
  this.#active.add(key);
  let active = true;
  return () => { if (active) { active = false; this.#active.delete(key); } };
 }
}

const api = <T>(host?: string) => ({ get: (path: string) => getJson<T>(path, host, 20_000), post: <R>(path: string, body: unknown) => postJson<R>(path, body, host), put: <R>(path: string, body: unknown) => putJson<R>(path, body, host), delete: <R>(path: string) => deleteJson<R>(path, host) });
const enc = (value: string) => encodeURIComponent(value);
const projectPath = (id: string) => `/api/research/projects/${enc(id)}`;

export function researchApi(host?: string) {
 const client = api<unknown>(host);
 return {
  status: () => client.get('/api/research/status') as Promise<ResearchStatus>,
  connect: () => client.post<ResearchStatus>('/api/research/connect', {}),
  projects: async () => (await client.get('/api/research/projects') as { projects: ResearchProject[] }).projects,
  registerProject: async (name: string, path: string) => (await client.post<{ project: ResearchProject }>('/api/research/projects', { name, path })).project,
  project: async (id: string) => (await client.get(`${projectPath(id)}`) as { project: ResearchProject }).project,
  experiments: async (id: string) => (await client.get(`${projectPath(id)}/experiments`) as { experiments: ResearchExperiment[] }).experiments,
  createExperiment: async (id: string, input: { title: string; description?: string; runCommand?: string; parentId?: string }) => (await client.post<{ experiments: ResearchExperiment[] }>(`${projectPath(id)}/experiments`, input)).experiments,
  runs: async (id: string) => (await client.get(`${projectPath(id)}/runs`) as { runs: ResearchRun[] }).runs,
  startRun: async (id: string, experimentId: string, expectedCommit?: string, targetId?: string) => (await client.post<{ run: ResearchRun }>(`${projectPath(id)}/runs`, { experimentId, approved: true, ...(expectedCommit ? { expectedCommit } : {}), ...(targetId ? { targetId } : {}) })).run,
  cancelRun: async (id: string, runId: string) => (await client.post<{ run: ResearchRun }>(`${projectPath(id)}/runs/${enc(runId)}/cancel`, { approved: true })).run,
  log: (id: string, runId: string, start = 0, ident?: unknown) => {
   const params = new URLSearchParams({ start: String(start), end: String(start + 65536) });
   if (ident !== undefined) params.set('ident', typeof ident === 'string' ? ident : JSON.stringify(ident));
   return client.get(`${projectPath(id)}/runs/${enc(runId)}/log?${params}`) as Promise<ResearchLog>;
  },
  papers: async (query: string) => (await client.get(`/api/research/papers?q=${encodeURIComponent(query)}`) as { papers: ResearchPaper[] }).papers,
  searchPapers: (query: string, sources?: ResearchSource[]) => client.get(`/api/research/papers?q=${encodeURIComponent(query)}${sources?.length ? `&sources=${encodeURIComponent(sources.join(','))}` : ''}`) as Promise<ResearchPaperSearch>,
  files: async (id: string) => (await client.get(`${projectPath(id)}/files`) as { files: ResearchFile[] }).files,
  refs: async (id: string) => await client.get(`${projectPath(id)}/refs`) as ProjectResearchRefs,
  settings: async (id: string, settings: ProjectResearchRefs['settings']) => await client.put<ProjectResearchRefs>(`${projectPath(id)}/settings`, settings),
  snapshot: (p: string, e: string) => client.get(`${projectPath(p)}/experiments/${enc(e)}/snapshot`) as Promise<ResearchSnapshot>,
  workspace: (p: string, e: string) => client.post<ResearchWorkspace>(`${projectPath(p)}/experiments/${enc(e)}/workspace`, {}),
  targets: (p: string) => client.get(`${projectPath(p)}/targets`) as Promise<ResearchTargets>,
  saveTargets: (p: string, bindings: ResearchTargetBinding[]) => client.put<ResearchTargets>(`${projectPath(p)}/targets`, { bindings }),
  metrics: (p: string, r: string) => client.get(`${projectPath(p)}/runs/${enc(r)}/metrics`) as Promise<ResearchMetrics>,
  reconnect: (p: string, r: string) => client.post<{ run: ResearchRun }>(`${projectPath(p)}/runs/${enc(r)}/reconnect`, {}),
  linkPaper: (p: string, paper: ResearchPaper, experimentId?: string) => client.post<ProjectResearchRefs>(`${projectPath(p)}/literature`, { paper, ...(experimentId ? { experimentId } : {}) }),
  unlinkPaper: (p: string, paperId: string) => client.delete<ProjectResearchRefs>(`${projectPath(p)}/literature/${enc(paperId)}`),
  saveLinks: (p: string, e: string, links: ResearchLinks) => client.put<ProjectResearchRefs>(`${projectPath(p)}/experiments/${enc(e)}/links`, links),
  requestApproval:(p:string,e:string,expectedCommit:string,targetId:string)=>client.post<{approval:ResearchApproval}>(`${projectPath(p)}/experiments/${enc(e)}/approvals`,{expectedCommit,targetId}),
  approval: (p: string, a: string) => client.get(`${projectPath(p)}/approvals/${enc(a)}`) as Promise<{ approval: ResearchApproval; run?: ResearchRun | null }>, 
  approve: (p: string, a: string, approved: boolean) => client.post<{ approval: ResearchApproval; run?: ResearchRun | null }>(`${projectPath(p)}/approvals/${enc(a)}`, { approved }),
  startChat: (p: string, e: string, requestId: string) => client.post<{ sessionId: string; host: string; draftContext?: unknown }>(`${projectPath(p)}/experiments/${enc(e)}/chat`, { requestId }),
  dashboard: (p: string, clientOrigin: string) => client.post<{ url: string }>(`${projectPath(p)}/dashboard`, { clientOrigin }),
  artifactUrl: (p: string, path: string, download = false) => `${host || ''}${projectPath(p)}/artifact?path=${encodeURIComponent(path)}${download ? '&download=1' : ''}`
 };
}

export function runLabel(status: string): string {
 switch (status) {
  case 'queued': case 'submitting': case 'starting': return '대기'; case 'running': return '실행 중'; case 'done': return '완료'; case 'best': return '완료'; case 'failed': return '실패'; case 'canceled': case 'cancelled': return '취소됨'; case 'lost': return '확인 필요'; default: return '확인 필요';
 }
}
export function isRunActive(status: string): boolean { return ['queued', 'submitting', 'starting', 'running'].includes(status); }
export function runContactNotice(type: string): string | null {
 switch (type) { case 'live': return null; case 'unreachable': return '실행 호스트에 연결할 수 없어요.'; case 'needs_auth': return '실행 호스트 인증이 필요해요.'; case 'unreconciled': return '실행 결과를 확인하지 못했어요.'; case 'stopping': return '취소 요청을 확인 중이에요.'; default: return `호스트 상태 확인 필요: ${type}`; }
}
