import { connect, getJson, type Socket } from './api';
import type { BoardRow, NodeInfo } from './types';

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
	connected = $state(false);
	#sock: Socket | null = null;

	start() {
		if (this.#sock) return;
		getJson<NodeInfo>('/api/node')
			.then((n) => (this.host = n.host))
			.catch(() => {});
		this.#sock = connect<BoardMsg>('/ws/board', {
			onOpen: () => {
				this.connected = true;
				void this.#refresh();
			},
			onClose: () => (this.connected = false),
			onMessage: (m) => this.#apply(m)
		});
	}

	find(id: string): BoardRow | undefined {
		return this.rows.find((r) => r.sessionId === id);
	}

	async #refresh() {
		try {
			const res = await getJson<{ sessions: BoardRow[] }>('/api/sessions');
			this.rows = res.sessions;
		} catch {
			// the board stays on the last known rows
		}
	}

	#apply(m: BoardMsg) {
		if (m.t === 'board.upsert') {
			const map = new Map(this.rows.map((r) => [r.sessionId, r]));
			for (const s of m.sessions) map.set(s.sessionId, s);
			this.rows = [...map.values()];
		} else if (m.t === 'board.remove') {
			const gone = new Set(m.ids);
			this.rows = this.rows.filter((r) => !gone.has(r.sessionId));
		}
	}
}

export const board = new Board();
