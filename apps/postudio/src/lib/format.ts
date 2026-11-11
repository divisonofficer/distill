import type { BoardRow, Status } from './types';

export type Tone = 'run' | 'wait' | 'idle' | 'err' | 'off' | 'sleep';

export function activityBadge(activity: string): { label: string; tone: Tone } {
	switch (activity) {
		case 'needs_input':
			return { label: '승인 대기', tone: 'wait' };
		case 'working':
			return { label: '실행 중', tone: 'run' };
		case 'error':
			return { label: '오류', tone: 'err' };
		case 'dormant':
			return { label: '휴면', tone: 'sleep' };
		default:
			return { label: '유휴', tone: 'idle' };
	}
}

export function statusBadge(status: Status | 'offline'): { label: string; tone: Tone } {
	switch (status) {
		case 'running':
			return { label: '실행 중', tone: 'run' };
		case 'waiting_permission':
			return { label: '승인 대기', tone: 'wait' };
		case 'error':
			return { label: '오류', tone: 'err' };
		case 'offline':
			return { label: '연결 끊김', tone: 'off' };
		default:
			return { label: '유휴', tone: 'idle' };
	}
}

export function basename(path: string): string {
	return path.replace(/\/+$/, '').split('/').pop() || path;
}

export function sessionTitle(row: BoardRow): string {
	return row.title || basename(row.cwd);
}
