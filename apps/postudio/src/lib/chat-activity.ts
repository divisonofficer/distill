export type ChatSource = { title: string; url: string; date?: string; citation?: string };
export type SearchActivity = { label: string; status: string; hits: ChatSource[] };
const heading = /^\*\*(alphaXiv 논문 검색|Brave 웹 검색|막대 차트|논문 본문 읽기|논문 원문 그림)(?:: (결과 \d+건|검색을 완료하지 못함|본문 발췌 확인|본문을 가져오지 못함|그림 \d+ 표시|그림 \d+개 확인|그림을 가져오지 못함)| (실행 중…))\*\*$/;
const hit = /^- \[([^\]\n]{1,160})\]\(<([^<>\s]{1,2048})>\)(?: · ([^\r\n]{1,150}))?$/;

// Strict adapter for the current server's receipt format. Ordinary Markdown stays untouched.
export function splitChatActivity(value: string, live = false) {
 const empty = { text: value, calls: [] as SearchActivity[], sources: [] as ChatSource[], writing: false };
 const separator = '\n\n---\n\n';
 const boundary = live ? 0 : value.lastIndexOf(separator);
 if (boundary < 0) return empty;
 const receipt = live ? value : value.slice(boundary + separator.length);
 const calls: SearchActivity[] = [];
 let writing = false;
 for (const line of receipt.split('\n')) {
  if (!line.trim()) continue;
  if (writing) return empty;
  if (live && line === '**응답 정리 중…**' && calls.length) { writing = true; continue; }
  const match = heading.exec(line);
  if (match && (live || !match[3])) { calls.push({ label: match[1], status: match[2] ?? match[3], hits: [] }); continue; }
  const source = hit.exec(line);
  const call = calls.at(-1);
  if (!source || !call || !(call.status.startsWith('결과 ') || call.status === '본문 발췌 확인')) return empty;
  try {
   const url = new URL(source[2]);
   if (!['https:', 'http:'].includes(url.protocol) || url.username || url.password) return empty;
   const meta = source[3];
   call.hits.push({ title: source[1], url: url.href, ...(meta ? /^\d{4}-\d{2}-\d{2}$/.test(meta) ? { date: meta } : { citation: meta } : {}) });
  } catch { return empty; }
 }
 if (!calls.length) return empty;
 const unique = new Map<string, ChatSource>();
 for (const call of calls) for (const source of call.hits) {
  const previous = unique.get(source.url);
  if (!previous) unique.set(source.url, source);
  else if (source.citation) unique.set(source.url, { ...previous, citation: source.citation, title: /^arXiv\s/i.test(previous.title) ? source.title : previous.title });
 }
 return { text: live ? '' : value.slice(0, boundary), calls, sources: [...unique.values()], writing };
}
