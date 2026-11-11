// Mirrors crates/custom/distill-node/src/sep.rs (source of truth) and CONTRACT §4.

export type Mode = 'ask' | 'auto' | 'always-approve';
export type Status = 'running' | 'waiting_permission' | 'idle' | 'error';

export interface BoardRow {
	sessionId: string;
	cwd: string;
	title: string;
	activity: string;
	resident: boolean;
	yolo?: boolean;
	autoMode?: boolean;
	modelId: string;
	isWorktree: boolean;
	lastChangeUnixMs: number;
}

export function modeFromRow(row: Pick<BoardRow, 'autoMode' | 'yolo'> | undefined): Mode | undefined {
	if (!row) return undefined;
	if (row.yolo === true) return 'always-approve';
	if (row.autoMode === true && row.yolo === false) return 'auto';
	if (row.autoMode === false && row.yolo === false) return 'ask';
	return undefined;
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

export interface QuestionOption {
	label: string;
	description: string;
}

export interface Question {
	question: string;
	options: QuestionOption[];
	multiSelect: boolean;
}

export type InteractionRequest =
	| {
			t: 'question.request';
			id: string | number;
			session_id: string;
			tool_call_id: string;
			mode: 'default' | 'plan';
			questions: Question[];
			preview?: string;
	  }
	| {
			t: 'plan.request';
			id: string | number;
			session_id: string;
			tool_call_id: string;
			plan_content?: string;
	  };

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
	| Extract<InteractionRequest, { t: 'question.request' }>
	| Extract<InteractionRequest, { t: 'plan.request' }>
	| { t: 'permission.resolved'; tool_call_id: string }
	| { t: 'usage'; used: number; size: number }
	| { t: 'turn.end'; stop_reason: string; error?: string }
	| { t: 'replay.end' }
	| { t: 'error'; message: string };

export type ClientMsg =
	| { t: 'prompt'; text: string; images: ImageIn[] }
	| { t: 'cancel' }
	| { t: 'permission'; id: unknown; option_id: string }
	| {
			t: 'question_reply';
			id: string | number;
			session_id: string;
			tool_call_id: string;
			outcome: 'accepted';
			answers: Record<string, string[]>;
	  }
	| {
			t: 'question_reply';
			id: string | number;
			session_id: string;
			tool_call_id: string;
			outcome: 'cancelled';
	  }
	| {
			t: 'plan_reply';
			id: string | number;
			session_id: string;
			tool_call_id: string;
			outcome: 'approved' | 'cancelled' | 'abandoned';
			feedback?: string;
	  }
	| { t: 'mode'; mode: Mode };
