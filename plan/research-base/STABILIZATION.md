# Postudio 로컬 안정화 우선순위

기준일: 2026-11-12. 기존의 **Mac 로컬 완성 후 실제 서버 배포** 결정을 유지한다. 이 문서는 다음 실행 순서와 합격 조건을 구체화한다. 구현 여부와 검증 여부는 별도로 기록하며 진행 상태의 원본은 `EXECUTION.md` §5다.

## P0 — 현재 변경의 통합 기준 확정

범위: 기존 기능의 회귀 검사와 발견된 결함 수정. 새 기능과 디자인 확장은 제외한다.

- 프론트: `apps/postudio`에서 `node --test tests/*.test.mjs`, `npm run check`, `npm run build`.
- node: `cargo test -p distill-node`, `cargo clippy -p distill-node --all-targets -- -D warnings`, 바이너리 빌드.
- 실제 leader 통합: `GROK_BINARY="$PWD/target/debug/distill" cargo test -p postudio-probe --test node -- --ignored --nocapture --test-threads=1`. 전역 설치된 다른 `distill` 바이너리로 대체하지 않는다. 모델은 테스트용 응답 서버를 사용한다.
- 브라우저: 채팅 목록·기록·초안 보존, 새 작업·기존 기록·모델 메뉴, 설정 진입·스크롤, 잘못된 호스트 거부, 임시 작업 숨김·명시적 펼치기. 데스크톱과 모바일 너비에서 검사한다.
- 기록: 실행 명령, 종료 코드, 바이너리 경로, 실제 검증 범위를 남긴다. 이전 테스트 개수를 최신 결과로 표시하지 않는다.

합격 조건: 위 검사에 실패가 없고 발견한 회귀를 수정한다. 살아 있는 CLI 작업의 승인·중단·재실행은 검사에 사용하지 않는다. 현재 실행 중인 서버와 새로 빌드한 바이너리를 구분한다. 배포·커밋은 포함 파일과 비공개 정보 검사를 거친 별도 체크포인트다.

## P1 — 요청 수명과 재시작 복구

선행: P0 통과. 최우선 구현 대상은 일반채팅의 결과 불명 상태 영속화다.

- 모델 호출 전에 요청 상태를 내구성 있게 저장한다. 저장 실패 시 모델을 호출하지 않는다.
- 정상 종료는 assistant 메시지와 요청 완료 상태를 일관되게 기록한다.
- 시작 시 미완료 요청은 결과 불명으로 복구한다. 자동 재호출하지 않는다.
- 동일 저장 메시지·모델의 완료된 요청은 기존 결과를 반환한다. 결과 불명은 기록 확인과 명시적인 중복 실행 위험 확인 없이 재실행하지 않는다.
- 테스트 경계: 요청 기록 직후 종료, provider 호출 중 종료, 결과 수신 후 저장 실패, node 재시작, 브라우저 종료·재접속.
- CLI와 웹이 동일 leader를 공유할지, 웹 전용 leader를 사용할지 운영 정책을 확정한다. 별도 leader의 같은 기록 ID가 같은 실행 인스턴스라는 의미는 아니다.

합격 조건: 재시작으로 미완료 요청이 조용히 재실행되지 않는다. 저장된 응답을 잃지 않으며, 코딩 세션 실행 위치를 다른 호스트로 자동 이동하지 않는다. 분산 exactly-once를 보장한다고 표시하지 않는다.

## P2 — 실제 두 서버·휴대폰 검증

선행: P1 통과 및 배포 대상 접근 가능. 실제 주소·키·로그인은 Git 외부 로컬 설정에만 둔다.

- 두 서버의 키를 별도 경로로 확인하고 HTTPS 및 인증된 동기화를 설정한다.
- 단방향 단절, 양쪽 오프라인 작성, 동시 형제 분기, 서버 재시작 후 수렴을 검사한다.
- 잘못된 키·토큰·Origin, 리다이렉트와 큰 본문을 거부하는지 확인한다.
- 한 호스트 장애가 다른 코딩 호스트로 실행을 넘기지 않는지 확인한다.
- 실제 휴대폰 알림 수신과 해당 작업 열기를 검사한다.
- 컨테이너 상주, 자동 시작, 로그·상태 확인, 안전한 업데이트·롤백 절차를 기록한다.

합격 조건: 실제 기기 두 곳에서 동일 기록을 재개하고 장애 후 분기 손실 없이 수렴한다. 현재 전체 스냅샷 동기화의 16 MiB 제한을 명시한다.

## P3 — 기록 규모와 사용성

- 작업 기록: 최근 구간 먼저 읽기, 이전 기록은 커서로 추가 로드. 현재 최신 위치 자동 스크롤과 구분한다.
- 작업 목록: 실제 부모 메타데이터로 서브에이전트를 부모 아래에 묶는다. 임시 경로만으로 부모 관계를 추정하지 않는다.
- 일반채팅: 기본은 선형 대화, 분기 제어는 보조 동작. 모델 설정의 재접속·다중 기기 보존 범위를 확정한다.
- 동기화: 증분 교환과 크기 제한 대응.

웹 로그인·모델별 계정 라우팅은 별도 후속 묶음이다. 실험 관리·NAS·논문·LaTeX는 `BACKLOG.md`에 유지한다.

## Local stabilization checkpoint

- Frontend: all 118 tests passed; type checking and production build passed.
- Node: all 144 tests passed serially; Clippy passed with `-D warnings`.
- Real leader with a local scripted model: all nine `postudio-probe` node tests passed, including permission approval, disconnect/reconnect, node task restart with the leader retained, and an actual `Task` child reaching the node roster with its persisted parent ID.
- Fixed two child visibility defects: missing parent metadata at child creation and hidden-subagent filtering before the opt-in roster merge. Default CLI listing still excludes hidden children.
- Node background notification and synchronization tasks now share the node task lifetime instead of retaining an old keeper after shutdown.
- Synthetic history stress covered 10,002 logical items across 201 pages, incremental reads, partial tails, index corruption, rewind, and limits. Seven focused history tests passed; interleaved updates now compact obsolete physical records.
- Browser checks covered desktop/mobile workspace tab retention, independent drafts, scroll retention, failed polling, and switching workspaces. Earlier interaction checks used controlled responses rather than approving existing user jobs.
- Remaining verification: operating-system process crash during an active turn, full leader cold restart, and browser display of a newly spawned real child. The restart probe cancels and recreates the node task in one test process; it does not prove process-crash recovery. Peak memory under large-history migration was not measured. Live servers are not automatically upgraded by this checkpoint.
