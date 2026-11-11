# Postudio 계약 (T0.5)

> 1단계 트랙이 공유하는 경계. 바꿀 수 있는 사람은 주 모델뿐이다. 트랙이 바꿔야 한다고 판단하면 멈추고 보고한다.
> 근거: `crates/custom/distill-node/tests/fixtures/acp/*.jsonl` (가짜 모델 서버 + 실제 `distill` leader에서 캡처, 2026-11-11).

## 1. 구성

```
브라우저 ──https──> tailscale serve ──unix sock──> postudio-node ──stdin/stdout──> distill agent --leader stdio ──> leader
                                                     (연결마다 자식 1개)                                  (세션 보관)
```

- node는 Distill 내부 크레이트에 의존하지 않는다(`distill-version`만). leader 연결은 **자식 프로세스** `distill agent --leader stdio`로 한다. 실행 파일 경로는 `--distill-bin`(기본 `distill`), 환경 변수는 node 것을 그대로 물려준다.
- 바이너리 이름은 `postudio-node`. `distill node` 서브커맨드는 만들지 않는다. 상주는 `nohup postudio-node up`.
- **keeper 연결**: node는 시작 시 자식 하나를 상시 연다. node가 만들었거나 브라우저가 연 세션은 keeper가 `session/load`로 붙어 있는다. 이유: 마지막 클라이언트가 떨어지면 유휴 세션과 그 백그라운드 프로세스가 내려간다(L2 실패, `P0-RESULTS.md`). keeper는 승인 푸시 트리거도 받는다(§6).

## 2. ACP 쪽 사실 (fixture에서 확인)

