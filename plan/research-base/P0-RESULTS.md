# Postudio P0 스파이크 결과

> 기준 커밋: `b7520313` (작업 트리에 `distill-pager` 미커밋 변경이 있으나 Postudio와 무관, 손대지 않음)
> 검증 위치: mac (macOS). 서버 쪽 항목은 아직 실행하지 못함(아래 "접근 제약").
> 판정: **PASS(실측)** / **PASS(코드)** = 코드로 확인했고 실행 검증은 남음 / **FAIL** / **대기**

## 요약

| # | 항목 | 판정 | 결론 |
|---|---|---|---|
| S1 | Tailscale HTTPS·신원 헤더·CORS | 부분 PASS(실측) | MagicDNS(`<tailnet>.ts.net`) + HTTPS 인증서 활성. `serve`·헤더·CORS는 서버에서 확인 필요 |
| S2 | `agent serve` 3개 OS 동작 | 대기 | 빌드 도구 없음 (cargo 미설치) |
| S3 | ACP 다중화 시제품 | 대기 | 빌드 필요 |
| S4 | `session/load` 기록 재생 | **PASS(코드)** | 재생 지원. `resume_session`/`close_session`도 있음 |
| S5 | Android Web Push | 대기 | — |
| S6 | TensorBoard 프록시 | 대기 | — |
| S7 | Studio 빌드 (Potero 셸 + OWUI 부품) | 대기 | Node v26 있음, 바로 가능 |
| S8 | 세션 목록 | **PASS(코드)** | 저장소를 직접 읽을 필요 없음 → ACP **`session/list`** 사용 |
| S9 | Gemini Code Assist | 대기 | — |
| S10 | 세션별 권한 모드 | **PASS(패치 #1 적용 후, 테스트)** | 원본은 전 세션에 적용 → `sessionId` 범위 추가 (`PATCHES.md` #1) |
| S11 | PWA 대체 node 재연결 | 대기 | — |

---

## S1 — tailnet 실측

`tailscale status --json` (2026-11-11 기준):

| tailnet 이름 | OS | 상태 | 추정 역할 |
|---|---|---|---|
| `mac` | macOS | 본 기기 | mac 랩탑 |
| `gpu-8000` | linux | online, direct (<campus-net>) | GPU 서버 ? |
| `gpu-3090` | linux | online, direct (<campus-net>) | GPU 서버 ? — **호스트명이 Docker 컨테이너 ID 형식** |
| `win-a` | windows | online | Windows PC ? |
| `win-b` | windows | offline 1일 | Windows PC ? (기기 2대?) |
| `phone` | android | offline 4일 | 폰 |

- `MagicDNSSuffix = <tailnet>.ts.net`, `CertDomains` 존재 → **HTTPS 인증서 활성** → `tailscale serve --https` 사용 가능.
- ACL 태그 없음, Tailscale SSH 미사용(`sshHostKeys` 없음).

**컨테이너 의심 (`gpu-3090`)이 설계에 주는 영향**
- systemd가 없을 가능성 → `install-service`에 **비-systemd 경로** 필요. Orca를 상주시킨 방법을 그대로 따른다.
- 컨테이너 안 tailscaled가 userspace networking 모드라면 **들어오는 연결(`serve`)은 되지만, 그 호스트에서 다른 tailnet 노드로 나가는 연결은 SOCKS5/HTTP 프록시를 거쳐야 함** → node 간 중계 클라이언트에 프록시 설정 지원 필요.

**접근 제약**: 이 Mac에서 두 서버로 SSH 불가(`gpu-8000` 22번 timeout, `gpu-3090` 22번 refused). 서버 항목(S1 나머지, S2, S3, S5, S6)은 서버에서 직접 실행하거나 Tailscale SSH를 켜야 한다.

## S4 — `session/load` 기록 재생: PASS(코드)

- `distill-shell/src/agent/mvp_agent/session_setup.rs` `replay_transcript_gate`: transcript를 재생한 뒤(`no_replay`가 아니면) 라이브 출력 게이트를 열어 **재생이 실시간 이벤트보다 먼저** 도착하도록 보장.
- `resume_session`, `close_session`도 구현됨 (`acp_agent.rs`) → node 재시작 후 재생 없이 재부착하는 경로 후보.

## S8 — 세션 목록: PASS(코드)

- ACP **`session/list`** 구현됨 (`agent/handlers/session.rs::handle_list_sessions`): `cwd` 필터 또는 전체, `SessionInfo` + `_meta`.
- 병합 메타(`session/merge.rs::MergedSession`): `session_id, summary, first_prompt, created/updated/last_active_at, cwd, hostname, model_id, num_messages, branch, repo_name, worktree_label, git_root_dir`.
- 저장 위치: `~/.distill/sessions/<URL 인코딩된 cwd>/`.
- **결정**: node는 Distill 내부 크레이트를 링크해 저장소를 읽지 않고 **ACP `session/list`만 호출**한다. 결합도가 낮아진다.

## S10 — 세션별 권한 모드: FAIL → 업스트림 패치 #1

확인 내용:
1. 런타임 전환 경로 `x.ai/yolo_mode_changed` (`acp_agent.rs` `ext_notification`)는 **`origin_client.product`가 `clientIdentifier`와 같은 모든 상주 세션**에 적용된다(`upload/turn.rs::apply_yolo_mode_to_matching_sessions`, auto 분기도 같은 필터). `clientIdentifier`가 없으면 **전 세션**에 적용된다.
   → node가 모든 세션의 단일 클라이언트이므로 **한 세션만 바꿀 수 없다.**
2. 세션 생성 시점에는 세션별 지정이 가능하다: `session/new`·`session/load`의 `_meta.yoloMode`, `_meta.autoMode`.
3. `session/load`의 meta는 yolo/auto를 **켜기만** 가능하고 끄기는 불가능하다(`session_setup.rs` 재접속 처리).
4. Auto 게이트 기본값은 **true**: `requirements > env GROK_AUTO_PERMISSION_MODE > [auto_mode] enabled > managed > remote > default(true)`.

**결정 — 업스트림 패치 #1** (`distill-shell`만 수정, 미커밋 `distill-pager` 변경과 충돌 없음):
- `x.ai/yolo_mode_changed` params에 선택 필드 **`sessionId`** 추가. 있으면 해당 세션에만 적용하고, 없으면 기존 동작을 유지(하위 호환).
- 대상: `acp_agent.rs`의 yolo·auto 두 분기 + `apply_yolo_mode_to_matching_sessions`에 세션 필터 인자.
- 테스트: ① sessionId 지정 시 다른 세션 불변 ② 미지정 시 기존 동작 ③ auto↔ask 왕복.
- 규모: 약 20~40줄 + 테스트.

### 패치 #1 검증 (2026-11-11)

| 명령 | 결과 |
|---|---|
| `cargo test --locked -p distill-shell --lib -- yolo_mode_changed auto_mode_changed extract_mode_changes yolo_toggle extract_yolo_mode_change extract_auto_mode_change` | **12 passed** (신규 4 + 기존 8) |
| `RUST_MIN_STACK=16777216 cargo test --locked -p distill-shell --lib -- agent::mvp_agent::tests leader::server` | **521 passed** |
| 위 명령을 기본 스택으로 실행 | 무관한 `adopting_attach_waits_for_the_installed_actors_stamp`에서 스택 오버플로(디버그 빌드 테스트 스레드 2MB). 스택을 늘리면 통과 → 환경 문제로 판단 |
| rustfmt (변경 파일) | `acp_agent.rs`, `leader/server.rs`, `leader/server_tests.rs` 차이 0. `mvp_agent/tests.rs`의 추가 구간(1818~1920행) 차이 0 (파일의 다른 차이는 기존 코드) |
| clippy `-p distill-shell` | 실행 불가: 기존 코드 `distill-sampling-types/src/types.rs:1617-1622` `indexing may panic` 5건으로 의존 크레이트 체크 실패 (패치와 무관) |

## WP1 골격 — `crates/custom/distill-node`

- 단독 바이너리 `postudio-node` (`distill node` 서브커맨드 통합은 `distill-pager` CLI를 고쳐야 하는데, 현재 그 크레이트에 미커밋 작업이 있어 보류)
- `GET /healthz`, `GET /api/node`, `/api` 소유자 검사(`Tailscale-User-Login`), loopback 바인딩, `--owner` 필수
- 검증: `cargo test -p distill-node` **3 passed**, `cargo clippy -p distill-node --all-targets -- -D warnings` 통과, rustfmt 통과
- 실행 확인(이 Mac): `/healthz` 200, 헤더 없는 `/api/node` 403, 소유자 헤더 있는 `/api/node` 200 + 호스트 정보
- 워크스페이스: 루트 `Cargo.toml` members에 추가, `Cargo.lock`은 `distill-node` 항목만 추가(+17줄)

## 빌드 환경

| 항목 | 상태 |
|---|---|
| Rust | rustup 설치되어 있음(`~/.cargo/bin`, 1.94.0). 비로그인 셸 PATH에만 없었음 → `export PATH="$HOME/.cargo/bin:$PATH"` |
| Node | v26.9.0 (S7 가능) |
| 워크스페이스 등록 | 루트 `Cargo.toml`은 "auto-generated" 표기이지만 생성기가 레포에 없음 → `members`에 직접 추가 |
| Orca | `/Applications/Orca.app` 설치됨, 두 서버에서도 상주 중(사용자 보고) |

## 다음 단계

1. S2(macOS) + S3: 이 Mac에서 `distill agent serve` 띄우고 ACP 다중화 시제품(WP2 시작)
2. S7: `apps/postudio` 골격 빌드
3. 서버 항목(두 컨테이너, nohup): Tailscale SSH 활성화 또는 서버에서 직접 스크립트 실행. 컨테이너 tailscaled 모드 확인
