import type { CommandExecution } from './executions.svelte';

export function commandStatus(item: CommandExecution, fresh: boolean): { label: string; tone: string } {
 if (!item.completed) return fresh ? { label: '실행 중', tone: 'running' } : { label: '알 수 없음', tone: 'unknown' };
 const signal = item.signal?.trim().toLowerCase();
 if (signal && ['sigterm', 'sigkill', 'sigint', 'signal 15', 'signal 9', 'signal 2', 'killed', 'cancelled', 'timeout', 'max_runtime', 'output_limit'].includes(signal)) return { label: '중지됨', tone: 'unknown' };
 if (item.exitCode !== null && item.exitCode !== -1 && item.exitCode !== 0) return { label: '실패', tone: 'failed' };
 if (!signal && item.exitCode === 0) return { label: '완료', tone: 'completed' };
 return { label: '알 수 없음', tone: 'unknown' };
}