- 줄 단위 JSON-RPC. 확장 메서드는 와이어에서 앞에 `_`가 붙는다: `_x.ai/sessions/list`, `_x.ai/yolo_mode_changed`.
- 시작: `initialize` → 응답의 `authMethods`에서 `xai.api_key` 또는 `cached_token` 선택 → `authenticate {methodId, _meta:{headless:true}}`. 구현은 `crates/custom/postudio-probe/src/lib.rs`의 `RawClient::initialize`를 따른다.
- 승인 요청: agent→client 요청 `session/request_permission`, params `{sessionId, options:[{optionId, kind, name}], toolCall:{toolCallId, title, kind, rawInput}}`. 응답 `{"result":{"outcome":{"outcome":"selected","optionId":"allow-once"}}}`. 붙어 있는 모든 클라이언트가 받고 먼저 온 응답이 채택된다. 늦게 붙어도 대기 중 요청을 받는다.
- 세션 목록: `_x.ai/sessions/list {}` → `{sessions:[{sessionId, cwd, title, activity, resident, yolo, modelId, isWorktree, lastChangeUnixMs, origin}]}`. `activity`: `working`·`idle`·`needs_input`·`dormant` 등. 변경 알림 `_x.ai/sessions/changed {upserted:[…], removed:[ids]}`는 모든 클라이언트에 오지만 **`needs_input`은 알림에 안 실린다**(목록 조회에는 나온다).
- 세션별 모드 변경(패치 #1): 알림 `_x.ai/yolo_mode_changed` + `sessionId`. 같은 세션의 다른 클라이언트에는 아무 알림도 가지 않는다(L4 관찰).

## 3. ACP → SEP 변환 (`src/reduce.rs`, 순수 함수)

`Reducer` 하나가 세션 WS 하나를 맡는다. I/O·시계 없음.

```rust
pub struct Reducer { /* 대기 중 요청 id 등 */ }
impl Reducer {
    pub fn new(session_id: String) -> Self;
    /// client→agent 줄. 상태를 바꾸면 이벤트를 낸다(예: prompt 전송 → running).
    pub fn on_client(&mut self, msg: &serde_json::Value) -> Vec<Sep>;
    /// agent→client 줄.
    pub fn on_agent(&mut self, msg: &serde_json::Value) -> Vec<Sep>;
}
```

다른 `sessionId`의 메시지는 버린다. `session/update`의 내용은 `params.update`, `_x.ai/session_notification`은 `params.update`(둘 다 `sessionUpdate`로 구분).
재생에서는 `turn_completed`가 `_x.ai/session/update`로 온다(같은 방식으로 처리). `tool_call_update`에 `status`도 `content`도 없으면 이벤트를 내지 않는다.

| ACP | SEP |
|---|---|
| client `session/prompt` | `session.state running` |
| `session/update` `user_message_chunk` (text) | `user.message` |
| `agent_message_chunk` (text) | `message.delta` |
| `agent_thought_chunk` (text) | `reasoning.delta` |
| `tool_call` | `tool {id: toolCallId, name: _meta["x.ai/tool"].name, title, kind: _meta["x.ai/tool"].kind, status: "pending", input: rawInput}` (`status`가 있으면 그 값 소문자) |
| `tool_call_update` | `tool {id, status?: 소문자, content?}`. content 항목: `{type:"diff",path,oldText,newText}` → `diff{path,old,new}`; `{type:"content",content:{type:"text",text}}` → `text`; `{type:"content",content:{type:"image",mimeType,data}}` → `image{mime,data}` |
| `usage_update` | `usage {used, size}` |
| `session_info_update` (`title` 있음) | `session.title` |
| agent `session/request_permission` | `permission.request {id: 요청 id, tool_call_id, title: toolCall.title, options}` + `session.state waiting_permission` |
| x `interaction_resolved` | `permission.resolved {tool_call_id}` + `session.state running` |
| x `turn_completed` | `turn.end {stop_reason, error: stop_reason=="error"이면 agent_result}` + `session.state idle` (`error`면 `error`) |
| client가 보낸 `session/load`의 응답 | `replay.end` |
| client가 보낸 `session/prompt`의 오류 응답 | 같은 턴에 `turn.end`가 아직 없으면 `turn.end {stop_reason:"error", error: error.data.message 또는 error.message}` + `session.state error` |
| 그 밖 (`available_commands_update`, `tool_call_delta_chunk`, `effort_applied`, `response_completed`, `session_summary_generated`, `retry_state`, `background_tasks`, `model_changed`, `_x.ai/session/setup`, `_x.ai/queue/changed`, `_x.ai/models/update`, `_x.ai/mcp*`, `_x.ai/session/prompt_complete`, `_x.ai/sessions/changed`) | 없음 |

재생(`_meta.isReplay`)도 같은 규칙으로 변환한다. 재생 중 `session/request_permission`이 오면(대기 중 승인 재전송) 그대로 `permission.request`를 낸다.

기대 출력: `tests/fixtures/sep/<fixture>.json` (SEP 배열). 트랙 A가 위 표로 만들고 주 모델이 검토한다. 테스트는 각 `acp/*.jsonl`을 `dir`에 따라 `on_client`/`on_agent`에 넣어 결과가 파일과 같은지 본다.

SEP 타입은 `src/sep.rs`(커밋됨)가 정본이다.

## 4. 세션 WS와 REST (트랙 A: `agent.rs`, `session.rs`, `board.rs`)

| 경로 | 동작 |
|---|---|
| `GET /healthz` | 인증 없음. `ok` |
| `GET /api/node` | `{host, version}` (기존) |
| `GET /api/sessions` | keeper로 `_x.ai/sessions/list` → 그대로 `{sessions:[…]}` |
| `WS /ws/board` | 연결 시 목록 한 번, 이후 keeper가 받는 `sessions/changed`를 `{t:"board.upsert", sessions:[…]}`/`{t:"board.remove", ids:[…]}`로 전달. keeper가 `pending_interaction`/`interaction_resolved`를 받으면 해당 행의 `activity`를 `needs_input`/`working`으로 바꿔 `board.upsert` |
| `POST /api/sessions` | body `{cwd, worktree?: {name}, mode?: "ask"|"auto"|"always-approve", prompt?: string}`. `worktree`가 있으면 `_x.ai/git/worktree/create` 후 그 경로로 `session/new`. 초기 모드는 `_meta`로(`yoloMode`/`autoMode`). keeper가 붙는다. 응답 `{sessionId}` |
| `WS /ws/session/:id` | 자식 하나 생성 → initialize/authenticate → `session/load` → SEP 스트림(재생, `replay.end`, 실시간). 받는 `ClientMsg`: `prompt` → `session/prompt`(이미지는 `{type:"image",mimeType,data}` 블록), `cancel` → `session/cancel` 알림, `permission` → 해당 id에 응답, `mode` → §2의 모드 알림(ask: `{yolo_mode:false,auto_mode:false,permission_mode:"ask"}`, auto: `{auto_mode:true,permission_mode:"auto"}`, always-approve: `{yolo_mode:true}`, 모두 `sessionId` 포함). WS가 닫히면 자식 종료 |

```rust
// agent.rs
pub struct AgentConfig { pub distill_bin: PathBuf }
pub struct AgentConn { /* 자식 + 송수신 */ }
impl AgentConn {
    pub async fn spawn(cfg: &AgentConfig) -> anyhow::Result<Self>; // initialize+authenticate까지
    pub async fn request(&mut self, method: &str, params: Value) -> anyhow::Result<Value>;
    pub fn notify(&self, method: &str, params: Value) -> anyhow::Result<()>;
    pub fn respond(&self, id: Value, result: Value) -> anyhow::Result<()>;
    pub async fn next(&mut self) -> Option<Value>; // 응답이 아닌 agent→client 메시지
}
```

## 5. 경계 보안 (트랙 B: `guard.rs`, `listen.rs`, `untrusted.rs`)

- **신뢰 리스너**: Unix(Linux/macOS)는 `~/.distill/postudio/node.sock`, 디렉터리 0700, 기존 소켓 파일은 지우고 다시 만든다. Windows는 `127.0.0.1:7410`. `--listen tcp:127.0.0.1:PORT`로 바꿀 수 있다(개발용).
- **비신뢰 리스너**: `--untrusted-listen`(기본 Unix `~/.distill/postudio/untrusted.sock`). 경로: `/p/:port/*rest` → `http://127.0.0.1:port/rest` 프록시(WS 포함), `/f/:sessionId/*path` → 세션 cwd 아래 파일. `/api`, `/ws`는 404.
- **guard** (신뢰 리스너의 `/api/*`, `/ws/*`):
  1. `Tailscale-User-Login` == `--owner` (기존). 아니면 403.
  2. `Origin`이 있으면 허용 목록에 있어야 한다. 목록 = `--allow-origin`(여러 번) . WS는 `Origin` 필수. 아니면 403.
  3. `POST`/`PUT`/`DELETE`는 `Content-Type: application/json`만. 아니면 415.
- **파일 서빙**: 경로를 정규화해 세션 cwd 밖이면 404. `..`, 절대경로, 밖을 가리키는 심볼릭 링크 거부. 응답에 `Content-Security-Policy: sandbox` 없이 그대로 서빙해도 된다(출처가 분리되어 있으므로).
- 시험 필수: 위조 Origin, Origin 없는 WS, 헤더 없음, 다른 사용자, `text/plain` POST, `..`, 심볼릭 링크 탈출, 비신뢰 리스너의 `/api` 접근.

## 6. Web Push (트랙 D: `push.rs`)

- VAPID(ES256, `ring`) + RFC 8291 `aes128gcm`. RFC 8291 부록 A 테스트 벡터로 암호화 검증.
- 키: `~/.distill/postudio/vapid.json`(없으면 생성, 0600). 구독: `~/.distill/postudio/push-subscriptions.json`.
- 경로: `GET /api/push/key` → `{publicKey}`(base64url), `POST /api/push/subscribe` body = 브라우저 `PushSubscription.toJSON()`, `POST /api/push/unsubscribe {endpoint}`.
- 2026-11-11 결정: MVP 알림은 Web Push로 간다. 별도 호스팅·Firebase 프로젝트 없음. 안드로이드 Chrome은 FCM 엔드포인트로 전달되지만 내용은 종단 암호화된다. node 호스트에 인터넷 HTTPS 송신이 필요하다(GPU 컨테이너는 3단계에서 확인).
- 트리거: keeper가 `pending_interaction`(kind `permission`·`question`)을 받았을 때. 같은 `tool_call_id`는 한 번만.
- 페이로드: `{"title": "<세션 제목> 승인 대기", "body": "<toolCall.title>", "url": "/s/<sessionId>", "tag": "<sessionId>"}`.
- 발송 실패 404/410이면 구독 삭제.

```rust
pub struct Push { /* 키, 구독, http client */ }
impl Push {
    pub fn load_or_create(dir: &Path) -> anyhow::Result<Self>;
    pub fn routes(self: Arc<Self>) -> axum::Router;
    pub async fn notify_all(&self, payload: &serde_json::Value);
}
```

## 7. 개발 실행 (로컬 완성용)

- `postudio-probe`에 `dev-stack` 바이너리: 가짜 모델 서버 + 샌드박스 + leader를 띄우고, 그 환경으로 `postudio-node --listen tcp:127.0.0.1:7410 --owner me@example.com --allow-origin http://127.0.0.1:5173 --web-dir apps/postudio/build`를 실행한다. 가짜 모델의 기본 응답과 대본 몇 개(승인 필요 편집, 이미지)를 넣는다. 비용 없음.
- 브라우저 확인 시 `Tailscale-User-Login`은 개발 서버 프록시(Vite)가 붙인다. node에는 우회 옵션을 두지 않는다.

## 8. A2 착수 시 결정 (2026-11-11)

1. `AgentConn`은 액터로 바꾼다: `AgentConn::spawn(cfg) -> anyhow::Result<(AgentHandle, mpsc::UnboundedReceiver<Value>)>`. `AgentHandle`은 `Clone`이고 `request(&self, ..)`, `notify(&self, ..)`, `respond(&self, ..)`를 가진다(대기 중 요청은 id → oneshot 맵). 수신 채널에는 응답이 아닌 agent→client 메시지가 온다. 자식이 죽으면 채널이 닫힌다.
2. `POST /api/sessions`의 `worktree`는 A2에서 구현하지 않는다(있으면 501). 통합 I2에서 실제 호출을 캡처해 정한다.
3. 확장 메서드 이름은 앞의 `_`를 떼고 비교한다(`_x.ai/session_notification`과 `x.ai/session_notification` 모두 처리). `params.method`로 감싼 형태(`{"method":"_x.ai/…","params":{"method":"x.ai/…","params":{…}}}`)도 풀어서 처리한다(leader `method_of`와 같은 규칙).
4. `/api/node`는 `{host, version}`만 낸다. `agent_url`과 그 테스트는 지운다. `--insecure-no-owner`는 없앤다.
