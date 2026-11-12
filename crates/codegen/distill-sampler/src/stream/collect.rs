// Modified for Distill by Samuel Fajreldines, 2026.
//! Buffered consumer for [`SamplingEvent`] streams.
//!
//! Drains a Layer-2 event stream into the final `(ConversationResponse, InferenceLatencyStats)` pair.
//! Used by callers that don't need streaming UI updates (e.g., compaction, `/btw`, dream-model calls).

use futures_util::StreamExt;
use futures_util::stream::Stream;

use distill_sampling_types::ConversationResponse;

use crate::events::{SamplingErrorInfo, SamplingErrorKind, SamplingEvent};
use crate::metrics::InferenceLatencyStats;

/// Drain a [`SamplingEvent`] stream, returning the final response.
/// Intermediate events (deltas, retries, metadata) are silently consumed; this function is for callers that only need the final result.
/// This error only fires when the stream was dropped mid-flight, e.g. the producer panicked or the underlying `tokio::spawn` was cancelled.
pub async fn collect_response(
    stream: impl Stream<Item = SamplingEvent>,
) -> Result<(ConversationResponse, InferenceLatencyStats), SamplingErrorInfo> {
    collect_response_with_text(stream, |_| {}).await
}

/// Collect a response while forwarding real assistant text deltas in stream order.
pub async fn collect_response_with_text(
    stream: impl Stream<Item = SamplingEvent>,
    mut on_text: impl FnMut(&str),
) -> Result<(ConversationResponse, InferenceLatencyStats), SamplingErrorInfo> {
    tokio::pin!(stream);

    while let Some(event) = stream.next().await {
        match event {
            SamplingEvent::Completed {
                response, metrics, ..
            } => return Ok((*response, metrics)),
            SamplingEvent::Failed { error, .. } => return Err(error),
            SamplingEvent::ChannelToken {
                channel: crate::events::SamplingChannel::Text,
                text,
                ..
            } => on_text(&text),
            // Ignore reasoning and other events in this text-only collector.
            _ => {}
        }
    }

    Err(SamplingErrorInfo {
        kind: SamplingErrorKind::Api,
        status_code: None,
        message: "stream ended without Completed or Failed".to_string(),
        is_retryable: false,
        retry_after_secs: None,
        should_retry: None,
        error_code: None,
        model_metadata: None,
        empty_response_context: None,
        doom_loop_triggers: None,
        doom_loop_aborted_at_chunk: None,
        credential: distill_sampling_types::SentCredential::Unknown,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use distill_sampling_types::{ConversationItem, SamplingError, StopReason};
    use futures_util::stream;

    use crate::events::SamplingChannel;
    use crate::stream::stream_chat_completions;
    use crate::types::RequestId;
    use distill_sampling_types::{
        ChatChunkChoice, ChatChunkDelta, ChatCompletionChunk, FinishReason, Role,
    };
    use std::time::Duration;

    fn rid() -> RequestId {
        RequestId::from("collect-test")
    }

    fn text_chunk(text: &str) -> ChatCompletionChunk {
        ChatCompletionChunk {
            id: "chunk".into(),
            object: "chat.completion.chunk".into(),
            created: 0,
            model: "test-model".into(),
            choices: vec![ChatChunkChoice {
                index: 0,
                delta: ChatChunkDelta {
                    role: Some(Role::Assistant),
                    content: Some(text.to_string()),
                    reasoning_content: None,
                    tool_calls: vec![],
                    tool_call_id: None,
                },
                finish_reason: None,
            }],
            usage: None,
            system_fingerprint: None,
        }
    }

    fn final_chunk() -> ChatCompletionChunk {
        ChatCompletionChunk {
            id: "chunk".into(),
            object: "chat.completion.chunk".into(),
            created: 0,
            model: "test-model".into(),
            choices: vec![ChatChunkChoice {
                index: 0,
                delta: ChatChunkDelta::default(),
                finish_reason: Some(FinishReason::Stop),
            }],
            usage: None,
            system_fingerprint: None,
        }
    }

    #[tokio::test]
    async fn happy_path_returns_response_and_metrics() {
        let chunks: Vec<Result<ChatCompletionChunk, SamplingError>> =
            vec![Ok(text_chunk("hello")), Ok(final_chunk())];
        let raw = stream::iter(chunks).boxed();
        let events = stream_chat_completions(raw, None, rid(), Duration::from_secs(60));

        let mut emitted = Vec::new();
        let (response, _metrics) = collect_response_with_text(events, |text| {
            emitted.push(text.to_owned());
        })
        .await
        .expect("happy path returns Ok");
        let a = response.assistant().expect("assistant item present");
        assert_eq!(emitted.concat(), a.content.as_ref());
        assert_eq!(a.content.as_ref(), "hello");
        assert_eq!(response.stop_reason, Some(StopReason::Stop));
    }

    #[tokio::test]
    async fn failure_path_returns_error() {
        let chunks: Vec<Result<ChatCompletionChunk, SamplingError>> = vec![
            Ok(text_chunk("partial")),
            Err(SamplingError::EventStreamError("boom".into())),
        ];
        let raw = stream::iter(chunks).boxed();
        let events = stream_chat_completions(raw, None, rid(), Duration::from_secs(60));

        let err = collect_response(events).await.expect_err("error returned");
        assert!(err.message.contains("boom"));
    }

    #[tokio::test]
    async fn truncated_stream_returns_error() {
        let truncated = stream::iter(vec![SamplingEvent::StreamStarted {
            request_id: rid(),
            timestamp_ms: 0,
        }]);
        let err = collect_response(truncated)
            .await
            .expect_err("truncated stream returns Err");
        assert_eq!(err.kind, SamplingErrorKind::Api);
        assert!(err.message.contains("stream ended without"));
    }

    #[tokio::test]
    async fn intermediate_events_are_dropped() {
        let token = SamplingEvent::ChannelToken {
            request_id: rid(),
            channel: SamplingChannel::Text,
            text: "hi".into(),
            chunk_index: 1,
        };
        let completed = SamplingEvent::Completed {
            request_id: rid(),
            response: Box::new(ConversationResponse {
                items: vec![ConversationItem::assistant("hi")],
                stop_reason: Some(StopReason::Stop),
                usage: None,
                cost_usd_ticks: None,
                message_chunks_emitted: 1,
                doom_loop_signals: Vec::new(),
                stop_message: None,
                message_id: None,
                raw_stop_reason: None,
                stop_sequence: None,
            }),
            metrics: InferenceLatencyStats::default(),
        };
        let s = stream::iter(vec![token, completed]);
        let (response, _) = collect_response(s).await.expect("ok");
        let a = response.assistant().expect("assistant item present");
        assert_eq!(a.content.as_ref(), "hi");
    }

    #[tokio::test]
    async fn text_callback_runs_before_completion_and_ignores_reasoning() {
        let token = |channel, text: &str| SamplingEvent::ChannelToken {
            request_id: rid(),
            channel,
            text: text.into(),
            chunk_index: 1,
        };
        let completed = SamplingEvent::Completed {
            request_id: rid(),
            response: Box::new(ConversationResponse {
                items: vec![ConversationItem::assistant("visible")],
                stop_reason: Some(StopReason::Stop),
                usage: None,
                cost_usd_ticks: None,
                message_chunks_emitted: 1,
                doom_loop_signals: Vec::new(),
                stop_message: None,
                message_id: None,
                raw_stop_reason: None,
                stop_sequence: None,
            }),
            metrics: InferenceLatencyStats::default(),
        };
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let (delta_tx, mut delta_rx) = tokio::sync::mpsc::unbounded_channel();
        let events = async_stream::stream! {
            yield token(SamplingChannel::Reasoning, "hidden");
            yield token(SamplingChannel::Text, "visible");
            let _ = release_rx.await;
            yield completed;
        };
        let task = tokio::spawn(async move {
            collect_response_with_text(events, move |text| {
                delta_tx.send(text.to_owned()).unwrap();
            })
            .await
        });

        assert_eq!(delta_rx.recv().await.as_deref(), Some("visible"));
        assert!(delta_rx.try_recv().is_err());
        assert!(!task.is_finished());
        release_tx.send(()).unwrap();
        let (response, _) = task.await.unwrap().unwrap();
        assert_eq!(response.assistant_text(), "visible");
    }
}
