import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createServer } from 'vite';

test('chat tool receipts are separate from the answer and deduplicate source URLs', async () => {
 const vite = await createServer({ server: { middlewareMode: true }, appType: 'custom', logLevel: 'silent' });
 try {
  const { splitChatActivity } = await vite.ssrLoadModule('/src/lib/chat-activity.ts');
  const source = '- [Paper](<https://arxiv.org/abs/2601.12345>) · 2026-01-01';
  const receipts = `**alphaXiv 논문 검색: 결과 1건**\n${source}\n\n**Brave 웹 검색: 결과 1건**\n${source}`;
  const answer = splitChatActivity(`답변\n\n---\n\n${receipts}`);
  assert.equal(answer.text, '답변');
  assert.equal(answer.calls.length, 2);
  assert.equal(answer.sources.length, 1);
  const live = splitChatActivity(`${receipts}\n\n**응답 정리 중…**`, true);
  assert.equal(live.writing, true);
  assert.equal(live.text, '');
  assert.equal(splitChatActivity('**alphaXiv 논문 검색 실행 중…**', true).calls[0].status, '실행 중…');
  assert.equal(splitChatActivity(`본문\n\n---\n\n**alphaXiv 논문 검색: 결과 1건**\n- [bad](<javascript:alert(1)>)`).calls.length, 0);
  const read = splitChatActivity(`답변\n\n---\n\n${receipts}\n\n**논문 본문 읽기: 본문 발췌 확인**\n- [Paper](<https://arxiv.org/abs/2601.12345>) · Kim et al. · 2026 · arXiv 프리프린트\n\n**논문 본문 읽기: 본문을 가져오지 못함**`);
  assert.equal(read.calls.at(-2).status, '본문 발췌 확인');
  assert.equal(read.calls.at(-1).status, '본문을 가져오지 못함');
  assert.equal(read.sources.length, 1);
  assert.equal(read.sources[0].citation, 'Kim et al. · 2026 · arXiv 프리프린트');
  assert.equal(read.sources[0].date, '2026-01-01');
  const ordinary = '본문\n\n---\n\n다음 문단';
  assert.equal(splitChatActivity(ordinary).text, ordinary);
 } finally { await vite.close(); }
});
