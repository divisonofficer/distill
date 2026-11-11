# Postudio — 업스트림 Distill 패치 목록

Postudio 코드는 `crates/custom/`, `apps/postudio/`에 격리한다. 업스트림 파일을 고치는 경우만 여기에 기록한다.
rebase할 때 이 목록만 확인하면 되도록 한다.

| # | 목적 | 파일 | 하위 호환 | 테스트 |
|---|---|---|---|---|
| 1 | 세션별 권한 모드 전환 (S10) | `distill-shell/src/agent/mvp_agent/acp_agent.rs`, `distill-shell/src/leader/server.rs` | ✅ `sessionId` 없으면 기존 동작 | `mvp_agent/tests.rs` 3개, `leader/server_tests.rs` 1개 |

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
