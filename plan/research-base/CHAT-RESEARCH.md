# 채팅 모드 × OpenResearch — 설계와 단계

> 목표: Postudio **채팅 모드**가 OpenResearch의 스킬, 논문 검색 등 기본 기능을 그대로 쓸 수 있게 한다.
> 기준 OpenResearch: `../OpenResearch` @ `cb75d884` (orx 0.2.27)

## 1. 현재 차이

OpenResearch의 채팅은 **에이전트 세션**이다. 구성 요소:
- 프로젝트 저장소의 채팅 전용 worktree
- 시스템 프롬프트(`SYSTEM_PROMPT.md` playbook): 증거 링크, 이미지 표시, 스킬 라우팅
- `orx-*` 네이티브 스킬
- `orx` CLI와 alphaXiv MCP 도구

Postudio의 일반 채팅은 Distill `x.ai/chat/complete`로 보내는 **도구 없는 단발 완성**이다(`distill-shell/src/extensions/chat_complete.rs`, 테스트명 `sends_tool_free_bounded_request`). 그래서 그대로는 검색도 스킬도 쓸 수 없다.

| OpenResearch 기능 | Postudio 현재 | 계획 |
|---|---|---|
| 문헌 검색: alphaXiv keyword·embedding, OpenAlex, bioRxiv, PubMed | 연구 화면은 4개 소스(embedding 없음). **에이전트 도구 `search_papers`: 5개 모두 (C1 완료)** | 채팅에서 사용 (C2) |
| 논문 읽기: report·full text·메타데이터 | **`get_paper`, 2.4만 자 페이징 (C1 완료)** | C2 |
| 스킬: overview + `orx-*` 14개, 지연 로딩 | **`list_skills`, `read_skill` + Postudio 안내문 (C1 완료)** | playbook이 스킬을 먼저 읽도록 유도 (C2) |
| Playbook: 증거 링크, 인라인 이미지, 수식, 경고 표기 | 없음 | 채팅용 playbook (C2) |
| 프로젝트·실험 트리·실행·로그·지표 | 연구 화면 + 실험 채팅 MCP 도구 7개 | 채팅에 프로젝트 연결, 로그·실행 목록 도구 추가 (C4) |
| 결과 해석, 보고서, 그림(artifacts) | 채팅에 없음 | 채팅 작업 폴더 + 이미지/HTML 렌더링 (C3) |
| LaTeX 논문 초안 (`orx-paper`) | 없음 | 채팅 작업 폴더에서 초안. 본격 기능은 Writing 백로그 |
| 하위 작업 위임 (`delegate`/`wait`) | Distill 서브에이전트 내장 | Distill task 도구로 대체 |
| 사용자 스킬·LaTeX 템플릿 추가 | 없음 | C5 |
| alphaXiv 라이브러리, `request_env`, 원격 컴퓨트(slurm/k8s/modal…) | 로컬 + SSH 대상만 | C5 |
| `orx feedback` (업스트림 보고) | — | 제외 |

## 2. 아키텍처 (C2)

```
채팅 화면 ── POST /api/chats/:id/messages/:m/completion ──▶ node
                                                            │ ChatCompleter
                         ┌──────────────────────────────────┴───────────────┐
                 도구 없는 채팅 (현행)                         연구 채팅 (신규: AgentChatCompleter)
                 x.ai/chat/complete                            ├ 채팅 전용 Distill 세션 (이 호스트)
                                                               │   cwd = state/chat-workspaces/<chatId>
                                                               ├ 연구 MCP: 프로젝트 없는 "라이브러리 grant"
                                                               │   (search_papers, get_paper, list/read_skill)
                                                               ├ 채팅 playbook + 이전 대화(참고 자료)
                                                               └ 최종 답 → chat_store (동기화되는 원본)
```

- **원본은 `chat_store`**: 사용자·어시스턴트 메시지만 저장·동기화한다. 세션은 이 호스트의 실행 캐시다.
- **다른 호스트에서 이어가기**: 그 호스트에 새 세션을 만들고 `chat_store`의 이전 대화를 참고 자료로 넣는다. 작업 폴더 파일은 원래 호스트에 남는다.
- **grant**: 현재 grant는 프로젝트·실험 범위다. 프로젝트 없는 라이브러리 grant를 추가하고, 그 grant에서는 프로젝트 도구를 거부한다.
- **진행 표시**: 현재 진행 API는 텍스트뿐이다. C2에서는 텍스트 + "도구 실행 중: search_papers" 상태 줄을 보여주고, 승인이 필요하면 세션 화면 링크를 띄운다. 채팅 안 승인 카드는 C3에서 한다.

## 3. 단계

| 단계 | 내용 | 상태 |
|---|---|---|
| **C1** | 연구 MCP에 문헌·스킬 도구 4개. 실험 채팅에서 바로 사용 | **완료** (`2b16122c`) |
| **C2** | 연구 채팅 런타임: 채팅별 세션·작업 폴더, 라이브러리 grant, playbook, 이전 대화 주입, 진행 상태, `chat_store` 저장 | 결정 대기 |
| C3 | 채팅 UI: 도구 타임라인, 논문 카드, 인라인 이미지·HTML(작업 폴더 artifacts), 인라인 승인 | |
| C4 | 채팅에 연구 프로젝트 연결: 프로젝트 범위 grant, 실행 목록·로그(페이징)·지표·실험 생성·실행 요청(승인) | |
| C5 | 사용자 스킬·템플릿, alphaXiv 라이브러리, 키 요청, 원격 컴퓨트 | |

## 4. 결정이 필요한 사항

1. **전환 방식**: (a) 모든 일반 채팅을 에이전트로 바꿀지, (b) 채팅별 "연구 모드" 토글로 둘지. 권장은 (b)이고, OpenResearch가 연결된 호스트에서는 기본을 켜 두는 방식이다. 빠른 도구 없는 채팅을 남겨 둘 수 있다.
2. **C2 도구 범위**: 권장은 문헌·스킬(읽기 전용, 승인 불필요) + 채팅 작업 폴더에서의 파일 작성·명령 실행(Distill 권한 모드에 따라 승인)이다. 이렇게 하면 그림·보고서·LaTeX 초안까지 가능하다. 다른 선택지는 읽기 전용만 여는 것이다.
3. **기본 권한 모드**: 수동(매번 승인)으로 할지, 자동(분류기)으로 할지.
4. **다른 호스트에서 이어가기**: 위 2절 방식(새 세션 + 이전 대화 주입)으로 충분한지.
