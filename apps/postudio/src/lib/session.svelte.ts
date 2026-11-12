import type { PermissionOption, Sep, Status, ToolContent, UserAttachment } from './types';

export function sessionDestination(search: string, configured: (origin: string) => boolean): string {
const host = new URLSearchParams(search).get('host');
if (host === null) return '';
try {
const url = new URL(host);
if (!['http:', 'https:'].includes(url.protocol) || url.origin !== host || url.username || url.password || url.pathname !== '/' || url.search || url.hash || !configured(url.origin)) throw new Error('Invalid or unconfigured session host');
return url.origin;
} catch (error) { throw error instanceof Error && error.message.startsWith('Invalid') ? error : new Error('Invalid session host'); }
}

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

export interface AttachmentItem {
	kind: 'attachment';
	key: number;
	attachment: UserAttachment;
}

export type Item = TextItem | ToolItem | AttachmentItem;

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
	modelId = $state('');
	usage = $state<{ used: number; size: number } | null>(null);
	permissions = $state<Pending[]>([]);
	revision = $state(0);

	#seq = 0;
	#canAppend = false;
	#toolIndex = new Map<string, number>();

	get running(): boolean {
		return this.status === 'running' || this.status === 'waiting_permission';
	}

	reset() {
		this.items = [];
		this.permissions = [];
		this.usage = null;
		this.title = '';
		this.modelId = '';
		this.#toolIndex.clear();
		this.#canAppend = false;
	}

	applyHistoryPage(items: Sep[], initial: boolean) {
		if (initial) {
			this.items = [];
			this.#toolIndex.clear();
			this.#canAppend = false;
			for (const item of items) this.apply(item);
			return;
		}
		const previousCanAppend = this.#canAppend;
		const page = new SessionView();
		for (const item of items) page.apply(item);
		const additions: Item[] = [];
		for (const item of page.items) {
			if (item.kind === 'tool' && this.#toolIndex.has(item.id)) {
const existing = this.items.find(current => current.kind === 'tool' && current.id === item.id);
if (existing?.kind === 'tool') {
const merged = { ...item, ...existing };
this.items = this.items.filter(current => current !== existing);
additions.push(merged);
continue;
}
}
			item.key = ++this.#seq;
			additions.push(item);
		}
		this.items = [...additions, ...this.items];
		this.#toolIndex.clear();
		this.items.forEach((item, index) => { if (item.kind === 'tool') this.#toolIndex.set(item.id, index); });
		this.#canAppend = previousCanAppend;
		this.revision++;
	}

	setConnected(on: boolean) {
		this.connected = on;
		if (!on) this.status = 'offline';
else this.status = 'idle';
	}

	apply(ev: Sep) {
		this.revision++;
		switch (ev.t) {
			case 'session.state':
				this.status = ev.status;
				if (ev.status === 'error') {
					this.permissions = [];
					this.#canAppend = false;
				}
				break;
			case 'session.title':
				this.title = ev.title;
				break;
			case 'session.model':
				this.modelId = ev.model_id;
				break;
			case 'usage':
				this.usage = { used: ev.used, size: ev.size };
				break;
			case 'user.message':
				this.#push('user', ev.text);
				break;
			case 'user.attachment':
				this.items.push({ kind: 'attachment', key: ++this.#seq, attachment: ev.attachment });
				this.#canAppend = false;
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
			case 'question.request':
case 'plan.request':
this.status = 'waiting_permission';
break;
case 'permission.request':
this.status = 'waiting_permission';
this.permissions = [...this.permissions.filter((p) => p.toolCallId !== ev.tool_call_id), {
id: ev.id, toolCallId: ev.tool_call_id, title: ev.title, options: ev.options
}];
break;
case 'permission.expired':
this.permissions = this.permissions.filter((p) => p.id !== ev.id);
break;
case 'permission.resolved':
				this.permissions = this.permissions.filter((p) => p.toolCallId !== ev.tool_call_id);
				break;
			case 'turn.end':
				this.permissions = [];
				this.#canAppend = false;
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
		this.#canAppend = false;
	}

	#append(role: 'assistant' | 'reasoning', text: string) {
		const last = this.items.at(-1);
		if (this.#canAppend && last?.kind === 'text' && last.role === role) {
			last.text += text;
			return;
		}
		this.items.push({ kind: 'text', role, key: ++this.#seq, text });
		this.#canAppend = true;
	}

	#tool(ev: Extract<Sep, { t: 'tool' }>) {
		this.#canAppend = false;
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

export function restoredHistoryScrollTop(previous: { scrollTop: number; scrollHeight: number }, nextScrollHeight: number): number {
return previous.scrollTop + nextScrollHeight - previous.scrollHeight;
}

export class SessionReplay {
#view: SessionView;
#pending = false;
#historyLoaded = false;
#liveTranscript: Sep[] = [];

constructor(view: SessionView) {
this.#view = view;
}

begin() {
this.#pending = true;
this.#historyLoaded = false;
this.#liveTranscript = [];
}

apply(ev: Sep): boolean {
if (ev.t === 'history.page') {
this.#view.applyHistoryPage(ev.items, ev.initial);
if (ev.initial) {
this.#historyLoaded = true;
for (const live of this.#liveTranscript) this.#view.apply(live);
this.#liveTranscript = [];
}
return false;
}
if (this.#pending && !this.#historyLoaded && isTranscriptEvent(ev)) {
this.#liveTranscript.push(ev);
} else {
this.#view.apply(ev);
}
if (!this.#pending || ev.t !== 'replay.end') return false;
if (!this.#historyLoaded) {
for (const event of this.#liveTranscript) this.#view.apply(event);
this.#liveTranscript = [];
}
this.#pending = false;
return true;
}
}

function isTranscriptEvent(ev: Sep): boolean {
return ev.t === 'user.message' || ev.t === 'user.attachment' || ev.t === 'message.delta' ||
ev.t === 'reasoning.delta' || ev.t === 'tool' || ev.t === 'error' ||
ev.t === 'turn.end';
}

export type TimelineEntry = { kind: 'item'; item: Item } | { kind: 'tools'; items: ToolItem[] };

export function groupCompletedTools(items: Item[]): TimelineEntry[] {
const entries: TimelineEntry[] = [];
for (let index = 0; index < items.length;) {
const item = items[index];
if (item?.kind === 'tool' && item.status === 'completed') {
const group: ToolItem[] = [item];
while (true) {
const next = items[index + 1];
if (next?.kind !== 'tool' || next.status !== 'completed') break;
index++;
group.push(next);
}
entries.push(group.length > 1 ? { kind: 'tools', items: group } : { kind: 'item', item });
} else if (item) {
entries.push({ kind: 'item', item });
}
index++;
}
return entries;
}
