// Mirrors crates/custom/distill-node/src/sep.rs (source of truth) and CONTRACT §4.

export type Mode = 'ask' | 'auto' | 'always-approve';
export type Status = 'running' | 'waiting_permission' | 'idle' | 'error';

export interface BoardRow {
	sessionId: string;
	cwd: string;
	title: string;
	activity: string;
	resident: boolean;
	yolo: boolean;
	modelId: string;
	isWorktree: boolean;
	lastChangeUnixMs: number;
}

export interface NodeInfo {
	host: string;
	version: string;
}

export interface NewSession {
	cwd: string;
	worktree?: { name: string };
	mode?: Mode;
	prompt?: string;
}

export interface ImageIn {
	mime: string;
	/** base64 without the data: prefix */
	data: string;
}

export type ToolContent =
	| { type: 'text'; text: string }
	| { type: 'image'; mime: string; data: string }
	| { type: 'diff'; path: string; old?: string; new: string };

export interface PermissionOption {
	id: string;
	/** allow_once, allow_always or reject_once */
	kind: string;
	name: string;
}

export type Sep =
	| { t: 'session.state'; status: Status }
	| { t: 'session.title'; title: string }
	| { t: 'user.message'; text: string }
	| { t: 'message.delta'; text: string }
	| { t: 'reasoning.delta'; text: string }
	| {
			t: 'tool';
			id: string;
			name?: string;
			title?: string;
			kind?: string;
			status?: string;
			input?: unknown;
			content?: ToolContent[];
	  }
	| {
			t: 'permission.request';
			id: unknown;
			tool_call_id: string;
			title: string;
			options: PermissionOption[];
	  }
	| { t: 'permission.resolved'; tool_call_id: string }
	| { t: 'usage'; used: number; size: number }
	| { t: 'turn.end'; stop_reason: string; error?: string }
	| { t: 'replay.end' }
	| { t: 'error'; message: string };

export type ClientMsg =
	| { t: 'prompt'; text: string; images: ImageIn[] }
	| { t: 'cancel' }
	| { t: 'permission'; id: unknown; option_id: string }
	| { t: 'mode'; mode: Mode };
