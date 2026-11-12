// Mirrors crates/custom/distill-node/src/sep.rs (source of truth) and CONTRACT §4.

export type Mode = 'ask' | 'auto' | 'always-approve';
export type Status = 'running' | 'waiting_permission' | 'idle' | 'error';

export interface BoardRow {
apiOrigin?: string;
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
	parentSessionId?: string;
	sessionKind?: string;
	sourceWorkspaceDir?: string;
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
	untrustedOrigin?: string;
}

export interface Model {
	modelId: string;
	name: string;
}

export interface ModelCatalog {
	currentModelId: string | null;
	availableModels: Model[];
}

export interface NewSession {
	cwd: string;
	modelId?: string;
	worktree?: { name: string };
	mode?: Mode;
	prompt?: string;
	images?: ImageIn[];
	text_files?: TextFileIn[];
}

export interface ImageIn {
	mime: string;
	/** base64 without the data: prefix */
	data: string;
}

export interface TextFileIn {
	name: string;
	text: string;
}

export type UserAttachment =
	| ({ kind: 'image' } & ImageIn & { name?: string })
	| { kind: 'text'; name: string; text: string }
	| { kind: 'file'; name: string; uri?: string };

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
	| { t: 'session.model'; model_id: string }
	| { t: 'user.message'; text: string }
	| { t: 'user.attachment'; attachment: UserAttachment }
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
	| { t: 'permission.expired'; id: unknown }
| { t: 'permission.resolved'; tool_call_id: string }
	| { t: 'usage'; used: number; size: number }
	| { t: 'turn.end'; stop_reason: string; error?: string }
	| { t: 'history.page'; items: Sep[]; cursor: string | null; has_more: boolean; initial: boolean; error?: string }
	| { t: 'replay.end' }
	| { t: 'error'; message: string };

export type ClientMsg =
	| { t: 'history.load'; cursor: string }
	| { t: 'prompt'; text: string; images: ImageIn[]; text_files?: TextFileIn[] }
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
	| { t: 'mode'; mode: Mode }
	| { t: 'model'; model_id: string };
