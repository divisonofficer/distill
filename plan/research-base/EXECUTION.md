# Postudio — 재검토 결과와 멀티 에이전트 실행 계획 (v0.5)

> 기준: `PLAN.md` v0.4, 기준 커밋 `40d7c6bc`(패치 #1), `9c16284d`(node 골격).
> **2026-11-11 결정: R1~R12 전부 수용.** R1은 실행 검증으로 확정(`P0-RESULTS.md`). 트랙 간 경계는 [`CONTRACT.md`](CONTRACT.md)가 정본이다.
> 이 문서는 (1) 코드를 읽고 찾은 계획 변경 제안, (2) 병렬 에이전트로 오래 돌릴 때의 작업 분할·소유권·검증 규칙을 정한다.
> "코드 확인"은 소스를 읽은 것이고 실행해 본 것이 아니다. 실행 검증은 §3의 0단계에서 한다.
> 실제 호스트 이름·tailnet 이름·로그인은 이 문서에 쓰지 않는다(§2).

---

## 1. 계획 변경 (수용됨)

| # | v0.4 계획 | 코드에서 확인한 사실 | 제안 |
|---|---|---|---|
| R1 | node가 ACP 다중화기를 직접 만든다 (§4.3, WP2) | Distill의 **leader 프로세스**가 이미 같은 일을 한다(`distill-shell/src/leader/server.rs`): 클라이언트별 요청 id 분리, 세션별 구독자 방송, 승인 요청을 구독자 전원에게 보내고 먼저 온 응답 채택, 늦게 붙은 클라이언트에 대기 중 승인 재전송, `session/load` 재생은 요청한 클라이언트에게만 전달. 테스트도 있다(`two_clients_one_session_broadcast_and_driver`, `interaction_request_broadcasts_to_all_subscribers`, `pending_interaction_replayed_to_late_joiner`). Unix 소켓과 Windows named pipe를 모두 지원한다 | **leader를 허브로 쓴다.** node는 브라우저 연결마다 leader 클라이언트 연결을 하나 열고 ACP↔SEP 변환만 한다. 다중화기는 만들지 않는다. 0단계 검증(L1~L6)이 실패하면 v0.4 방식(`agent serve` + 자체 다중화)으로 돌아간다 |
| R2 | mac TUI가 `--remote`로 node에 붙는다 (§3, WP9, 완료 기준 5) | TUI에는 원격 접속 옵션이 없다. `--remote`는 `agent serve`의 인자이고 `distill-pager-bin`에서 읽지 않는다. TUI는 `[cli] use_leader = true` 또는 `--leader`일 때 **같은 호스트의** leader에 붙는다(기본값은 꺼짐) | 완료 기준 5를 "세션이 있는 호스트에서 `distill --leader` TUI + 다른 기기 브라우저 동시 접속"으로 바꾼다. 다른 호스트의 TUI는 pager에 새 전송 계층이 필요하므로 P2로 미룬다. WP9 삭제 |
| R3 | 허브 node가 다른 호스트를 중계한다 (`/h/:host/*`, `/api/fleet/sessions`, fleet 토큰, WP5) | 폰은 이미 tailnet 안에 있고 계획에 fleet 간 CORS가 들어 있다. `tailscale serve`는 같은 사용자의 기기에서 온 요청에 그 사용자의 로그인 헤더를 붙인다 | **브라우저가 각 호스트의 node에 직접 연결한다.** 중계·집계 엔드포인트와 fleet 토큰을 없앤다. 컨테이너의 userspace Tailscale 문제도 사라진다(들어오는 연결만 쓴다). 완료 기준 7은 "꺼진 호스트만 오프라인으로 표시되고 나머지는 정상"이 된다 |
| R4 | node → node 인증에 "출발지가 tailnet 대역인지" 확인 (§7) | `tailscale serve` 뒤에서는 출발지가 항상 localhost라 이 확인은 동작하지 않는다 | 삭제. 인증은 로그인 헤더 하나로 한다 |
| R5 | `/api`는 로그인 헤더만 확인 (현재 골격) | 헤더는 `tailscale serve`가 붙인다. 폰 브라우저에서 연 **다른 웹사이트**가 node 주소로 요청을 보내도 헤더가 붙는다. WebSocket에는 CORS가 적용되지 않는다 | 모든 `/api`, `/ws` 요청에서 **`Origin` 허용 목록**을 확인한다(fleet 호스트의 Studio 출처만). 상태를 바꾸는 요청은 `Content-Type: application/json`만 받는다. WP1에 포함, 테스트 필수 |
| R6 | node는 `127.0.0.1:7410`에 바인딩 (§4.2) | loopback TCP는 같은 기기의 **다른 OS 사용자**도 접속할 수 있고, 헤더는 직접 써넣을 수 있다. `tailscale serve`는 Unix 소켓 대상을 지원한다(Linux/macOS) | Linux/macOS는 `~/.distill/postudio/node.sock`(디렉터리 0700)에 바인딩하고 `tailscale serve unix:…`로 노출한다. Windows는 loopback TCP를 유지한다 |
| R7 | `/p/:port/*`와 `/api/files`가 Studio와 같은 출처 (§4.2, §6.4) | 프록시한 앱이나 에이전트가 만든 HTML의 스크립트가 Studio API를 그대로 호출할 수 있다. iframe sandbox는 주소를 직접 열면 적용되지 않는다 | 신뢰하지 않는 콘텐츠는 **별도 출처**로 낸다: node가 두 번째 리스너를 열고 `tailscale serve --https=8443`으로 노출한다. `/p/:port/*`와 산출물 파일은 그쪽에서만 서빙하고, API는 그 출처의 요청을 거부한다. `--path_prefix` 요구는 그대로 |
| R8 | 새 작업은 node가 `distill-fast-worktree`로 worktree를 만든다 (§4.2) | ACP 확장 `x.ai/git/worktree/create`가 이미 있다(`extensions/worktree.rs`) | 이 호출을 쓴다. node는 Distill 내부 크레이트에 의존하지 않는다 |
| R9 | 세션 보드는 `session/list`로 만든다 (S8) | 대시보드용 `x.ai/sessions/list`와 변경 알림 `x.ai/sessions/changed`가 있다. 항목에 상태(`working`/`idle`/`needs_input`/`dormant`/…)가 들어 있고 알림은 모든 클라이언트에 방송된다 | 보드와 **승인 푸시 트리거**를 이 알림으로 만든다. node는 세션마다 구독하지 않아도 `needs_input` 전환을 알 수 있다 |
| R10 | OWUI에서 Markdown 렌더러·메시지·입력창 이식 (§6.1, S7) | 이 Mac에 Potero·OpenWebUI 소스가 없다. OpenWebUI는 최근 버전에서 브랜딩 조항이 붙은 라이선스로 바뀌었다(이식 전 확인 필요). 이 레포는 Apache-2.0이다 | MVP는 `marked` + `DOMPurify` + 코드 하이라이터로 직접 만든다. OWUI 이식은 하지 않는다. S7은 "SvelteKit 정적 빌드 + 렌더 확인"으로 줄인다 |
| R11 | `init`이 1회용 코드를 내고 `join <code>` (§4.1) | R3·R4로 fleet 토큰이 없어졌다. 공유할 것은 Web Push용 VAPID 키뿐이다 | `distill node join <호스트>`: 기존 node에서 HTTPS로 설정 묶음을 받는다(로그인 헤더로 인증). userspace Tailscale이면 이때만 `ALL_PROXY`가 필요하다 |
| R12 | Studio 정적 파일을 바이너리에 내장 (§4.2) | 워크스페이스에 임베드용 크레이트가 없고 `tower-http`는 있다 | 개발 중에는 `--web-dir`로 디스크에서 서빙한다. 내장은 배포 단계(3단계)에서 결정한다 |

R1이 가장 큰 변경이다. 다음 작업으로 잡혀 있던 "WP2 ACP 다중화기"를 시작하기 전에 0단계 L1~L6을 먼저 돌린다.

### R1에서 아직 모르는 것 (0단계에서 실행해 확인)

| # | 확인할 것 | 통과 조건 | 실패 시 |
|---|---|---|---|
| L1 | `distill agent stdio --leader` 두 개가 같은 세션에 붙는다 | 둘 다 스트림을 받고, 한쪽 승인이 반영되며, 늦게 붙은 쪽이 대기 중 승인을 받는다 | v0.4 방식으로 복귀 |
| L2 | 마지막 클라이언트가 끊기면 유휴 세션은 디스크로 내려간다(`handle_evict_sessions`). 그 세션이 띄운 백그라운드 프로세스(TensorBoard)가 살아남는가 | 프로세스 유지 | node가 상시 연결 하나로 해당 세션을 구독해 둔다 |
| L3 | `distill agent leader --no-exit-on-disconnect`가 클라이언트 0개에서도 유지된다 | node 재시작 후 세션·작업 유지(완료 기준 8) | leader 수명 관리 방식 재설계 |
| L4 | 패치 #1을 leader 경유로 보냈을 때 그 세션만 바뀌고, 같은 세션의 TUI 모드 표시가 바뀐다 | 완료 기준 9 | 모드 변경 알림 패치 #2 범위 산정 |
| L5 | Windows에서 leader(named pipe) 동작 | 세션 생성·스트림 | Windows만 `agent serve` 경로 |
| L6 | 샌드박스 프로필을 요청하면 leader가 꺼진다 | node 실행 경로가 프로필을 요청하지 않음을 확인 | — |

---

## 2. 비공개 정보 규칙 (적용 완료)

| 대상 | 규칙 |
|---|---|
| 실제 기기 이름, tailnet 이름, tailnet·학내 IP, Tailscale 로그인, 홈 디렉터리 경로 | 커밋 금지. 문서·코드·테스트·fixture에는 별칭(`gpu-3090`, `gpu-8000`, `win-a`, `win-b`, `mac`, `phone`, `<tailnet>`)과 `me@example.com`만 쓴다 |
| 별칭 ↔ 실제 값 | `plan/research-base/LOCAL.md` (`.gitignore`에 등록됨) |
| 실행 시 설정(소유자 로그인, VAPID 키, 푸시 구독) | `~/.distill/postudio/` 아래에만 둔다. 레포 안에 두지 않는다 |
| 서버 검증 출력, 캡처 원본 | `.postudio/` (`.gitignore`에 등록됨). 커밋할 fixture는 `/tmp` 아래 임시 경로에서 캡처하고 치환 후 넣는다 |
| 강제 수단 | 공용 git 디렉터리의 `hooks/pre-commit`이 스테이징된 추가 줄을 `postudio-private-patterns.txt`와 대조해 커밋을 막는다. 훅과 패턴 파일은 모든 worktree가 공유하고 레포에는 들어가지 않는다 |

패턴 파일은 `tailscale status --json`의 기기 이름·DNS 이름·IP·로그인과 홈 경로로 만든다. 기기를 추가하면 다시 만든다. 훅은 `git commit --no-verify`로 우회되므로 에이전트는 이 옵션을 쓰지 않는다. 훅은 이 Mac에만 있다. 다른 기기에서 커밋하려면 그 기기에도 설치해야 한다.

---

## 3. 실행 단계

### 0단계 — 직렬, 주 모델이 직접 (병렬 작업의 전제)

| 작업 | 내용 | 산출물 |
|---|---|---|
| T0.1 | 기준 커밋: 패치 #1, node 골격, 계획 문서를 각각 커밋한다. worktree는 HEAD에서 갈라지므로 미커밋 작업은 다른 에이전트에게 보이지 않는다 | 커밋 3개 |
| T0.2 | 패치 #1 보강: 새 테스트 4개가 패치를 뺀 코드에서 **실패하는지** 확인한다 | 결과를 `PATCHES.md`에 기록 |
| T0.3 | 허브 검증 L1~L4, L6 (이 Mac). L5는 Windows 접근 후 | `P0-RESULTS.md` 갱신, R1 확정 또는 철회 |
| T0.4 | ACP 캡처: 실제 에이전트로 시나리오 5개(텍스트 응답, 툴 실행, 승인 요청·응답, 이미지 출력, 오류)를 줄 단위 JSON으로 기록 | `crates/custom/distill-node/tests/fixtures/acp/*.jsonl` |
| T0.5 | 계약 고정: SEP 타입(`src/sep.rs`), 캡처별 기대 SEP 출력(`tests/fixtures/sep/*.json`), HTTP·WS 경로 표, 푸시 페이로드 형태 | 이후 변경은 주 모델만 한다 |

T0.4는 비용이 들지 않는다. 레포의 테스트용 가짜 모델 서버(`distill-test-support`의 `MockInferenceServer`)가 툴 호출과 오류를 대본대로 돌려주므로 실제 모델 계정을 쓰지 않는다.

### 1단계 — 병렬 4트랙 (+E)

각 트랙은 자기 worktree(브랜치 `postudio/<트랙>`)에서 `CONTRACT.md`의 해당 절만 구현한다. `src/lib.rs` 라우터 조립, `Cargo.toml`, `src/sep.rs`는 주 모델만 고친다. 모듈 파일은 이미 빈 채로 커밋돼 있다.

| 트랙 | 범위 (`CONTRACT.md`) | 소유 파일 | 검증 명령 |
|---|---|---|---|
| A node-core | §3 변환, §4 세션 WS·보드·REST, keeper | `src/{reduce,agent,session,board}.rs`, `tests/reduce.rs`, `tests/fixtures/sep/*.json` | `cargo test -p distill-node` (fixture 7개 일치), 실제 leader 대상 통합 테스트는 `postudio-probe`에 추가 |
| B node-edge | §5 guard·리스너·비신뢰 출처 | `src/{guard,listen,untrusted}.rs`, `tests/edge.rs` | `cargo test -p distill-node`; §5 시험 목록 전부 |
| C web | Studio 앱 | `apps/postudio/**` | `npm run check && npm run build`; §7 dev-stack에 붙여 브라우저 390×844, 1280×800 |
| D push | §6 | `src/push.rs`(필요하면 `src/push/`), `tests/push.rs` | `cargo test -p distill-node push` (RFC 8291 벡터) |
| E 업스트림 | 패치 #2: 세션 단위 모드 변경을 그 세션 구독자에게 알림(L4 관찰) | `distill-shell` 해당 파일, `PATCHES.md` | `RUST_MIN_STACK=16777216 cargo test -p distill-shell --lib -- agent::mvp_agent::tests leader::server`, `postudio-probe` l4 확장 |

트랙 A·B·D는 `CARGO_TARGET_DIR=target-node`를 쓴다. 트랙 E와 `postudio-probe` 실행만 `target/`(전체 빌드)를 쓴다. **`distill-node`에 Distill 내부 크레이트 의존을 추가하지 않는다**(`distill-version` 제외). 트랙 C는 A가 끝나기 전에는 `tests/fixtures/sep/*.json`으로 화면을 만든다.

### 2단계 — 통합 (트랙 E 완료 후)

| 작업 | 내용 | 선행 |
|---|---|---|
| I1 | 실제 leader 연결로 세션 보기·프롬프트·중단·승인 | A, E |
| I2 | 새 작업: `x.ai/git/worktree/create` → `session/new`(`_meta`로 초기 모드) | I1 |
| I3 | 모드 칩: 패치 #1 호출, 계획 모드는 `session/set_mode` | I1 |
| I4 | 호스트 목록(`tailscale status --json`), 브라우저 직접 연결, 오프라인 표시, `join` | B, C |
| I5 | 운영: `postudio-node up`(nohup, pidfile, 멱등, leader 감시·재시작), launchd, Windows 작업 스케줄러 | A |
| I6 | 푸시 종단: 구독 → 승인 대기 → 알림 → 탭하면 해당 세션 | C, D, I1 |

### 3단계 — 실제 호스트 검증 (보류)

**2026-11-11 결정: 먼저 이 Mac에서 끝까지 완성한다.** 서버 접근은 그 뒤에 정한다. 그때 `PLAN.md` §9 완료 기준을 호스트에서 실행해 기록하고, S1(위조 헤더가 `tailscale serve`에서 제거되는지 포함), S5, S6, L5를 확인한다.

로컬 완성의 뜻: 이 Mac 한 대에서 leader + node + Studio가 동작하고, 같은 Mac의 브라우저(390×844, 1280×800)와 TUI로 완료 기준 2, 5, 6, 8, 9를 통과한다. 완료 기준 1, 3, 4, 7(폰, 푸시 수신, 여러 호스트)은 3단계로 넘긴다. 푸시는 발송 요청이 만들어지는 것까지 테스트로 확인한다.

---

## 4. 에이전트 운영 규칙

| 항목 | 규칙 |
|---|---|
| 역할 | 주 모델: 계약, 보안 판단, 통합, diff 검토. 작업 모델: 명세가 확정된 트랙 구현과 검증 실행 |
| 격리 | 트랙마다 worktree 하나, 브랜치 `postudio/<트랙>`. 병합은 주 모델이 한다 |
| 소유권 | 표에 없는 파일을 고쳐야 하면 고치지 말고 보고한다 |
| 스테이징 | `git add <경로>`만 쓴다. `git add -A`, `--no-verify` 금지. 작업 트리에 누구 것도 아닌 `.snap.new` 파일 11개가 있다 |
| 완료 보고 | 바뀐 파일 목록, 검증 명령과 종료 코드, 실패·미검증 항목. 실행하지 않은 것을 통과로 쓰지 않는다 |
| 검토 | 트랙 B·D·E와 I3·I4는 병합 전에 별도 코드 검토를 거친다(인증, 암호, 권한 모드) |
| 화면 작업 | 트랙 C와 I1~I6의 화면 변경은 브라우저에서 직접 눌러 확인한다. 빌드 성공만으로 완료 처리하지 않는다 |
| lint | `cargo clippy -p distill-node`는 통과해야 한다. `distill-shell`은 `distill-sampling-types`의 기존 오류 5건 때문에 clippy가 멈추므로 `cargo check`로 대신한다 |
| 중단 조건 | 같은 작업이 두 번 실패하면 더 쪼개거나 주 모델이 맡는다. 계약 변경이 필요하면 해당 트랙을 멈추고 주 모델이 T0.5를 고친다 |
| 진행 기록 | 이 문서 §5 표만 갱신한다 |

---

## 5. 진행 상태

### 2026-11-12 계정 설정 후속 요청

사용자의 추가 요청에 따라 로컬 안정화 다음에 계정 웹 로그인·Auto 라우팅 설정을 연결했다. node 전체 125개, 웹 전체 52개 테스트 통과. node Clippy, 웹 타입 검사와 빌드 통과. 공급자 인증 모듈 38개 테스트와 login-web CLI 테스트를 실행했다. node만 재실행하여 실제 기존 라우팅 조회를 확인했으며 기존 계정·라우팅 값은 변경하지 않았다.

Orca에서는 별도 브라우저 fetch fixture로 로그인 시작·인증 링크·취소·완료 표시, 라우팅 옵션 추가·모델 선택·Worker 설정·순서 변경·저장 실패 후 초안 유지·저장 성공을 확인했다. 모바일 너비도 확인했다. 실제 로그인 승인 및 계정 변경은 실행하지 않았다. 로그인은 동일 컴퓨터의 ChatGPT·Claude·OpenRouter만 지원하고, 원격 브라우저와 Grok은 CLI 경로를 유지한다. 인증 링크는 자동으로 열지 않는다.

### 2026-11-12 로컬 안정화 체크포인트

실행 순서와 합격 조건은 [STABILIZATION.md](STABILIZATION.md)에 구체화했다. 아래 수치는 계정 웹 로그인·라우팅 추가 작업을 시작하기 전 체크포인트이며 이후 변경의 검증을 대신하지 않는다.

- node 전체 테스트 112개 통과, `cargo clippy -p distill-node --all-targets -- -D warnings` 및 바이너리 빌드 통과.
- 웹 전체 테스트 47개, 타입 검사, 빌드 통과. 기존 `w-72` 고정 검사를 현재 사이드바 내부·외부·이동 거리의 일치 검사로 수정했다.
- 실제 leader + mock 모델 통합 테스트 7개 통과: 승인, 두 클라이언트, keeper 유지, 인증 거부, 생성 프롬프트, 재접속 권한 모드, worktree 생성.
- 첫 통합 실행은 전역 설치 바이너리를 사용해 권한 모드 검사가 실패했다. `GROK_BINARY="$PWD/target/debug/distill"`로 프로젝트 바이너리를 지정한 재실행은 7개 모두 통과했다.
- Orca에서 현재 개발 서버의 채팅 기록, 새 작업, 기존 작업 링크, 임시 작업 숨김, 설정 진입·스크롤을 확인했다. 살아 있는 작업을 승인하거나 모델 요청을 보내지 않았다.
- 실제 서버 동기화·휴대폰 푸시·재시작 결과 불명 상태 영속화는 이 체크포인트에서 검증하지 않았다. 실행 중 node/leader는 변경하지 않았으며 새 빌드 배포와 변경 묶음 커밋은 별도다.


| 작업 | 구현 상태 | 자동 테스트 | 브라우저 확인 | 비고 |
|---|---|---|---|---|
| T0.1–T0.5, 패치 #1, 캡처·계약 | 완료(역사 기록) | 과거 결과 유지: §3·`P0-RESULTS.md`·`PATCHES.md` | 해당 없음 | 기존 검증 수치를 새로 실행한 것으로 보지 않음 |
| node 기본·A1 변환 | 구현됨 | 과거: 3개 골격 테스트, fixture 7개 일치 | 미기재 | R5/R6 등 경계 범위는 아래 진행 중 작업과 구분 |
| 세션·keeper·보드·push 트리거 | 구현됨 | node 112개 검사에 keeper 시작·복구·대기 상태 검사가 포함됨 | 임시 node에서 세션 생성·승인·재생·호스트 전환 확인 | 실제 push 수신 및 node 재시작 후 학습 프로세스 생존은 미검증 |
| 모델·계정 기능 | 구현됨 | 과거 로컬 검사 기록 | 과거 Orca 확인 기록(UI-CAPABILITIES.md) | 새로 검증한 것으로 간주하지 않음 |
| 코딩 세션 대화·승인·질문·계획·권한 모드 | 구현됨 | 과거: node 59개, leader 통합 6개, 프런트 7개 통과 | 과거 Orca 확인 | 기록은 당시 결과이며 현재 dirty 변경 검증 아님 |
| 일반 대화·첨부·동기화 관련 변경 | 패치 진행 중; 검증 대기 | 미확인 | 미확인 | 현재 작업 트리에 변경이 있으나 완료·지속성/다중 기기 검증으로 표시하지 않음 |
| Keeper retention, proxy 제한, worktree, artifacts/ports, fleet | 구현·로컬 검증 | node 112개, 웹 49개 통과; 실제 leader worktree 회귀 1개 통과 | 아래 2026-11-12 검증 기록 참조 | 실제 폰·다중 OS·Tailscale 배포와 학습 프로세스의 node 재시작 생존은 별도 검증 필요 |
| Operations CLI (`up`/`join`/서비스 자동화) | 미구현·미완료 | 미확인 | 미확인 | 현재 CLI는 `serve`만 제공. §2단계 I5 및 join 자동화는 outstanding |
| 1단계 B 경계 보안 | proxy·출처 보완 구현 및 로컬 검증 | 전체 node 검사에서 경계·CORS·WebSocket subprotocol·hop-by-hop 회귀 통과 | 별도 출처 HTML 및 HTTP 포트 프록시 확인 | 실제 Tailscale 배포와 `HOME` 오류 경로의 추가 검토는 별도 |
| 1단계 D Web Push | 구현됨 | 과거 RFC 8291 벡터·7개 통과 | 푸시 수신 미확인 | 구독/발송 코드 및 keeper trigger 존재; 실제 브라우저 수신은 미확인 |
| 1단계 C 웹 앱 | 디자인 및 로컬 backend 연동 구현 | 최신 check/build 및 프런트엔드 49개 통과 | 실제 leader·mock 모델의 데스크톱/모바일 뷰포트 조작 확인 | 서비스워커 장애 복구·실제 푸시 수신은 미완료 |
| 1단계 E 패치 #2 | 상태 재확인 필요 | 기록 없음 | 미확인 | 이전 표의 결정 대기 기록은 역사적; 현 source/검증 결과 확인 후 갱신 |

### 2026-11-12 리뷰 보완 및 제공 프로토타입 검증

- `CARGO_TARGET_DIR=target-node cargo test -p distill-node`: 112개 통과. `cargo clippy -p distill-node`도 통과했다. `apps/postudio`에서 `npm run check`, `node --test tests/*.test.mjs`(49개), `npm run build`가 모두 통과했다.
- 실제 프로젝트 바이너리와 mock 모델을 사용하는 worktree 통합 회귀 1개가 통과했다. 실제 실행에서 발견한 ACP 확장 메서드의 `_` 접두사 누락과 macOS `/var`·`/private/var` 경로 차이를 수정했다. 실패 시 원본 경로에 세션을 만드는 대체 동작은 없다.
- 별도 임시 환경의 두 node와 Playwright 브라우저로 작업 생성·편집 승인·재생, 원격 호스트 worktree 생성, 보드와 다른 호스트에 고정된 세션의 권한 변경을 확인했다. 호스트 중단 후 다른 호스트로 보드가 재연결되며, 작성 중인 새 작업은 잘못된 호스트에 생성되지 않고 명시적 재선택을 요구했다.
- PNG 로드, 별도 출처의 HTML iframe 내부 버튼 동작(`sandbox="allow-scripts"`), 하위 폴더·상위 폴더 이동, 포트 목록·HTTP 프록시 열기, 잘못된 호스트 오류 화면을 확인했다. 실제 TensorBoard 서버와 Tailscale 경유 인증·푸시 수신은 이번 검증에 포함하지 않았다.
- 제공된 라이브 프로토타입 ZIP과 두 번째 기준 이미지에 맞춰 실제 일반 대화 템플릿과 사이드바를 다시 구성했다. 모델 선택은 상단으로 옮겼고, 저장·진단·분기 조작은 메뉴로 정리했다. 1280×800과 390×844 실제 뷰포트에서 모델 응답, 저장·새로고침 복구, 메뉴, 모바일 드로어·Escape, 사이드바 접기·복원을 확인했고 가로 넘침은 없었다. 연구 도구는 미구현 상태를 명시하고 비활성화했다.
- 위 브라우저 검증은 localhost의 임시 node와 mock 모델에 한정된다. 실제 휴대폰, 다른 OS, 서버 간 대화 복제, 운영 자동화 및 node 재시작 후 실제 학습 프로세스 생존을 완료로 간주하지 않는다.

## 6. 결정 기록과 남은 질문

| 항목 | 결정 (2026-11-11) |
|---|---|
| R1~R12 | 전부 수용 |
| 기준 커밋 | 진행 |
| 서버 검증 | 보류. 로컬 완성 먼저 |
| Windows PC | 개인 Windows 11 PC. `win-a`·`win-b` 중 어느 쪽인지는 3단계에서 정한다 |
| GPU 서버의 다른 OS 사용자 | 있음. 그래서 Docker 컨테이너로 분리해 쓴다 → R6(Unix 소켓) 유지. 컨테이너가 호스트 네트워크를 쓰면 loopback이 다른 사용자에게 열리기 때문이다 |

남은 질문: 없음. T0.4 캡처는 가짜 모델 서버로 하므로 모델·계정 결정이 필요 없다.

## 7. 인터랙티브 하네스 UI 보완 작업

점검에서 확인한 결함을 먼저 고친다. 외형 변경은 유지하며, 새 디자인이나 의존성 추가는 범위에서 제외한다.

- **대화 변환 worker:** `reduce.rs`, 변환 테스트와 기대 출력만 소유한다. 라이브 사용자 메시지와 재생 툴 콘텐츠를 복구한다.
- **웹 상태 worker:** `session.svelte.ts`, `api.ts`, 새 작업 화면과 회귀 테스트를 소유한다. 턴 경계, 연결 해제, 미지원 worktree 기본 선택을 수정한다. 대화 변환과 병렬 실행한다.
- **프로토콜 조사 worker:** 질문·계획 승인과 권한 모드의 실제 ACP 형식을 확인한다. 구현 worker에 소스 근거를 전달한다.
- **질문·계획 승인 worker:** 앞선 웹 상태 작업이 끝난 뒤 이벤트 타입, 세션 라우팅, 상호작용 카드와 세션 화면을 수정한다. 요청 ID·세션·응답 종류를 검증하고 명시적인 사용자 동작으로만 응답한다.
- **권한 모드 동기화:** 프로토콜 조사와 질문·계획 승인 작업 이후 같은 파일 소유권을 넘겨 구현한다. 추측한 초기 상태 대신 서버의 실제 상태를 사용한다.
- **통합 검토:** 부모 에이전트가 diff와 검사 결과를 검토하고, 독립 리뷰 뒤 Orca 브라우저에서 생성·대화·승인·질문·계획·재연결을 검증한다. 데스크톱과 모바일 크기, 오류·연결 해제 상태를 포함한다.

모델 선택, HTML 결과물 패널, 실제 푸시 수신과 운영 자동화는 이 결함 수정 묶음 다음 작업이다. 미구현 기능을 완료로 표시하지 않는다.

### 보완 결과

- 대화 변환과 웹 상태 작업을 통합했다. 라이브 사용자 텍스트, 재생 툴 콘텐츠, 턴별 메시지 분리, 미지원 worktree 비활성화와 오류 표시를 구현했다.
- 질문 선택·자유 입력·취소 및 계획 승인·수정 요청·중단을 구현했다. 요청 ID/세션/종류를 검증하며, 다른 종류의 응답으로 우회하거나 이미 끝난 요청에 다시 답하는 것을 거부한다. 독립 리뷰에서 발견한 권한 응답 우회, 복구 가능한 오류의 카드 삭제, 질문 키의 객체 프로토타입 충돌을 수정했다.
- 실제 권한 상태를 roster의 `yolo`/`autoMode`로 노출하고 node가 2초 간격으로 갱신한다. 브라우저는 서버 확인 전 상태를 바꾸지 않는다. 실제 ACP의 중첩 `result.sessions` 응답 누락도 수정했다.
- Orca 브라우저에서 실제 leader와 가짜 모델을 사용해 작업 생성, 편집 승인, 질문 답변, 계획 승인, 자동 모드 전환과 새로고침 후 유지까지 확인했다. 390px iframe에서 가로 넘침이 없음을 확인했다. 기기 에뮬레이션이 실제 뷰포트를 바꾸지 않아 모바일 실기기 검증으로 간주하지 않는다.
- node 테스트 59개, 실제 leader 통합 테스트 6개, 프런트 회귀 테스트 7개가 통과했다. 프런트 check/build와 node clippy도 통과했다. 모델은 가짜 서버이므로 모델 호출 비용은 없다.
- 남은 범위: 질문의 계획 모드 전용 추가 동작(chat_about_this/skip_interview), 모델 선택, HTML 결과물 패널, 이미지 첨부의 사용자 메시지 재생, 실제 푸시 수신, worktree 생성. 미구현 상태로 유지한다.
