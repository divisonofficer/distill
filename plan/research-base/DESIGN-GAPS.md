# 최신 목업 차이 추적

기준 목업: `/tmp/postudio-verify-latest-d95bb57eca60/App.dc.html` (SHA-256 `24692bdaf3332a649ac9aca8778883e30f20630851c0c200bd7b90806ed79c8f`). 참조 ZIP SHA-256 `d95bb57eca6023c6b05db3a21e1e7be26eace9a393d7b01bf3e0fc7fed054d2e`. 현재 비교본은 ZIP 내부 HTML과 바이트 단위로 일치한다. 목업 위치는 해당 HTML 줄 번호다. 구성요소 존재와 실제 목업 일치·브라우저 검증은 별개다.

| ID | 목업 위치 | 현재 구현 위치·근거 | 차이·변경 상태 | 실제 브라우저 결과 |
|---|---|---|---|---|
| C-01 | 헤더·프로젝트 선택 234–258, 임시 채팅 253–256 | 일반 채팅 `apps/postudio/src/routes/c/[id]/+page.svelte`; 앱 셸 `apps/postudio/src/routes/+layout.svelte` | 프로젝트 선택/임시 대화 전환은 현재 백엔드 계약에 없어 미지원. 모델 선택기는 헤더가 아닌 작성기에 둔다. **프로젝트·임시 대화 계약 대기** | 프로젝트·임시 전환 미지원; 모델 선택기 위치는 C-02 최신 브라우저 검증 참조 |
| C-02 | 작성기 모델 선택·도구·첨부 520–583 | `apps/postudio/src/lib/components/ModelSelect.svelte`; `apps/postudio/src/lib/components/ComposerRouting.svelte`; `apps/postudio/src/routes/c/[id]/+page.svelte`; `apps/postudio/src/routes/new/+page.svelte`; `apps/postudio/src/routes/s/[id]/+page.svelte` | 입력창 모델 버튼 높이는 34px, 최대 폭은 180px이며 현재 모델명을 유지한다. 검색은 폭 290px의 독립 팝업 안에 둔다. 일반 채팅·새 작업·기존 작업에서 직접 모델 메뉴를 열고, 계정 라우팅 정보는 별도 버튼으로 분리했다. 설정의 폼 입력 방식은 유지한다. **구현·브라우저 검증됨** | `/tmp/postudio-structure-5174-pass.log`, `/tmp/postudio-structure-5190-pass.log`: each 18 PASS, 0 FAIL; picker tested at 1280/375 across all three routes, settings kept editable |
| C-03 | 도구 선택·첨부 칩 520–558 | `apps/postudio/src/routes/c/[id]/+page.svelte`, `apps/postudio/src/lib/chats.ts`; `crates/custom/distill-node/src/{attachments,chats,chat_store}.rs` | TXT/MD 첨부 저장·재표시·모델 전달 구현. 요청 `textFiles: [{name,text}]`, 최대 10개·합계 1 MiB. 선택한 분기의 파일 내용을 기존 텍스트 생성 계약으로 전달한다. 전체 생성 입력은 128개 메시지·1 MiB까지이며 초과 시 모델 호출 전 거부한다. 빈 파일 필드는 직렬화에서 제외해 기존 서명 입력을 보존한다. 이미지·도구는 미지원. 일반 채팅/작업 첨부 진입 메뉴는 아직 완전히 통일되지 않음: 일반은 TXT/MD(기능 게이트), session/new는 네이티브 파일 선택. 승인 정책은 실행 프리셋이 아님. **코드·검증 완료, 실행 백엔드 배포 대기** | 두 포트의 모의 API 브라우저 검증 각 10개 통과: 반복 선택·미리보기·제거·파일만 저장·실패 유지·편집 분기·대화 이동·재마운트·기능 지원 상실·누락된 저장 응답·읽기 중 이탈. 네이티브 파일 선택/업로드 별도 통과. Rust 전체 148개(직렬), 프런트엔드 125개, 타입 검사·빌드 통과(이전 체크포인트). 실제 모델 호출과 실행 중 서버 재시작은 하지 않았다. |
| C-04 | 프로젝트 선택·임시 채팅 234–258 | `apps/postudio/src/routes/c/[id]/+page.svelte`; `crates/custom/distill-node/src/chats.rs:390–430` | 프로젝트/임시 토글은 목업에 명시돼 있다. 일반 채팅 API에 해당 계약 없음. **서버 계약 필요** | 미검증 — 대화 상태 및 전환 결과 대기 |
| C-05 | 마이크·받아쓰기 제어 579–582 | `apps/postudio/src/routes/c/[id]/+page.svelte` | 목업에는 명시적 마이크 버튼, 받아쓰기 시작/중지 레이블이 있다. 일반 채팅에 음성 입력 제어/API 없음. **UI 및 음성 계약 필요** | 미검증 — 권한·인식·작성기 반영 결과 대기 |
| C-06 | 생성 중단 497–500 | `apps/postudio/src/routes/c/[id]/+page.svelte`; `crates/custom/distill-node/src/chats.rs:403–430` | 목업에는 스트림 중단 버튼이 있다. 일반 채팅 라우트에 취소 엔드포인트 없음. **서버 취소 계약 필요** | 지원되지 않는 중지는 노출하지 않고 중복 생성 방지·진행/완료 상태 확인; 실제 중지는 서버 계약까지 차단 |
| C-07 | 메시지 인라인 수정 328–345 | `apps/postudio/src/routes/c/[id]/+page.svelte:339–343, 711, 775–787`; 저장/분기 상태 `apps/postudio/src/lib/chats.ts` | 기존 메시지는 그대로 두고 기존 메시지의 `parentId`를 사용해 새 형제 분기를 저장한다. 인라인 편집 구현. **구현·검증됨** | 취소/Escape 시 초안·부모·포커스 복원, 실패 후 초안 유지 및 재시도, 원본 불변 및 모바일 인라인 배치 확인 |
| C-08 | 분기 탐색/목록 297–325, 새 시작 분기 511–516 | `apps/postudio/src/routes/c/[id]/+page.svelte:576–610, 654–658, 711, 740`; `apps/postudio/src/lib/chats.ts` | 현재 분기 탐색기와 `parentId` 기반 새 분기 흐름이 있다. 저장·새 시작 분기는 `⋯`로 분리했다. 목업의 분기 바·목록 배치와 밀도는 추가 대조가 필요하다. 분기 동작 확인됨; 목업과 레이아웃/밀도 차이는 남음. **부분 검증** | 저장만, 새 루트, 기존 부모 형제 분기, 분기 탐색 테스트됨; 전체 목업 일치 주장 안 함 |
| C-09 | 대화 로딩 상태 272–295, 생성 스트림 490–500 | `apps/postudio/src/routes/c/[id]/+page.svelte`; API `crates/custom/distill-node/src/chats.rs` | 목업에는 대화 로딩 골격과 스트림 상태가 있다. 구현 구성요소 존재만으로 최신 브라우저 동작은 확인되지 않는다. 로딩 지연 골격, 생성 진행 및 완료 상태 확인됨. 오프라인 전송/오류 전체 매트릭스는 미검증. **부분 검증** | 지연 기록 로딩, 생성 진행/중복 방지/완료 테스트됨; 전체 오류 매트릭스 대기 |
| W-01 | 작업 화면 상단 헤더·탭; 모바일 그룹 메뉴 | `apps/postudio/src/routes/s/[id]/+page.svelte`; `apps/postudio/src/routes/+layout.svelte`; `apps/postudio/src/lib/components/Sidebar.svelte` | 작업 헤더는 모든 작업 상태에서 숨김. 데스크톱 탭스트립 중복 헤더 없음; 모바일 각 focused group에 메뉴 제공, 데스크톱 전용 메뉴 숨김. 로딩 중 자체 탐색 유지, 메타데이터 중복 없음. **브라우저 검증됨** | 양쪽 structure 로그 각 18 PASS: 펼침/접힘 헤더, 모바일 두 번째 그룹 메뉴/자료 drawer, 로딩 전 탐색 확인 |
| W-02 | 목업 작업 패널의 도구 요약·상세 | `apps/postudio/src/lib/components/WorkbenchPane.svelte`; `apps/postudio/src/lib/components/SessionConversation.svelte`; `apps/postudio/src/lib/components/ToolRow.svelte` | 요약은 인라인 표시, 상세 탭은 명시 버튼에서만 연다. read_file target_file, grep pattern/path/glob, todo_write 실제 배열 수, spawn_subagent 실제 description 표시. 성공 행 조밀 그룹화; 실패 exit 보존·펼친 상태. 전문 도구 카드는 여전히 부분적 의미 행이며 완전한 mock 일치 아님. **핵심 요약 검증됨·전문 카드 부분 구현** | 양쪽 structure 로그 각 18 PASS: 인라인/상세 분리, 실값 요약, 조밀 행, 실패 출력 유지 |
| W-03 | 작업 탭 승인·실행 모니터·결과 | `apps/postudio/src/lib/components/WorkbenchPane.svelte`; `apps/postudio/src/lib/components/SessionMonitor.svelte`; `apps/postudio/src/routes/s/[id]/+page.svelte` | 하단 실행표시는 실제 활성 명령과 범위가 지정된 작업 에이전트 수를 쓴다. 자식 목록/취소는 실제 상태이며 모니터는 변경 시각 사용, 경과시간 아님. BACKGROUND lifecycle/result payload/jobID, 신뢰 가능한 시작 시각, 전용 agent-result attach/intervention API는 미구현; 가짜 UI로 채우지 않음. 승인 정책은 실행 프리셋 아님. **일부 검증·백엔드/계약 대기** | 양쪽 structure 로그 각 18 PASS: 실제 명령/검증된 agent 수, 부모·자식 취소 범위. 나머지 API 미검증·미구현 |
| W-04 | 해당 목업의 작업 탭 상태·본문 | `apps/postudio/src/routes/s/[id]/+page.svelte`; `apps/postudio/src/lib/components/WorkbenchPane.svelte` | 작업 탭 새로고침 유지 수정은 포트 5174·5190에서 각각 여섯 사례 통과. 참조·초안·스크롤 유지, WebSocket 하나만 유지 확인. 이 검증은 W-01 헤더 검증과 별개다. **해당 수정 검증됨** | 5174·5190 각 6사례 통과; test 118, check 0 |
| G-01 | 작업 화면 질문 카드·스낵바 706–716 | `apps/postudio/src/lib/components/InteractionCard.svelte`; `apps/postudio/src/routes/+layout.svelte`; `apps/postudio/src/lib/components/Snackbar.svelte`; 일반 채팅 `apps/postudio/src/routes/c/[id]/+page.svelte` | 질문 카드는 중립 배경과 높이 36px의 작은 버튼을 사용하며 단일·다중 선택 및 직접 입력 동작을 유지한다. 저장·제목 성공 알림은 실제 ACK 후 표시; 오류·닫기·타이머 처리. **브라우저 검증됨** | 양쪽 structure 로그 각 18 PASS: 질문 응답 유형/모바일·데스크톱, 저장 실패 후 초안 보존·ACK 성공 토스트 닫기/시간종료, 제목 오류·ACK 알림 |

