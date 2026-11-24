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

## 2026-11-16 작업 상태 구조 후속 점검

최신 기준은 사용자가 붙인 HTML 컴포넌트 참조 섹션 1–7이다. 위 기록은 이전 체크포인트 증거로 보존하며 아래 변경의 검증을 대신하지 않는다. 이미 구현된 헤더 제거·도구 묶음·승인 카드·모델 선택/라우팅 분리·실행 개수·알림은 유지한다.

| ID | 참조 위치 | 현재 코드·변경 | 상태·검증 |
|---|---|---|---|
| W-05 | 상단 탭·모바일 탐색 | `WorkbenchPane.svelte`: 탐색·탭 추가를 가로 스크롤 영역 밖에 두고 데스크톱 모바일 메뉴 CSS 우선순위를 수정했다. 단일 그룹 제목은 숨기며 분할 구분은 유지한다. | 모의 API 브라우저 검증 통과: 14개 탭·1402×880·390×844 |
| W-06 | 섹션 4 명령 단계 | `ToolRow.svelte`: 진행·실패 도구 기본 접힘, 사용자 펼침 상태 유지, 인라인 출력 전체 높이 제한, 실제 오류 한 줄 표시. 기존 상세 탭 재사용. 완료 표시 중복 축소, 실제 diff 변경량 유지. | 모의 API 브라우저 검증 통과: 500줄 로그·상태 갱신·상세 탭 |
| W-07 | 섹션 7 압축 여부 | `CompactionChoice.svelte`, `SessionConversation.svelte`: `cold-return-` 요청만 작성기 위 전용 카드로 표시. 서버 원본 선택지로 응답하며 한국어 설명·직접 답변·포커스 범위 내 키보드 조작 제공. 예상 토큰 수를 꾸며내지 않는다. | 세 선택·직접 답변·취소·키보드·일반 질문 회귀를 두 화면 크기에서 검증 |
| W-08 | 압축 실행·결과 | `session.svelte.ts`, `CompactionStatus.svelte`, node `reduce.rs`/`sep.rs`, shell `compaction.rs`: 실제 시작·완료·실패·취소 이벤트 연결. 수동 압축 시작·실패 알림 추가. 연결 단절·확인 불가 상태를 완료로 표시하지 않는다. | 브라우저 모의 이벤트 및 Rust 테스트 통과. 실제 백엔드 재시작·배포 안 함. 압축 도중 새 연결이 시작 상태를 복원할 서버 계약은 아직 없음 |
| W-09 | 하단 라우팅 요약 | `ComposerRouting.svelte`: Auto·단일 설정 경로의 실제 Main/Worker 설정 요약. 여러 경로는 개수만 표시하며 실제 선택 경로로 오인시키지 않는다. | 브라우저 요약·팝업·좁은 화면 검증 통과 |
| W-10 | 섹션 1·2 서브에이전트 | node `executions.rs`, 웹 `executions.svelte.ts`, `ExecutionMonitor.svelte`, `AgentDetailModal.svelte`: 실제 실행 목록·상세·취소 확장을 연결했다. 기본 보기는 대화 탭이 아니라 네이티브 상세 모달이다. 유형·설명·실제 시간·호출 수·결과·실패·취소 상태를 표시한다. | 구현 및 모의 API 브라우저 검증 완료. 부모 범위·오래된 응답·중복 중지는 계약 테스트로 검증한다. 현재 화면에서 관찰한 완료 실행만 유지하며 과거 전체 목록·실제 서버 배포는 미완료 |
| W-11 | 섹션 2 백그라운드 작업 | 실제 TaskSnapshot의 소유 세션·백그라운드 전환 여부로 목록을 걸러 `CommandOutputModal.svelte`에 명령·출력·시각·종료 코드·잘림을 연결한다. 공유 터미널의 다른 세션·포그라운드 작업은 노출하지 않는다. 명령 도구에는 기존 상세 탭과 별도 출력 모달 동작을 제공한다. | 구현됨. 안전한 개별 중지는 새 `x.ai/task/kill_scoped`와 `scopedKillSupported`가 있는 서버에서만 제공한다. 구버전에는 중지 버튼을 표시하지 않는다. 원자적 소유권·백그라운드 여부 확인을 같은 터미널 액터에서 수행하며 실제 임시 프로세스 테스트로 검증했다. 노드 계약 테스트·격리 프리뷰 브라우저 검증 완료. 실행 서버 배포는 하지 않았다 |
| W-12 | 실행 중 지시 | 셸 queue/send-now 기능은 존재한다. 현재 Postudio 전송 계약에 선택·접수 확인이 없다. | 대기/즉시 개입을 구분해 연결해야 함. 문구만 변경하지 않음 |
| W-13 | 실행 모드·계획·컨텍스트 | 승인 정책은 실행 프리셋과 별개다. 컨텍스트는 실제 usage 이벤트만 사용한다. | 실행 프리셋·계획 토글 연결 및 정보 없음 안내 보완 필요 |
| W-14 | 승인 의미·키보드 | 기존 승인 카드 유지. 명령 문자열만으로 안전성을 추정하지 않는다. | 포커스 범위 내 단축키·영구 적용 범위·처리 중/실패 상태 검증 필요 |
| W-15 | 사이드바·시각 규격 | 호스트 이름·프로젝트 관계·실제 작업 상태·계정/사용량 위계를 먼저 정리한다. | 미완료. 비상주를 완료로 추정하지 않음. 앞선 명시적 요구대로 작성기에 cwd를 중복 표시하지 않음 |

