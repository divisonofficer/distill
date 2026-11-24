const tools = new Set(['create_experiment', 'list_experiments', 'request_run', 'get_run', 'get_request', 'metrics', 'link_references']);
const idPattern = /^[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}$/;

/** @param {string} toolName @param {{ type: string; text?: string }[]} content */
export function researchMarker(toolName, content) {
  if (typeof toolName !== 'string' || !/^(?:mcp__)?postudio-research__(?:create_experiment|list_experiments|request_run|get_run|get_request|metrics|link_references)$/.test(toolName)) return null;
  const name = toolName.slice(toolName.lastIndexOf('__') + 2);
  if (!tools.has(name) || !Array.isArray(content)) return null;
  for (const item of content) {
    if (!item || item.type !== 'text' || typeof item.text !== 'string' || item.text.length > 8192) continue;
    let parsed;
    try { parsed = JSON.parse(item.text); } catch { continue; }
    const marker = parsed && typeof parsed === 'object' && !Array.isArray(parsed) ? parsed.postudioResearch : null;
    if (!marker || typeof marker !== 'object' || Array.isArray(marker)) continue;
    const keys = Object.keys(marker);
    if (keys.some(key => !['projectId', 'experimentId', 'runId', 'approvalId', 'title', 'kind'].includes(key))) continue;
    if (!idPattern.test(marker.projectId ?? '') || !idPattern.test(marker.experimentId ?? '')) continue;
    if (marker.runId !== undefined && !idPattern.test(marker.runId)) continue;
    if (marker.approvalId !== undefined && !idPattern.test(marker.approvalId)) continue;
    if (marker.title !== undefined && (typeof marker.title !== 'string' || marker.title.length > 200)) continue;
    if (marker.kind !== undefined && (typeof marker.kind !== 'string' || marker.kind.length > 40)) continue;
    return { projectId: marker.projectId, experimentId: marker.experimentId, ...(marker.runId ? { runId: marker.runId } : {}), ...(marker.approvalId ? { approvalId: marker.approvalId } : {}), ...(marker.title ? { title: marker.title } : {}), ...(marker.kind ? { kind: marker.kind } : {}) };
  }
  return null;
}

/** Resolve only the existing MCP dispatcher; never infer a tool from output text.
 * @param {{name?:string,input?:unknown}} item
 */
export function researchToolName(item) {
  if (item.name === 'use_tool' && item.input && typeof item.input === 'object' && !Array.isArray(item.input) && 'tool_name' in item.input && typeof item.input.tool_name === 'string') return item.input.tool_name;
  return item.name ?? '';
}
