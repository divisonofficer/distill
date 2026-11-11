# Postudio — 업스트림 Distill 패치 목록

Postudio 코드는 `crates/custom/`, `apps/postudio/`에 격리한다. 업스트림 파일을 고치는 경우만 여기에 기록한다.
rebase할 때 이 목록만 확인하면 되도록 한다.

| # | 목적 | 파일 | 하위 호환 | 테스트 |
|---|---|---|---|---|
| 1 | 세션별 권한 모드 전환 (S10) | `distill-shell/src/agent/mvp_agent/acp_agent.rs`, `distill-shell/src/leader/server.rs` | ✅ `sessionId` 없으면 기존 동작 | `mvp_agent/tests.rs` 3개, `leader/server_tests.rs` 1개 |
| 2 | 세션별 모드 변경을 그 세션 구독자 전체에 알림 (L4) | `distill-shell/src/agent/mvp_agent/acp_agent.rs`, `distill-pager/src/app/acp_handler/mod.rs`, `distill-pager/src/app/acp_handler/settings.rs` | ✅ `sessionId` 없으면 알림 없음 (기존 동작) | `mvp_agent/tests.rs` 2개, `acp_handler/tests/settings.rs` 1개, `postudio-probe` `l4_session_scoped_mode_change` |

---

## #1 `x.ai/yolo_mode_changed`에 `sessionId` 범위 추가

**배경**: 이 알림은 `origin_client.product`가 `clientIdentifier`와 같은 모든 세션에 적용된다(`clientIdentifier`가 없으면 전 세션). Postudio node는 ACP 연결 하나로 여러 세션을 다중화하므로, 한 세션만 수동↔자동으로 바꿀 방법이 없었다.

**변경**:
- `acp_agent.rs` `ext_notification`: params의 선택 필드 `sessionId`를 읽어, 있으면 YOLO 분기와 auto 분기 모두 그 세션에만 적용한다. 기존 `clientIdentifier` 필터는 그대로 함께 적용된다. 로그에 `session` 필드 추가.
- `leader/server.rs` `extract_yolo_mode_change`, `extract_auto_mode_change`: `sessionId`가 있으면 `None`을 반환한다. 세션 범위 변경이 leader의 연결 단위 기본값(`ClientCapabilities`)을 덮어쓰지 않게 하기 위함이다.

**와이어 예시** (Postudio node → agent):
```json
{"jsonrpc":"2.0","method":"x.ai/yolo_mode_changed",
 "params":{"permission_mode":"auto","sessionId":"<uuid>"}}
```

**테스트**:
- `yolo_mode_changed_with_session_id_updates_only_that_session`
- `auto_mode_changed_with_session_id_updates_only_that_session` (auto 켜기 → ask로 끄기 왕복)
- `yolo_mode_changed_without_session_id_keeps_client_wide_scope` (하위 호환)
- `extract_mode_changes_ignore_session_scoped_notifications` (leader)

**패치 없는 코드에서의 결과** (2026-11-11, `acp_agent.rs`와 `leader/server.rs`만 `5fcab86e`로 되돌리고 실행):
- 실패 3개: `yolo_mode_changed_with_session_id_updates_only_that_session`, `auto_mode_changed_with_session_id_updates_only_that_session`, `extract_mode_changes_ignore_session_scoped_notifications`
- 통과 1개: `yolo_mode_changed_without_session_id_keeps_client_wide_scope`. 기존 동작을 확인하는 테스트라서 패치와 무관하게 통과하는 것이 맞다.
- 실행 명령: `RUST_MIN_STACK=16777216 cargo test --locked -p distill-shell --lib -- session_id session_scoped`

---

## #2 세션별 모드 변경을 그 세션의 모든 구독자에게 알림

**배경**: 패치 #1은 한 클라이언트가 세션 하나의 모드를 바꾸게 했지만, 같은 세션에 붙은 다른 클라이언트(예: `distill --leader` TUI)는 알림을 받지 못해 표시 모드가 낡았다 (`P0-RESULTS.md` L4).