### 이번 검증 범위

- 브라우저: `/tmp/postudio-compaction-structure-browser.js`, 24개 통과. 최종 로그: `/tmp/postudio-compaction-browser-final.log`. 모든 API·WebSocket은 검증용 데이터로 대체했다. 실제 유료 요청·승인·중지는 수행하지 않았다. 사용자 5174 서버 재시작 없음.
- 화면 캡처: `/tmp/postudio-compaction-structure-1402.png`, `/tmp/postudio-compaction-structure-390.png`.
- 프런트엔드 테스트 139개 통과, 프로덕션 빌드 통과. 최종 타입 검사는 오류·경고 0개로 통과했다. 중간 검사에서 발견한 별도 진행 중 음성 입력 코드의 타입 오류는 최종 검사에서 해소되어 있었다.
- 노드 전체 직렬 테스트 및 수동 압축 실패 경로 테스트 통과. 실제 실행 서버 배포 완료를 뜻하지 않는다.
- 기존 포트 미리보기·상세 탭·파일 읽기를 재사용한다. 대화형 터미널·리소스 화면 지원이나 전체 목업 일치를 주장하지 않는다.

## 에이전트 상세·명령 출력 모달 체크포인트

- 일반 대화 탭과 실행 상세 모달을 구분했다. 실행 레지스트리의 기본 상세 동작은 `AgentDetailModal.svelte`이며, 기존 부모·자식 대화 목록은 ‘연결된 하위 대화 (참고)’로 분리했다.
- `ExecutionMonitor.svelte`는 실제 실행 목록과 백그라운드 프로세스를 각각 조회한다. `CommandOutputModal.svelte`는 도구 호출이 끝난 뒤에도 살아 있는 명령의 출력·시간·잘림·종료 코드를 갱신한다. 도구 기록만 가진 경우에는 시간·중지를 만들지 않는 읽기 전용 모달이다.
- 개별 중지는 소유 세션과 백그라운드 여부를 원자적으로 확인하는 `x.ai/task/kill_scoped`를 사용한다. 이전 `x.ai/task/kill`로 우회하지 않는다. 지원하지 않는 서버에는 중지 버튼을 표시하지 않고 이유를 안내한다.
- 독립 리뷰에서 발견한 종료 시각 없는 완료 명령의 증가하는 시간 표시와 비활성 범위의 열린 모달 잔류를 수정했다. 완료 시간 정보가 없으면 ‘정보 없음’을 표시하며, 범위를 잃으면 모달을 닫는다.

### 검증

- 프런트엔드 전체 161개 테스트 통과. 타입 검사 오류·경고 0개, 프로덕션 빌드 통과.
- 노드 전체 직렬 테스트 통과. 새 실행 API 테스트 4개가 인증, 다른 부모·작업 차단, 실제 직렬화 형태, 구버전 중지 차단, 잘못된 응답·출력 제한을 포함한다.
- 로컬 터미널의 실제 임시 프로세스로 소유자·백그라운드 조건을 검사하는 테스트 통과. 셸의 새 중지 요청·응답·기능 지원 정보 계약 테스트 통과. 실제 사용자 작업을 종료하지 않았다.
- 최종 브라우저는 별도 프로덕션 프리뷰 `127.0.0.1:5190`에서 18개 통과했다. 데스크톱 1402×880·모바일 390×844, 모달 중앙 배치·포커스·Escape·바깥 클릭, 실행 정보 갱신·완료 시간 고정, 중지 성공·오류·중복 방지, 호스트·세션 이동, 기능 미지원, 기존 상세 탭, 주변 9개 경로를 포함한다. 완료 전환과 에이전트 목록에서 상세 조회로의 전환을 12회 반복했다.
- 모든 브라우저 API·WebSocket은 검증용 데이터로 대체했다. 호스트 전환 검사는 전체 탐색 후 정리를 확인한 것이며, 동일 페이지 내 호스트 변경을 직접 검증한 것은 아니다.
- 개발 서버 5174 검증 중에는 상태 갱신 시간 초과와 연결 거부가 있었다. 시간 초과 원인을 제품 결함으로 확정하지 못했고, 이를 수정됐다고 주장하지 않는다. 진단 정보를 추가해 보존했고, 빌드가 고정된 별도 프리뷰의 최종 반복 검사에서는 재현되지 않았다. 사용자 서버를 재시작하지 않았다.
- 최종 스크립트 `/tmp/postudio-execution-modals-preview.js`, 로그 `/tmp/postudio-execution-modals-preview-final.log`. 캡처 `/tmp/postudio-execution-modals-browser-{agent,command}-{1402,390}.png`. 스크립트·로그·화면은 세션 자산 `postudio-executions-20261116`에도 보관한다.

