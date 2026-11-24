import test from 'node:test';
import assert from 'node:assert/strict';
import { researchMarker, researchToolName } from './research-result-marker.js';

const result = (value) => [{ type: 'text', text: JSON.stringify(value) }];
const payload = { postudioResearch: { projectId: 'proj-1', experimentId: 'exp-2', approvalId: 'approval:3', title: 'Test', kind: 'approval' } };

test('accepts strict JSON marker only from known namespaced research tools', () => {
  assert.deepEqual(researchMarker('mcp__postudio-research__request_run', result(payload)), payload.postudioResearch);
  assert.equal(researchMarker('mcp__other__request_run', result(payload)), null);
  assert.equal(researchMarker('mcp__postudio-research__request_run', [{ type: 'text', text: `output ${JSON.stringify(payload)}` }]), null);
});

test('rejects malformed, oversized, unknown-key, and invalid-id markers', () => {
  assert.equal(researchMarker('mcp__postudio-research__get_run', [{ type: 'text', text: '{' }]), null);
  assert.equal(researchMarker('mcp__postudio-research__get_run', [{ type: 'text', text: 'x'.repeat(8193) }]), null);
  assert.equal(researchMarker('mcp__postudio-research__get_run', result({ postudioResearch: { ...payload.postudioResearch, extra: true } })), null);
  assert.equal(researchMarker('mcp__postudio-research__get_run', result({ postudioResearch: { ...payload.postudioResearch, projectId: '../bad' } })), null);
});

test('recognizes the real MCP dispatcher without accepting unrelated dispatchers',()=>{
 assert.deepEqual(researchMarker(researchToolName({name:'use_tool',input:{tool_name:'postudio-research__request_run'}}),result(payload)),payload.postudioResearch);
 assert.equal(researchMarker('evil__postudio-research__request_run',result(payload)),null);
 assert.equal(researchMarker(researchToolName({name:'other',input:{tool_name:'postudio-research__request_run'}}),result(payload)),null);
});
