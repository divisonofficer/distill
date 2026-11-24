import type { ResearchExperiment, ResearchRun, ResearchTarget } from './research';

export function buildExperimentTree(experiments: ResearchExperiment[], runs:ResearchRun[]=[], metricValues:Record<string,Record<string,number>>={}, metric:string|null=null, direction:'higher'|'lower'|undefined=undefined) {
 const runByExperiment=new Map<string,ResearchRun[]>();for(const run of runs){const list=runByExperiment.get(run.experimentId)||[];list.push(run);runByExperiment.set(run.experimentId,list);}
 const stats=new Map<string,{state:string;latest:ResearchRun|null;value:number|null;best:boolean}>();let bestId:string|null=null;let bestValue:number|null=null;
 for(const experiment of experiments){const actual=runByExperiment.get(experiment.id)||[];const newest=[...actual].sort((a,b)=>b.createdAt-a.createdAt)[0];const state=actual.some(r=>['queued','submitting','starting','running'].includes(r.status))?'running':!newest?'draft':newest.status==='failed'?'failed':newest.status==='cancelled'?'cancelled':newest.status==='lost'?'lost':latestDoneRun(actual)?'done':'draft' ;const latest=latestDoneRun(actual);const raw=latest&&metric?metricValues[latest.id]?.[metric]:undefined;const value=typeof raw==='number'&&Number.isFinite(raw)?raw:null;stats.set(experiment.id,{state,latest,value,best:false});if(direction&&value!==null&&(bestValue===null||(direction==='higher'?value>bestValue:value<bestValue))){bestValue=value;bestId=experiment.id;}}
 if(bestId)stats.get(bestId)!.best=true;
 const byId = new Map(experiments.map((experiment) => [experiment.id, experiment]));
 const children = new Map<string, ResearchExperiment[]>();
 const roots: ResearchExperiment[] = [];
 for (const experiment of experiments) {
  const parent = experiment.parentExperimentId;
  if (parent && parent !== experiment.id && byId.has(parent)) {
   const siblings = children.get(parent) ?? [];
   siblings.push(experiment);
   children.set(parent, siblings);
  } else roots.push(experiment);
 }
 const result: Array<{ experiment: ResearchExperiment; depth: number; phase: string; value: number|null; delta: 'better'|'worse'|'neutral'; best: boolean }> = [];
 const visited = new Set<string>();
 const visit = (experiment: ResearchExperiment, depth: number) => {
  if (visited.has(experiment.id)) return;
  visited.add(experiment.id);
  const summary=stats.get(experiment.id)!; const parent=experiment.parentExperimentId?stats.get(experiment.parentExperimentId):undefined; result.push({ experiment, depth, phase:summary.state, value:summary.value, delta:metricDelta(summary.value,parent?.value??null,direction), best:summary.best });
  for (const child of children.get(experiment.id) ?? []) visit(child, depth + 1);
 };
 for (const root of roots) visit(root, 0);
 for (const experiment of experiments) visit(experiment, 0);
 return result;
}

export function stableRunSequence(runs: ResearchRun[]): Map<string, number> {
 const sorted = [...runs].sort((a, b) => a.createdAt - b.createdAt || a.id.localeCompare(b.id));
 const counts=new Map<string,number>();const result=new Map<string,number>();
 for(const run of sorted){const group=run.experimentId||'';const next=(counts.get(group)||0)+1;counts.set(group,next);result.set(run.id,next);}return result;
}

export function finiteMetric(run: ResearchRun, metric: string): number | null {
 const metrics = run.metrics;
 if (!metrics || typeof metrics !== 'object') return null;
 const value = (metrics as Record<string, unknown>)[metric];
 return typeof value === 'number' && Number.isFinite(value) ? value : null;
}

export function latestDoneRun(runs: ResearchRun[]): ResearchRun | null {
 return runs.filter((run) => ['done', 'best'].includes(run.status) && Number.isFinite(run.endedAt ?? run.updatedAt))
  .sort((a, b) => (b.endedAt ?? b.updatedAt) - (a.endedAt ?? a.updatedAt) || b.id.localeCompare(a.id))[0] ?? null;
}

export function bestMetricRun(runs: ResearchRun[], metric: string, direction: 'higher' | 'lower' | undefined): ResearchRun | null {
 if (!direction) return null;
 return runs.filter((run) => ['done', 'best'].includes(run.status) && finiteMetric(run, metric) !== null)
  .sort((a, b) => {
   const av = finiteMetric(a, metric)!; const bv = finiteMetric(b, metric)!;
   return (direction === 'higher' ? bv - av : av - bv) || (b.endedAt ?? b.updatedAt) - (a.endedAt ?? a.updatedAt) || a.id.localeCompare(b.id);
  })[0] ?? null;
}

export function phaseLabel(phase: string): string {
 const labels: Record<string, string> = { draft:'실행 전', queued: '대기', submitting: '대기', starting: '대기', running: '실행 중', done: '완료', best: '완료', failed: '실패', cancelled: '취소됨', lost: '확인 필요' };
 return labels[phase] ?? `상태 확인 필요 (${phase || '알 수 없음'})`;
}

export function safeResearchUrl(value: string | undefined): string | null {
 if (!value) return null;
 try {
  const url = new URL(value);
  if (url.protocol !== 'https:' || url.username || url.password || !url.hostname.includes('.') || /^(localhost|.*\.(local|internal))$/i.test(url.hostname) || /^\d{1,3}(\.\d{1,3}){3}$/.test(url.hostname)) return null;
  return url.href;
 } catch { return null; }
}

export function metricDelta(value: number | null, baseline: number | null, direction: 'higher' | 'lower' | undefined): 'better' | 'worse' | 'neutral' {
 if (value === null || baseline === null || !direction || value === baseline) return 'neutral';
 const improved = direction === 'higher' ? value > baseline : value < baseline;
 return improved ? 'better' : 'worse';
}

export function availableTarget(targets: ResearchTarget[], selectedId: string): ResearchTarget | null {
 return targets.find((target) => target.id === selectedId && target.available) ?? null;
}
