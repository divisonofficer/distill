import type { TextFileIn } from './types';

// General chat's wire format only carries text files, not images or PDFs.
export const CHAT_ATTACHMENT_HELP = 'TXT · MD · 최대 10개 · 전체 1 MiB';
export const CHAT_MAX_FILES = 10;
export const CHAT_MAX_BYTES = 1024 * 1024;

export async function readChatTextFiles(files: File[], current: TextFileIn[]): Promise<{ files: TextFileIn[]; error: string }> {
 const accepted = [...current];
 const errors: string[] = [];
 let total = accepted.reduce((sum, file) => sum + new TextEncoder().encode(file.text).byteLength, 0);
 for (const file of files) {
  try {
   if (!/\.(txt|md)$/i.test(file.name)) throw new Error('지원하지 않는 형식이에요. TXT/MD 파일만 첨부할 수 있어요.');
   if (accepted.length >= CHAT_MAX_FILES) throw new Error('최대 10개까지 첨부할 수 있어요.');
   if (file.size > CHAT_MAX_BYTES || total + file.size > CHAT_MAX_BYTES) throw new Error('첨부 전체 크기는 1 MiB까지예요.');
   const text = await file.text();
   const bytes = new TextEncoder().encode(text).byteLength;
   if (total + bytes > CHAT_MAX_BYTES) throw new Error('첨부 전체 크기는 1 MiB까지예요.');
   if (text.includes('\0') || text.includes('\uFFFD')) throw new Error('UTF-8 텍스트로 읽을 수 없어요. 파일 인코딩을 확인해 주세요.');
   accepted.push({ name: file.name, text });
   total += bytes;
  } catch (cause) {
   errors.push(`${file.name}: ${cause instanceof Error ? cause.message : '파일을 읽지 못했어요.'}`);
  }
 }
 return { files: accepted, error: errors.join(' ') };
}