### 남은 범위

- 노드와 실행 엔진의 새 코드를 실행 서버에 배포하지 않았다. 실제 실행 환경의 표시·중지는 배포 뒤 별도로 확인해야 한다.
- 대화형 셸 입력은 구현하지 않았다. 셸에는 PTY 관련 확장이 있지만 Postudio의 입력·화면 연결은 별도 작업이다. 현재 명령 모달은 실행 조회·출력·개별 중지용이다.
- 완료 에이전트 전체 과거 목록과 모델·추론 수준은 현재 실행 상세 계약에서 보장되지 않는다. 관찰한 완료 실행만 유지하며, 세션 제목·변경 시각에서 실행 사실을 추정하지 않는다. 이 체크포인트를 전체 목업 일치로 간주하지 않는다.

## 실행 API 미지원 화면 회귀 수정

사용자 화면에서 새 실행 API가 없는 서버에 프런트엔드 모니터가 먼저 표시되어 긴 404 오류 두 줄과 안내가 작업 본문을 밀어내는 문제가 확인됐다. 실제 5174 프록시를 통한 에이전트 조회는 본문 없는 HTTP 404였고 `/api/node`는 200이었다. 새 API 코드가 실행 서버에 반영되지 않은 상태와, 미지원 표시의 정보 위계 문제를 분리했다.

- `ApiError`로 HTTP 상태, 요청 경로, 파싱한 오류와 원문을 보존한다. 실행 API의 본문 없는 404만 미지원으로 처리한다. JSON `session not found`, 인증·권한·서버·네트워크 오류는 실제 오류로 남긴다.
- 미지원 자원은 자동 반복 조회를 멈추고 명시적 ‘다시 시도’로 재확인한다. 다른 지원 자원은 계속 갱신한다. 호스트·세션 범위와 오래된 응답 및 중지 차단 규칙을 유지한다.
- 모니터는 짧은 접힌 요약으로 시작한다. 두 API가 없으면 ‘서버 업데이트 후 실행 현황을 사용할 수 있습니다.’를 표시하며, 미지원을 빈 실행 목록으로 표시하지 않는다. 원문 경로·오류는 펼친 기술 정보에만 둔다.
- 실제 새 실행은 자동으로 펼치며, 사용자가 접은 상태는 같은 실행의 반복 갱신에서 유지한다. 상세 모달·출력·중지 기능은 기존대로 유지한다.
- 프런트엔드 전체 165개 테스트 및 타입 검사·프로덕션 빌드 통과. 격리 프리뷰의 데스크톱 1402×880·모바일 390×844 브라우저 10개 검사 통과. 접힌 미지원 모니터 높이는 각각 33px, 53px였다. 반복 조회 중단, 재시도 복구, 일부 지원, 실제 오류 구분, 모달·포커스, 접힘 유지, 범위 이동을 확인했다. 브라우저 API·WebSocket은 모의 데이터이며 실제 변경 요청은 없었다.
- 증거: `/tmp/postudio-execution-compat-parent.js`, `/tmp/postudio-execution-compat-parent.log`, `/tmp/postudio-compat-unsupported-{1402,390}.png`. 세션 자산 `postudio-execution-compat-20261116`에도 보관했다.
- 실행 노드와 사용자 5174 서버는 재시작하지 않았다. 실제 에이전트·백그라운드 실행 기능은 새 노드 및 실행 엔진 배포가 여전히 필요하다. 이 수정은 배포 완료가 아니라 미지원 서버에서의 표시·조회 회귀 수정이다.
