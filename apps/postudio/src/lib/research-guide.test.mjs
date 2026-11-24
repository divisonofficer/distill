import assert from 'node:assert/strict';
import { availableTarget, bestMetricRun, buildExperimentTree, finiteMetric, latestDoneRun, metricDelta, phaseLabel, stableRunSequence } from './research-guide.ts';
const exp = (id, parentExperimentId = null) => ({ id, parentExperimentId });
const tree = buildExperimentTree([exp('child', 'root'), exp('root'), exp('orphan', 'missing'), exp('cycle-a', 'cycle-b'), exp('cycle-b', 'cycle-a')]);
assert.equal(tree.length, 5);
assert.equal(tree.find((item) => item.experiment.id === 'child').depth, 1);
const run = (id, status, endedAt, metrics = {}) => ({ id, status, endedAt, updatedAt: endedAt, createdAt: endedAt, metrics });
const runs = [run('a', 'done', 10, { score: 4 }), run('b', 'best', 20, { score: 7 }), run('bad', 'failed', 99, { score: 100 })];
assert.deepEqual([...stableRunSequence(runs).values()], [1, 2, 3]);
assert.equal(latestDoneRun(runs).id, 'b');
assert.equal(bestMetricRun(runs, 'score', 'higher').id, 'b');
assert.equal(bestMetricRun(runs, 'score', 'lower').id, 'a');
assert.equal(bestMetricRun(runs, 'score', undefined), null);
assert.equal(finiteMetric(run('x', 'done', 1, { score: Infinity }), 'score'), null);
assert.equal(metricDelta(3, 2, 'higher'), 'better');
assert.equal(metricDelta(3, 2, undefined), 'neutral');
assert.match(phaseLabel('unknown-phase'), /unknown-phase/);
console.log('research-guide helper checks passed');

const scoped=[{...run('root-run','done',1),experimentId:'root'},{...run('child-run','done',2),experimentId:'child'},{...run('child-fail','failed',3),experimentId:'child'}];
assert.deepEqual([...stableRunSequence(scoped).values()],[1,1,2]);
const derived=buildExperimentTree([exp('root'),exp('child','root'),exp('draft','child')],scoped,{'root-run':{score:4},'child-run':{score:5}},'score','higher');
assert.equal(derived.find(x=>x.experiment.id==='child').phase,'failed');
assert.equal(derived.find(x=>x.experiment.id==='child').delta,'better');
assert.equal(derived.filter(x=>x.best).length,1);
assert.equal(derived.find(x=>x.experiment.id==='draft').phase,'draft');

const terminalTree=buildExperimentTree([exp('lost'),exp('cancelled')],[{...run('lost-run','lost',3),experimentId:'lost'},{...run('cancelled-run','cancelled',4),experimentId:'cancelled'}]);
assert.equal(terminalTree.find(x=>x.experiment.id==='lost').phase,'lost');
assert.equal(terminalTree.find(x=>x.experiment.id==='cancelled').phase,'cancelled');
assert.equal(phaseLabel('lost'),'확인 필요');
assert.equal(phaseLabel('cancelled'),'취소됨');

const targets=[{id:'offline',label:'Offline',backend:'local',available:false},{id:'ready',label:'Ready',backend:'local',available:true}];
assert.equal(availableTarget(targets,'offline').id,'ready');
assert.equal(availableTarget(targets,'missing').id,'ready');
assert.equal(availableTarget([],''),null);