## TXT/MD 첨부 체크포인트

- 실행 백엔드는 재시작하지 않았다. `textAttachments` 기능 정보를 제공하는 새 노드로 배포한 뒤 파일 선택이 활성화된다. 구버전에는 빈 파일 필드를 보내지 않아 일반 텍스트 저장을 보존한다.
- 브라우저 스크립트: `/tmp/postudio-text-attachments-browser.js`; 최종 로그: `/tmp/postudio-textfiles-5174-final.log`, `/tmp/postudio-textfiles-5190-final-v2.log`. 각 10개 통과. 네이티브 파일 선택: `/tmp/postudio-native-picker.log`.
- `cargo test -p distill-node --target-dir target-node -- --test-threads=1`: 148개 통과. 앞선 병렬 실행의 로그인 시간 제한 실패는 직렬 실행에서 통과했다.
- 프런트엔드 전체 125개 통과, `npm run check` 오류·경고 0개, `npm run build` 성공(첨부 구현 당시 결과이며, 최신 구조 변경 검증은 아래 별도 기록).
- 스크립트·로그·화면 캡처: 세션 자산 `postudio-text-attachments-20261113`. 실제 유료 모델 호출 및 실제 대화 변경은 수행하지 않았다.

## 단계 순서

텍스트 첨부(TXT/MD) → 네이티브 이미지 첨부 → 도구 → 음성. 단계별로 하나씩 진행한다.

## 브라우저 증거

- 최신 구조 검증 로그: `/tmp/postudio-structure-5174-pass.log`, `/tmp/postudio-structure-5190-pass.log`. 각각 18 PASS, 0 FAIL; 여섯 모델 picker route/viewport 조합, 도구 요약, 질문·토스트, footer/child monitor, 헤더·모바일 탐색·로딩·settings·공유 라우트 포함.
- 이 구조 검증은 Playwright API fixture 기반이며 실제/유료 모델 호출이나 실제 데이터 변경을 뜻하지 않는다.
- 이전 채팅 정렬 검증: 스크립트 `/tmp/postudio-chat-alignment-final.js`; 로그 `/tmp/postudio-chat-alignment-5174-verified.log`, `/tmp/postudio-chat-alignment-5190-verified.log`, 각 13 PASS, 0 FAIL.
- 이번 체크포인트: 전체 프런트엔드 테스트 133개 통과, `npm run check` 오류·경고 0개, `npm run build` 성공. 스크립트·검증 로그·화면 캡처는 세션 자산 `postudio-structure-20261113`에 보관했다.
