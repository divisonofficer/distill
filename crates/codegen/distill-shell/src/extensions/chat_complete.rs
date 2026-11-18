use super::{ExtResult, parse_params, to_raw_response};
use crate::agent::MvpAgent;
use agent_client_protocol as acp;
use distill_sampling_types::{ConversationItem, ConversationRequest};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

const MAX_MESSAGES: usize = 128;
const MAX_INPUT_BYTES: usize = 1024 * 1024;
const MAX_PROGRESS_TEXT_BYTES: usize = 1024 * 1024;
const MAX_REQUEST_ID_BYTES: usize = 128;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(50);
const MIN_PROGRESS_GROWTH: usize = 128;
const MAX_OUTPUT_TOKENS: u32 = 4096;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChatCompleteRequest {
    model_id: String,
    messages: Vec<ChatMessage>,
    #[serde(default)]
    request_id: Option<String>,
}

#[derive(Deserialize)]
struct ChatMessage {
    role: ChatRole,
    text: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum ChatRole {
    User,
    Assistant,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ChatCompleteResponse {
    model_id: String,
    text: String,
}

struct ChatProgress {
    gateway: distill_acp_lib::AcpAgentGatewaySender,
    request_id: String,
    text: String,
    last_notification_at: Instant,
    last_notification_len: usize,
}

impl ChatProgress {
    fn new(gateway: distill_acp_lib::AcpAgentGatewaySender, request_id: String) -> Self {
        Self {
            gateway,
            request_id,
            text: String::new(),
            last_notification_at: Instant::now(),
            last_notification_len: 0,
        }
    }

    fn push(&mut self, delta: &str) {
        let remaining = MAX_PROGRESS_TEXT_BYTES.saturating_sub(self.text.len());
        let mut end = remaining.min(delta.len());
        while !delta.is_char_boundary(end) {
            end -= 1;
        }
        self.text.push_str(&delta[..end]);

        let growth = self.text.len().saturating_sub(self.last_notification_len);
        let threshold = MIN_PROGRESS_GROWTH.max(self.last_notification_len / 2);
        if growth >= threshold && self.last_notification_at.elapsed() >= PROGRESS_INTERVAL {
            self.notify();
        }
    }

    fn finish(&mut self) {
        if !self.text.is_empty() && self.last_notification_len != self.text.len() {
            self.notify();
        }
    }

    fn notify(&mut self) {
        let params = serde_json::json!({
            "requestId": self.request_id,
            "text": self.text,
        });
        if let Ok(raw) = serde_json::value::to_raw_value(&params) {
            self.gateway
                .forward_fire_and_forget(acp::ExtNotification::new(
                    "x.ai/chat/progress",
                    raw.into(),
                ));
        }
        self.last_notification_at = Instant::now();
        self.last_notification_len = self.text.len();
    }
}

fn validate(request: &ChatCompleteRequest) -> Result<(), acp::Error> {
    if request.model_id.trim().is_empty() {
        return Err(acp::Error::invalid_params().data("modelId must not be empty"));
    }
    if request.request_id.as_ref().is_some_and(|request_id| {
        request_id.trim().is_empty()
            || request_id.len() > MAX_REQUEST_ID_BYTES
            || request_id.chars().any(char::is_control)
    }) {
        return Err(acp::Error::invalid_params().data("requestId is invalid"));
    }
    if request.messages.is_empty() || request.messages.len() > MAX_MESSAGES {
        return Err(acp::Error::invalid_params().data("messages must contain 1 to 128 items"));
    }
    if !matches!(
        request.messages.first().map(|message| &message.role),
        Some(ChatRole::User)
    ) {
        return Err(acp::Error::invalid_params().data("messages must start with a user message"));
    }
    if !matches!(
        request.messages.last().map(|message| &message.role),
        Some(ChatRole::User)
    ) {
        return Err(acp::Error::invalid_params().data("messages must end with a user message"));
    }
    let mut input_bytes = 0usize;
    for message in &request.messages {
        input_bytes = input_bytes
            .checked_add(message.text.len())
            .ok_or_else(|| acp::Error::invalid_params().data("messages exceed 1 MiB"))?;
        if input_bytes > MAX_INPUT_BYTES {
            return Err(acp::Error::invalid_params().data("messages exceed 1 MiB"));
        }
    }
    Ok(())
}

pub(crate) async fn handle(agent: &MvpAgent, args: &acp::ExtRequest) -> ExtResult {
    let request: ChatCompleteRequest = parse_params(args)
        .map_err(|_| acp::Error::invalid_params().data("invalid chat completion params"))?;
    validate(&request)?;
    let mut progress = request
        .request_id
        .clone()
        .map(|request_id| ChatProgress::new(agent.gateway.clone(), request_id));

    let model = agent
        .resolve_model_id(&acp::ModelId::new(request.model_id.clone()))
        .map_err(|_| acp::Error::invalid_params().data("unknown model id"))?;
    let sampling_config = agent.prepare_sampling_config_for_model(&model, None);
    let output_tokens = sampling_config
        .max_completion_tokens
        .map_or(MAX_OUTPUT_TOKENS, |limit| limit.min(MAX_OUTPUT_TOKENS));
    let items = request
        .messages
        .into_iter()
        .map(|message| match message.role {
            ChatRole::User => ConversationItem::user(message.text),
            ChatRole::Assistant => ConversationItem::assistant(message.text),
        })
        .collect();
    let sampling_request = ConversationRequest {
        items,
        model: Some(sampling_config.model.clone()),
        max_output_tokens: Some(output_tokens),
        ..Default::default()
    };

    let client = crate::sampling::SamplingClient::new(sampling_config)
        .map_err(|_| acp::Error::internal_error().data("chat completion failed"))?;
    let response = client
        .conversation_collect_with_text(sampling_request, |delta| {
            if let Some(progress) = progress.as_mut() {
                progress.push(delta);
            }
        })
        .await
        .map_err(|_| acp::Error::internal_error().data("chat completion failed"))?;
    if let Some(progress) = progress.as_mut() {
        progress.finish();
    }
    to_raw_response(&ChatCompleteResponse {
        model_id: request.model_id,
        text: response.assistant_text(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(messages: Vec<ChatMessage>) -> ChatCompleteRequest {
        ChatCompleteRequest {
            model_id: "model".into(),
            messages,
            request_id: None,
        }
    }

    fn user(text: impl Into<String>) -> ChatMessage {
        ChatMessage {
            role: ChatRole::User,
            text: text.into(),
        }
    }

    fn assistant(text: impl Into<String>) -> ChatMessage {
        ChatMessage {
            role: ChatRole::Assistant,
            text: text.into(),
        }
    }

    #[test]
    fn validates_chat_roles_and_limits() {
        assert!(validate(&request(vec![user("question")])).is_ok());
        assert!(validate(&request(vec![assistant("answer"), user("question")])).is_err());
        assert!(validate(&request(vec![user("question"), assistant("answer")])).is_err());
        let mut at_message_limit = vec![user("first")];
        at_message_limit.extend((0..MAX_MESSAGES - 2).map(|_| assistant("prior answer")));
        at_message_limit.push(user("last"));
        assert!(validate(&request(at_message_limit)).is_ok());
        assert!(validate(&request(vec![user("x".repeat(MAX_INPUT_BYTES))])).is_ok());
        let too_many = std::iter::repeat_with(|| user("x"))
            .take(MAX_MESSAGES + 1)
            .collect();
        assert!(validate(&request(too_many)).is_err());
        assert!(validate(&request(vec![user("x".repeat(MAX_INPUT_BYTES + 1))])).is_err());
        assert!(validate(&request(vec![user("🦣".repeat(MAX_INPUT_BYTES / 4 + 1))])).is_err());
    }

    #[test]
    fn rejects_empty_model_and_invalid_role_values() {
        let mut empty_model = request(vec![user("question")]);
        empty_model.model_id = "  ".into();
        assert!(validate(&empty_model).is_err());
        let err = serde_json::from_value::<ChatMessage>(serde_json::json!({
            "role": "tool",
            "text": "ignored"
        }));
        assert!(err.is_err());

        let mut invalid_request_id = request(vec![user("question")]);
        invalid_request_id.request_id = Some("bad\nrequest".into());
        assert!(validate(&invalid_request_id).is_err());
        invalid_request_id.request_id = Some("x".repeat(MAX_REQUEST_ID_BYTES + 1));
        assert!(validate(&invalid_request_id).is_err());
    }

    fn configured_agent(
        base_url: &str,
    ) -> (
        MvpAgent,
        tempfile::TempDir,
        tokio::sync::mpsc::UnboundedReceiver<distill_acp_lib::AcpClientMessage>,
    ) {
        use crate::agent::config::{Config, ModelEntry};
        use crate::agent::remote_config::ModelsManager;
        use distill_login::{AuthManager, GrokComConfig};
        use indexmap::IndexMap;
        use std::sync::Arc;

        let auth_home = tempfile::tempdir().unwrap();
        let auth = Arc::new(AuthManager::new(auth_home.path(), GrokComConfig::default()));
        let config = Config::default();
        let mut model = ModelEntry::fallback("provider-model", &config.endpoints);
        model.info.base_url = base_url.into();
        model.info.api_backend = crate::sampling::ApiBackend::ChatCompletions;
        model.info.max_completion_tokens = Some(8192);
        model.api_key = Some("mock-api-key".into());
        let models = IndexMap::from([("configured-alias".to_string(), model)]);
        let manager = ModelsManager::new(
            Some(models.clone()),
            models,
            acp::ModelId::new("configured-alias"),
            auth.clone(),
            config.clone(),
        );
        let (gateway_tx, gateway_rx) = tokio::sync::mpsc::unbounded_channel();
        let agent = MvpAgent::with_models(
            distill_acp_lib::AcpAgentGatewaySender::new(gateway_tx),
            &config,
            auth,
            manager,
        );
        (agent, auth_home, gateway_rx)
    }

    fn ext_request(params: serde_json::Value) -> acp::ExtRequest {
        acp::ExtRequest::new(
            "x.ai/chat/complete",
            serde_json::value::to_raw_value(&params).unwrap().into(),
        )
    }

    #[tokio::test(flavor = "current_thread")]
    async fn dispatch_resolves_configured_model_and_sends_tool_free_bounded_request() {
        use distill_test_support::sse::chat_completion_script_exact;
        use distill_test_support::{MockInferenceServer, ScriptedResponse};

        tokio::task::LocalSet::new()
            .run_until(async {
                let server = MockInferenceServer::start().await.unwrap();
                let answer = "word ".repeat(32);
                server.set_chunk_delay(Some(Duration::from_millis(20)));
                server.enqueue_response(
                    "/v1/chat/completions",
                    ScriptedResponse::sse(chat_completion_script_exact(&answer, "provider-model")),
                );
                let (agent, _auth_home, mut gateway_rx) = configured_agent(&server.url());
                let params = serde_json::json!({
                    "modelId": "configured-alias",
                    "requestId": "request-42",
                    "messages": [{"role": "user", "text": "question"}]
                });
                let agent = std::rc::Rc::new(agent);
                let call_agent = agent.clone();
                let call = tokio::task::spawn_local(async move {
                    <MvpAgent as acp::Agent>::ext_method(call_agent.as_ref(), ext_request(params))
                        .await
                });
                let first = tokio::time::timeout(Duration::from_secs(3), async {
                    loop {
                        if let Some(distill_acp_lib::AcpClientMessage::ExtNotification(args)) =
                            gateway_rx.recv().await
                        {
                            if args.request.method.as_ref() == "x.ai/chat/progress" {
                                break serde_json::from_str::<serde_json::Value>(
                                    args.request.params.get(),
                                )
                                .unwrap();
                            }
                        }
                    }
                })
                .await
                .expect("provider delta arrives before completion");
                assert_eq!(first["requestId"], "request-42");
                assert!(first["text"].as_str().unwrap().len() >= MIN_PROGRESS_GROWTH);
                assert!(!call.is_finished());
                let response = call.await.unwrap().expect("chat completion succeeds");
                let result: serde_json::Value = serde_json::from_str(response.0.get()).unwrap();
                assert_eq!(result["modelId"], "configured-alias");
                assert_eq!(result["text"], answer);

                let mut progress = None;
                while let Ok(message) = gateway_rx.try_recv() {
                    if let distill_acp_lib::AcpClientMessage::ExtNotification(args) = message {
                        if args.request.method.as_ref() == "x.ai/chat/progress" {
                            progress = Some(
                                serde_json::from_str::<serde_json::Value>(
                                    args.request.params.get(),
                                )
                                .unwrap(),
                            );
                        }
                    }
                }
                assert_eq!(
                    progress,
                    Some(serde_json::json!({
                        "requestId": "request-42",
                        "text": answer
                    }))
                );
                let sent = server.requests().pop().expect("provider request recorded");
                let body = sent.body.unwrap();
                assert_eq!(body["model"], "provider-model");
                assert_eq!(body["max_tokens"], MAX_OUTPUT_TOKENS);
                assert!(body.get("tools").is_none());
                assert!(body.get("tool_choice").is_none());

                let missing = <MvpAgent as acp::Agent>::ext_method(
                    &agent,
                    ext_request(serde_json::json!({
                        "modelId": "not-configured",
                        "messages": [{"role": "user", "text": "question"}]
                    })),
                )
                .await
                .expect_err("unknown models rejected");
                assert_eq!(missing.code, acp::Error::invalid_params().code);
                assert_eq!(
                    server.request_count(),
                    1,
                    "resolution failure sends no inference request"
                );
            })
            .await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn provider_errors_do_not_leak_response_secrets() {
        use distill_test_support::{MockInferenceServer, ScriptedResponse};

        tokio::task::LocalSet::new()
            .run_until(async {
                let server = MockInferenceServer::start().await.unwrap();
                server.enqueue_response(
                    "/v1/chat/completions",
                    ScriptedResponse::json(
                        400,
                        serde_json::json!({"error": "mock-provider-secret"}),
                    ),
                );
                let (agent, _auth_home, _gateway_rx) = configured_agent(&server.url());
                let error = <MvpAgent as acp::Agent>::ext_method(
                    &agent,
                    ext_request(serde_json::json!({
                        "modelId": "configured-alias",
                        "messages": [{"role": "user", "text": "question"}]
                    })),
                )
                .await
                .expect_err("provider error propagated safely");
                assert_eq!(error.code, acp::Error::internal_error().code);
                assert!(!format!("{error:?}").contains("mock-provider-secret"));
            })
            .await;
    }
}
