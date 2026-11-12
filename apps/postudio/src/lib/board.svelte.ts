import { connect, getJson, type Socket } from './api';
import { orderedOrigins } from './fleet';
import type { BoardRow } from './types';

type BoardMsg =
	| { t: 'board.upsert'; sessions: BoardRow[] }
	| { t: 'board.remove'; ids: string[] };

const RANK: Record<string, number> = { needs_input: 0, working: 1 };

/** Sessions sorted: needs_input, then working, then the rest; newest change first. */
export function sortRows(rows: BoardRow[]): BoardRow[] {
	const rank = (r: BoardRow) => RANK[r.activity] ?? 2;
	return [...rows].sort((a, b) => rank(a) - rank(b) || b.lastChangeUnixMs - a.lastChangeUnixMs);
}

class Board {
	rows = $state<BoardRow[]>([]);
	host = $state('');
	get origin() { return this.host; }
	connected = $state(false);
	#sock: Socket | null = null;

	start() {
		if (this.#sock) return;
		const hosts = orderedOrigins();
		this.host = hosts[0] ?? (typeof location === 'undefined' ? '' : location.origin);
		this.#sock = connect<BoardMsg>('/ws/board', {
			onHost: (origin) => {
				if (this.host !== origin) this.rows = [];
				this.host = origin;
			},
			onOpen: () => {
				this.connected = true;
				void this.#refresh();
			},
			onClose: () => (this.connected = false),
			onMessage: (m) => this.#apply(m)
		}, { failover: true });
	}

	find(id: string): BoardRow | undefined {
		return this.rows.find((r) => r.sessionId === id);
	}

	async #refresh() {
		try {
			const origin = this.host;
			const res = await getJson<{ sessions: BoardRow[] }>('/api/sessions', origin || undefined);
			if (this.host === origin) this.rows = res.sessions.map((row) => ({ ...row, apiOrigin: origin }));
		} catch {
			// the board stays on the last known rows
		}
	}

	#apply(m: BoardMsg) {
		if (m.t === 'board.upsert') {
			const map = new Map(this.rows.map((r) => [r.sessionId, r]));
			for (const s of m.sessions) map.set(s.sessionId, { ...s, apiOrigin: this.origin });
			this.rows = [...map.values()];
		} else if (m.t === 'board.remove') {
			const gone = new Set(m.ids);
			this.rows = this.rows.filter((r) => !gone.has(r.sessionId));
		}
	}
}

export const board = new Board();
