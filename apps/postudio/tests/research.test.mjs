import assert from 'node:assert/strict';
import { after, before, test } from 'node:test';
import { createRequire } from 'node:module';
const require = createRequire(new URL('../package.json', import.meta.url));
const { createServer } = require('vite');
const { svelte } = require('@sveltejs/vite-plugin-svelte');
let vite, guide, research;
before(async () => { vite = await createServer({ root: new URL('..', import.meta.url).pathname, configFile: false, plugins: [svelte()], server: { middlewareMode: true }, appType: 'custom' }); guide = await vite.ssrLoadModule('/src/lib/research-guide.ts'); research = await vite.ssrLoadModule('/src/lib/research.ts'); });
after(async () => { await vite?.close(); });
const call = (name, ...args) => guide[name](...args);

const experiment = (id, parentExperimentId = null) => ({ id, parentExperimentId });
const run = (id, status, endedAt, metrics = {}) => ({ id, status, endedAt, updatedAt: endedAt, createdAt: endedAt, metrics });

test('tree handles child-before-parent, orphan, self-parent, and cycle safely', () => {
 const tree = call('buildExperimentTree', [experiment('child', 'root'), experiment('root'), experiment('orphan', 'missing'), experiment('self', 'self'), experiment('a', 'b'), experiment('b', 'a')]);
 assert.equal(tree.length, 6);
 assert.equal(tree.find((item) => item.experiment.id === 'child').depth, 1);
 assert.equal(tree.find((item) => item.experiment.id === 'a').depth, 0);
});

test('state and metric derivations use actual finished values and stable ordering', () => {
 const runs = [run('one', 'done', 10, { score: 2 }), run('two', 'best', 20, { score: 4 }), run('failed', 'failed', 99, { score: 100 })];
 assert.deepEqual([...call('stableRunSequence', runs).values()], [1, 2, 3]);
 assert.equal(call('latestDoneRun', runs).id, 'two');
 assert.equal(call('bestMetricRun', runs, 'score', 'higher').id, 'two');
 assert.equal(call('bestMetricRun', runs, 'score', 'lower').id, 'one');
 assert.equal(call('bestMetricRun', runs, 'score', undefined), null);
 assert.equal(call('finiteMetric', run('nan', 'done', 1, { score: NaN }), 'score'), null);
 assert.equal(call('metricDelta', 4, 2, 'higher'), 'better');
 assert.equal(call('metricDelta', 4, null, 'higher'), 'neutral');
 assert.match(call('phaseLabel', 'new-native-phase'), /new-native-phase/);
 assert.equal(call('safeResearchUrl', 'https://doi.org/10.1000/test'), 'https://doi.org/10.1000/test');
 assert.equal(call('safeResearchUrl', 'http://example.org/paper'), null);
 assert.equal(call('safeResearchUrl', 'https://user:pass@example.org/paper'), null);
 assert.equal(call('safeResearchUrl', 'https://localhost/paper'), null);
});

test('connection notices stay distinct from run phase', () => {
 assert.equal(research.runContactNotice('live'), null);
 assert.match(research.runContactNotice('unreachable'), /연결할 수 없/);
 assert.equal(call('phaseLabel', 'running'), '실행 중');
 assert.equal(call('phaseLabel', 'lost'), '확인 필요');
});
