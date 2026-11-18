import type { Mode } from './types';

const RECENT_KEY = 'postudio.recentCwds';
const MODE_KEY = 'postudio.defaultMode';

export const MODES: Mode[] = ['ask', 'auto', 'always-approve'];
export const MODE_LABEL: Record<Mode, string> = {
	ask: '수동',
	auto: '자동',
	'always-approve': '항상 승인'
};

export function loadRecent(): string[] {
	try {
		const v: unknown = JSON.parse(localStorage.getItem(RECENT_KEY) ?? '[]');
		return Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string').slice(0, 5) : [];
	} catch {
		return [];
	}
}

export function rememberCwd(cwd: string) {
	const list = [cwd, ...loadRecent().filter((c) => c !== cwd)].slice(0, 5);
	localStorage.setItem(RECENT_KEY, JSON.stringify(list));
}

export function loadDefaultMode(): Mode {
	const v = localStorage.getItem(MODE_KEY);
	return v === 'auto' || v === 'always-approve' ? v : 'ask';
}

export function saveDefaultMode(mode: Mode) {
	localStorage.setItem(MODE_KEY, mode);
}
