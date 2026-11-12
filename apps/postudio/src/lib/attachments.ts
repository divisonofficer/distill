import type { UserAttachment } from './types';

export const PASTE_TEXT_THRESHOLD = 4000;
export const MAX_IMAGE_BYTES = 10 * 1024 * 1024;
export const MAX_TEXT_BYTES = 1024 * 1024;
export const MAX_ATTACHMENTS = 10;
export const MAX_TOTAL_BYTES = 20 * 1024 * 1024;

export interface DraftAttachment {
	key: number;
	attachment: UserAttachment;
	size: number;
}

export interface ReadAttachmentsResult {
	attachments: DraftAttachment[];
	nextKey: number;
	error: string;
}

const imageMimes = new Set(['image/png', 'image/jpeg', 'image/gif', 'image/webp']);
const mimeByExtension: Record<string, string> = {
	png: 'image/png',
	jpg: 'image/jpeg',
	jpeg: 'image/jpeg',
	gif: 'image/gif',
	webp: 'image/webp'
};

export function isLongPaste(text: string): boolean {
	return text.length > PASTE_TEXT_THRESHOLD;
}

export function textSize(text: string): number {
	return new TextEncoder().encode(text).byteLength;
}

export function isPreviewImageMime(mime: string): boolean {
	return imageMimes.has(mime.toLowerCase());
}

function imageMime(file: File): string | undefined {
	const typed = file.type.toLowerCase();
	if (isPreviewImageMime(typed)) return typed;
	const extension = file.name.split('.').at(-1)?.toLowerCase();
	return extension ? mimeByExtension[extension] : undefined;
}

export function canHandleAttachmentFile(file: File): boolean {
	return (
		imageMime(file) !== undefined ||
		/\.(txt|md)$/i.test(file.name)
	);
}

export function clipboardFiles(data: DataTransfer): File[] {
	if (data.files.length > 0) return Array.from(data.files);
const files = new Set<File>();
	for (const item of Array.from(data.items)) {
		if (item.kind !== 'file') continue;
		const file = item.getAsFile();
		if (file) files.add(file);
	}
	return [...files];
}

export function createTextDraft(name: string, text: string, key: number): DraftAttachment {
	const size = textSize(text);
	if (size > MAX_TEXT_BYTES) throw new Error('텍스트 파일은 1 MiB 이하만 첨부할 수 있습니다.');
	return { key, attachment: { kind: 'text', name, text }, size };
}

function readDataUrl(file: File): Promise<string> {
	return new Promise((resolve, reject) => {
		const reader = new FileReader();
		reader.onload = () => resolve(String(reader.result));
		reader.onerror = () => reject(reader.error ?? new Error('파일을 읽지 못했습니다.'));
		reader.readAsDataURL(file);
	});
}

async function readAttachmentFile(file: File, key: number): Promise<DraftAttachment> {
	const mime = imageMime(file);
	if (mime) {
		if (file.size > MAX_IMAGE_BYTES) throw new Error('이미지는 10 MiB 이하만 첨부할 수 있습니다.');
		const url = await readDataUrl(file);
		const comma = url.indexOf(',');
		if (comma < 0) throw new Error('이미지 파일을 읽지 못했습니다.');
		return {
			key,
			attachment: { kind: 'image', mime, data: url.slice(comma + 1), name: file.name },
			size: file.size
		};
	}

	if (!/\.(txt|md)$/i.test(file.name)) throw new Error('TXT 또는 MD 파일만 첨부할 수 있습니다.');
	if (file.size > MAX_TEXT_BYTES) throw new Error('텍스트 파일은 1 MiB 이하만 첨부할 수 있습니다.');
	return { key, attachment: { kind: 'text', name: file.name, text: await file.text() }, size: file.size };
}

export async function readAttachmentFiles(
	files: File[],
	current: DraftAttachment[],
	nextKey: number
): Promise<ReadAttachmentsResult> {
	const attachments = [...current];
	const errors: string[] = [];
	for (const file of files) {
		if (attachments.length >= MAX_ATTACHMENTS) {
			errors.push('최대 10개까지 첨부할 수 있습니다.');
			break;
		}
		nextKey++;
		try {
			attachments.push(await readAttachmentFile(file, nextKey));
		} catch (error) {
			errors.push(error instanceof Error ? error.message : '파일을 읽지 못했습니다.');
		}
	}
	return { attachments, nextKey, error: [...new Set(errors)].join(' ') };
}

export function nextPastedTextName(attachments: DraftAttachment[]): string {
	const used = new Set(
		attachments.flatMap(({ attachment }) => {
			if (attachment.kind !== 'text') return [];
			const match = /^pasted-text-(\d+)\.txt$/i.exec(attachment.name);
			return match ? [Number(match[1])] : [];
		})
	);
	let next = 1;
	while (used.has(next)) next++;
	return `pasted-text-${next}.txt`;
}

export function mergeDrafts(current: DraftAttachment[], incoming: DraftAttachment[]): { attachments: DraftAttachment[]; error: string } {
const attachments = [...current];
const errors = new Set<string>();
let total = attachments.reduce((sum, item) => sum + item.size, 0);
for (const item of incoming) {
if (attachments.length >= MAX_ATTACHMENTS) { errors.add('최대 10개까지 첨부할 수 있습니다.'); continue; }
if (total + item.size > MAX_TOTAL_BYTES) { errors.add('첨부 전체 크기는 20 MiB 이하만 가능합니다.'); continue; }
attachments.push(item);
total += item.size;
}
return { attachments, error: [...errors].join(' ') };
}