**변경**:
- `acp_agent.rs` `ext_notification`: 패치 #1의 `sessionId` 범위 분기가 실제로 적용되면 agent→client `x.ai/yolo_mode_changed`를 한 번 보낸다 (`emit_session_mode_changed`). params: `sessionId`, `yolo_mode`(적용 후 세션 상태), 그리고 이 변경이 auto를 설정한 경우에만 `auto_mode`, `permission_mode`. 셸은 세션별 auto 플래그를 갖지 않으므로 auto를 건드리지 않은 변경에는 두 필드를 넣지 않는다. 기존 gateway 경로(`forward_fire_and_forget`)를 쓰므로 leader가 `sessionId`로 구독자에게 라우팅한다.
- `leader/server.rs`: 변경 없음. `extract_yolo_mode_change`/`extract_auto_mode_change`는 `ServerEvent::Message`(클라이언트→leader)에서만 호출되고, agent→client 알림 경로(`session_subscribers` 팬아웃)에는 닿지 않는다. 이미 `sessionId`가 있으면 `None`을 반환한다.
- `distill-pager/src/app/acp_handler/settings.rs` `handle_session_mode_changed` + `mod.rs` dispatch 1줄: 수신한 `x.ai/yolo_mode_changed`가 `sessionId`를 가지면 그 세션(정확히 일치하는 root 세션만)의 `yolo_mode`/`auto_mode`를 맞춘다. `auto_mode`는 `effective_auto`(yolo가 auto를 이김)로 계산한다. 전역 `current_ui.permission_mode`, `default_yolo`, `[ui] permission_mode` 설정 파일은 쓰지 않는다. 알림을 다시 보내지 않으므로 루프가 없다. 같은 값을 다시 적용해도 무해하다. `sessionId`가 없거나 모르는 세션이면 무시한다.

**와이어 예시** (shell → 구독자 클라이언트, leader가 세션 구독자에게 전달):
```json
{"jsonrpc":"2.0","method":"_x.ai/yolo_mode_changed",
 "params":{"sessionId":"<uuid>","yolo_mode":true}}
```

**테스트**:
- `mvp_agent/tests.rs` `session_scoped_mode_change_emits_notification_for_that_session`: 세션 S-a에 yolo, 이어서 auto 변경 시 각각 정확히 1개의 알림이 S-a로 나가고 상태(`yolo_mode`, `auto_mode`, `permission_mode`)가 맞다.
- `mvp_agent/tests.rs` `client_wide_mode_change_emits_no_session_notification`: `sessionId` 없는 변경은 알림을 보내지 않는다.
- `acp_handler/tests/settings.rs` `session_scoped_mode_echo_updates_only_matching_session`: 일치 세션만 바뀌고, yolo가 auto를 이기며, 알 수 없는/`sessionId` 없는 메시지는 무시되고, 송신 채널에 아무것도 나가지 않으며, `current_ui.permission_mode`와 `default_yolo`가 그대로다.
- `postudio-probe` `l4_session_scoped_mode_change`: 클라이언트 B(S1에 attach)가 10초 안에 S1에 대한 `_x.ai/yolo_mode_changed`(`yolo_mode: true`)를 받는다. 래핑된 형태도 인정한다.

**결과** (2026-11-11):
- `RUST_MIN_STACK=16777216 cargo test --locked -p distill-shell --lib -- agent::mvp_agent::tests leader::server`: 523 passed, 0 failed, exit 0 (신규 테스트 2개 포함).
- `cargo test --locked -p distill-pager --lib -- acp_handler::tests::settings acp_handler::tests::plan_mode session_scoped_mode_echo`: 46 passed, 0 failed, exit 0.
- `cargo check --locked -p distill-shell -p distill-pager`: exit 0.
- `cargo build --locked -p distill-pager-bin --bin distill`: exit 0.
- `GROK_BINARY=$CARGO_TARGET_DIR/debug/distill cargo test -p postudio-probe --test probe -- --ignored --nocapture --test-threads=1`: 7 passed, 1 failed (`l2_background_process_survives_disconnect`, 기존 알려진 실패). `l4_session_scoped_mode_change` 통과, B가 `_x.ai/yolo_mode_changed` 수신.
- `cargo fmt -p distill-shell -p distill-pager -p postudio-probe -- --check`: exit 1. 기준 커밋도 이 명령에서 실패한다(수정 전 diff 903개, 수정 후 908개). 이번 변경 줄은 포맷을 맞췄으나, 기존 미포맷 코드를 바꾸지 않으려고 전체 포맷은 적용하지 않았다.

