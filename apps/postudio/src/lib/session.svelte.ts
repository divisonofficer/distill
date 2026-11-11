import type { PermissionOption, Sep, Status, ToolContent } from './types';

export interface TextItem {
	kind: 'text';
	role: 'user' | 'assistant' | 'reasoning' | 'error';
	key: number;
	text: string;
}

export interface ToolItem {
	kind: 'tool';
	key: number;
	id: string;
	name?: string;
	title?: string;
	toolKind?: string;
	status?: string;
	input?: unknown;
	content?: ToolContent[];
}

export type Item = TextItem | ToolItem;

export interface Pending {
	id: unknown;
	toolCallId: string;
	title: string;
	options: PermissionOption[];
}

/** Transcript state for one session WebSocket. */
export class SessionView {
	items = $state<Item[]>([]);
	status = $state<Status | 'offline'>('offline');
	connected = $state(false);
	title = $state('');
	usage = $state<{ used: number; size: number } | null>(null);
	permissions = $state<Pending[]>([]);
	revision = $state(0);

	#seq = 0;
	#toolIndex = new Map<string, number>();

	get running(): boolean {
		return this.status === 'running' || this.status === 'waiting_permission';
	}

	reset() {
		this.items = [];
		this.permissions = [];
		this.usage = null;
		this.title = '';
		this.#toolIndex.clear();
	}

	setConnected(on: boolean) {
		this.connected = on;
		if (!on) this.status = 'offline';
	}

	apply(ev: Sep) {
		this.revision++;
		switch (ev.t) {
			case 'session.state':
				this.status = ev.status;
				break;
			case 'session.title':
				this.title = ev.title;
				break;
			case 'usage':
				this.usage = { used: ev.used, size: ev.size };
				break;
			case 'user.message':
				this.#push('user', ev.text);
				break;
			case 'message.delta':
				this.#append('assistant', ev.text);
				break;
			case 'reasoning.delta':
				this.#append('reasoning', ev.text);
				break;
			case 'tool':
				this.#tool(ev);
				break;
			case 'permission.request':
				if (!this.permissions.some((p) => p.toolCallId === ev.tool_call_id)) {
					this.permissions.push({
						id: ev.id,
						toolCallId: ev.tool_call_id,
						title: ev.title,
						options: ev.options
					});
				}
				break;
			case 'permission.resolved':
				this.permissions = this.permissions.filter((p) => p.toolCallId !== ev.tool_call_id);
				break;
			case 'turn.end':
				if (ev.error) this.#push('error', ev.error);
				break;
			case 'error':
				this.#push('error', ev.message);
				break;
			case 'replay.end':
				break;
		}
	}

	#push(role: TextItem['role'], text: string) {
		this.items.push({ kind: 'text', role, key: ++this.#seq, text });
	}

	#append(role: 'assistant' | 'reasoning', text: string) {
		const last = this.items.at(-1);
		if (last?.kind === 'text' && last.role === role) {
			last.text += text;
			return;
		}
		this.items.push({ kind: 'text', role, key: ++this.#seq, text });
	}

	#tool(ev: Extract<Sep, { t: 'tool' }>) {
		let idx = this.#toolIndex.get(ev.id);
		if (idx === undefined) {
			idx = this.items.push({ kind: 'tool', key: ++this.#seq, id: ev.id }) - 1;
			this.#toolIndex.set(ev.id, idx);
		}
		const item = this.items[idx];
		if (item?.kind !== 'tool') return;
		if (ev.name !== undefined) item.name = ev.name;
		if (ev.title !== undefined) item.title = ev.title;
		if (ev.kind !== undefined) item.toolKind = ev.kind;
		if (ev.status !== undefined) item.status = ev.status;
		if (ev.input !== undefined) item.input = ev.input;
		if (ev.content !== undefined) item.content = ev.content;
	}
}
