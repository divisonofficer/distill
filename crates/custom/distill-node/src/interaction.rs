use std::collections::{BTreeMap, HashSet};

use serde_json::{Value, json};

use crate::sep::{PlanReplyOutcome, Question, QuestionMode, QuestionOption, QuestionReplyOutcome};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Interaction {
    Question {
        id: Value,
        session_id: String,
        tool_call_id: String,
        mode: QuestionMode,
        questions: Vec<Question>,
        preview: Option<String>,
    },
    Plan {
        id: Value,
        session_id: String,
        tool_call_id: String,
        plan_content: Option<String>,
    },
}

impl Interaction {
    pub fn id(&self) -> &Value {
        match self {
            Self::Question { id, .. } | Self::Plan { id, .. } => id,
        }
    }

    pub fn tool_call_id(&self) -> &str {
        match self {
            Self::Question { tool_call_id, .. } | Self::Plan { tool_call_id, .. } => tool_call_id,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Question,
    Plan,
}

/// Parses direct extension requests and the `{method, params}` extension wrapper.
pub fn parse_request(msg: &Value, expected_session: &str) -> Result<Option<Interaction>, String> {
    let Some(method) = msg.get("method").and_then(Value::as_str) else {
        return Ok(None);
    };
    let outer_kind = kind(method);
    let params = msg.get("params").and_then(Value::as_object);
    let Some(params) = params else {
        return if outer_kind.is_some() {
            Err("interaction params must be an object".into())
        } else {
            Ok(None)
        };
    };
    let inner = params.get("method");
    let inner_kind = match inner {
        None => None,
        Some(Value::String(name)) => kind(name),
        Some(_) if outer_kind.is_some() => return Err("wrapped method must be a string".into()),
        Some(_) => None,
    };
    let (kind, body) = match (outer_kind, inner, inner_kind) {
        (Some(a), None, _) => (a, msg.get("params").unwrap_or(&Value::Null)),
        (Some(a), Some(_), Some(b)) if a == b => (a, params.get("params").unwrap_or(&Value::Null)),
        (Some(_), Some(_), _) => return Err("wrapped method does not match request".into()),
        (None, Some(_), Some(b)) => (b, params.get("params").unwrap_or(&Value::Null)),
        (None, _, _) => return Ok(None),
    };
    if !body.is_object() {
        return Err("interaction params must be an object".into());
    }
    let id = msg
        .get("id")
        .filter(|v| matches!(v, Value::String(_) | Value::Number(_)))
        .cloned()
        .ok_or_else(|| "interaction request id must be a number or string".to_owned())?;
    let session_id = string(body, "sessionId")?;
    if session_id != expected_session {
        return Err("interaction session does not match this connection".into());
    }
    let tool_call_id = nonempty(string(body, "toolCallId")?, "toolCallId")?;
    match kind {
        Kind::Question => {
            let mode = match string(body, "mode")? {
                "default" => QuestionMode::Default,
                "plan" => QuestionMode::Plan,
                _ => return Err("unsupported question mode".into()),
            };
            let items = body
                .get("questions")
                .and_then(Value::as_array)
                .filter(|items| !items.is_empty())
                .ok_or_else(|| "questions must be a nonempty array".to_owned())?;
            let mut seen_questions = HashSet::new();
            let mut questions = Vec::with_capacity(items.len());
            for item in items {
                let question = nonempty(string(item, "question")?, "question")?.to_owned();
                if !seen_questions.insert(question.clone()) {
                    return Err("questions must be unique".into());
                }
                let options = item
                    .get("options")
                    .and_then(Value::as_array)
                    .ok_or_else(|| "question options must be an array".to_owned())?;
                let mut seen_options = HashSet::new();
                let mut parsed_options = Vec::with_capacity(options.len());
                for option in options {
                    let label = nonempty(string(option, "label")?, "option label")?.to_owned();
                    let description = string(option, "description")?.to_owned();
                    if !seen_options.insert(label.clone()) {
                        return Err("option labels must be unique".into());
                    }
                    parsed_options.push(QuestionOption { label, description });
                }
                let multi_select = match item.get("multiSelect") {
                    None | Some(Value::Null) => false,
                    Some(Value::Bool(value)) => *value,
                    Some(_) => return Err("multiSelect must be a boolean".to_owned()),
                };
                questions.push(Question {
                    question,
                    options: parsed_options,
                    multi_select,
                });
            }
            let preview = optional_string(body, "preview")?;
            Ok(Some(Interaction::Question {
                id,
                session_id: session_id.to_owned(),
                tool_call_id: tool_call_id.to_owned(),
                mode,
                questions,
                preview,
            }))
        }
        Kind::Plan => Ok(Some(Interaction::Plan {
            id,
            session_id: session_id.to_owned(),
            tool_call_id: tool_call_id.to_owned(),
            plan_content: optional_string(body, "planContent")?,
        })),
    }
}

#[derive(Debug, Default)]
pub struct Outstanding(Vec<Interaction>);

impl Outstanding {
    pub fn insert(&mut self, request: Interaction) -> Result<(), String> {
        if self.0.iter().any(|item| item.id() == request.id()) {
            return Err("duplicate interaction request id".into());
        }
        self.0.push(request);
        Ok(())
    }

    pub fn answer_question(
        &mut self,
        id: Value,
        session_id: &str,
        tool_call_id: &str,
        outcome: QuestionReplyOutcome,
        answers: Option<BTreeMap<String, Vec<String>>>,
    ) -> Result<(Value, Value), String> {
        let index = self.index(&id)?;
        let Interaction::Question {
            session_id: expected_session,
            tool_call_id: expected_tool,
            questions,
            ..
        } = &self.0[index]
        else {
            return Err("interaction request kind does not match reply".into());
        };
        validate_target(session_id, tool_call_id, expected_session, expected_tool)?;
        let result = match outcome {
            QuestionReplyOutcome::Cancelled if answers.is_none() => json!({"outcome":"cancelled"}),
            QuestionReplyOutcome::Cancelled => {
                return Err("cancelled question reply cannot include answers".into());
            }
            QuestionReplyOutcome::Accepted => {
                let answers =
                    answers.ok_or_else(|| "accepted question reply needs answers".to_owned())?;
                validate_answers(questions, &answers)?;
                json!({"outcome":"accepted","answers":answers})
            }
        };
        self.0.remove(index);
        Ok((id, result))
    }

    pub fn answer_plan(
        &mut self,
        id: Value,
        session_id: &str,
        tool_call_id: &str,
        outcome: PlanReplyOutcome,
        feedback: Option<String>,
    ) -> Result<(Value, Value), String> {
        let index = self.index(&id)?;
        let Interaction::Plan {
            session_id: expected_session,
            tool_call_id: expected_tool,
            ..
        } = &self.0[index]
        else {
            return Err("interaction request kind does not match reply".into());
        };
        validate_target(session_id, tool_call_id, expected_session, expected_tool)?;
        if feedback
            .as_deref()
            .is_some_and(|text| text.trim().is_empty())
        {
            return Err("plan feedback cannot be empty".into());
        }
        if outcome == PlanReplyOutcome::Cancelled && feedback.is_none() {
            return Err("requesting plan changes needs feedback".into());
        }
        let mut result = json!({"outcome": outcome});
        if let Some(feedback) = feedback {
            result["feedback"] = Value::String(feedback);
        }
        self.0.remove(index);
        Ok((id, result))
    }

    pub fn remove_tool_call(&mut self, tool_call_id: &str) {
        self.0.retain(|item| item.tool_call_id() != tool_call_id);
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }

    fn index(&self, id: &Value) -> Result<usize, String> {
        self.0
            .iter()
            .position(|item| item.id() == id)
            .ok_or_else(|| "unknown or stale interaction request id".into())
    }
}

fn validate_target(
    session: &str,
    tool_call: &str,
    expected_session: &str,
    expected_tool: &str,
) -> Result<(), String> {
    if session != expected_session {
        return Err("interaction reply session does not match request".into());
    }
    if tool_call != expected_tool {
        return Err("interaction reply tool call does not match request".into());
    }
    Ok(())
}

fn validate_answers(
    questions: &[Question],
    answers: &BTreeMap<String, Vec<String>>,
) -> Result<(), String> {
    if answers.len() != questions.len() {
        return Err("answers must contain exactly one entry per question".into());
    }
    for question in questions {
        let values = answers
            .get(&question.question)
            .filter(|values| !values.is_empty())
            .ok_or_else(|| "every question needs an answer".to_owned())?;
        if !question.multi_select && values.len() != 1 {
            return Err("single-select questions need exactly one answer".into());
        }
        let mut seen = HashSet::new();
        if values
            .iter()
            .any(|value| value.trim().is_empty() || !seen.insert(value))
        {
            return Err("answers must be nonempty and unique".into());
        }
    }
    if answers
        .keys()
        .any(|key| !questions.iter().any(|q| q.question == *key))
    {
        return Err("answers contain an unknown question".into());
    }
    Ok(())
}

fn kind(method: &str) -> Option<Kind> {
    match method {
        "_x.ai/ask_user_question" | "x.ai/ask_user_question" => Some(Kind::Question),
        "_x.ai/exit_plan_mode" | "x.ai/exit_plan_mode" => Some(Kind::Plan),
        _ => None,
    }
}

fn string<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{key} must be a string"))
}

fn optional_string(value: &Value, key: &str) -> Result<Option<String>, String> {
    value
        .get(key)
        .filter(|field| !field.is_null())
        .map(|field| {
            field
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("{key} must be a string"))
        })
        .transpose()
}

fn nonempty<'a>(value: &'a str, field: &str) -> Result<&'a str, String> {
    if value.trim().is_empty() {
        Err(format!("{field} cannot be empty"))
    } else {
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn optional_fields_accept_absent_and_null() {
        let mut params = body();
        params["questions"][0]
            .as_object_mut()
            .unwrap()
            .remove("multiSelect");
        params["questions"][1]["multiSelect"] = Value::Null;
        params["preview"] = Value::Null;
        let request = wrapped("_x.ai/ask_user_question", params, json!(1));
        let Some(Interaction::Question {
            questions, preview, ..
        }) = parse_request(&request, "sid").unwrap()
        else {
            panic!("question expected")
        };
        assert!(questions.iter().all(|question| !question.multi_select));
        assert!(preview.is_none());
        assert!(
            parse_request(
                &wrapped(
                    "_x.ai/exit_plan_mode",
                    json!({"sessionId":"sid", "toolCallId":"p", "planContent":null}),
                    json!(2)
                ),
                "sid"
            )
            .is_ok()
        );
    }

    fn body() -> Value {
        json!({
            "sessionId": "sid",
            "toolCallId": "tool-1",
            "mode": "default",
            "questions": [{
                "question": "Choose",
                "options": [
                    {"label": "A", "description": "first"},
                    {"label": "B", "description": "second"}
                ],
                "multiSelect": true
            }, {
                "question": "One",
                "options": [{"label": "X", "description": "only"}],
                "multiSelect": false
            }]
        })
    }

    fn wrapped(method: &str, body: Value, id: Value) -> Value {
        json!({"jsonrpc":"2.0", "id":id, "method":method, "params":{
            "method": method.trim_start_matches('_'), "params":body
        }})
    }

    #[test]
    fn parses_direct_and_wrapped_question_and_plan_requests() {
        let mut question = body();
        question
            .as_object_mut()
            .unwrap()
            .remove("questions")
            .unwrap();
        question["questions"] = json!([{"question":"Q","options":[],"multiSelect":false}]);
        assert!(
            matches!(parse_request(&json!({"id":501,"method":"_x.ai/ask_user_question","params":question}), "sid"), Ok(Some(Interaction::Question { id, .. })) if id == json!(501))
        );
        assert!(
            matches!(parse_request(&wrapped("_x.ai/ask_user_question", body(), json!("question-id")), "sid"), Ok(Some(Interaction::Question { id, .. })) if id == json!("question-id"))
        );

        let plan = json!({"sessionId":"sid","toolCallId":"tool-plan","planContent":"## Plan"});
        assert!(
            matches!(parse_request(&json!({"id":502,"method":"_x.ai/exit_plan_mode","params":plan}), "sid"), Ok(Some(Interaction::Plan { plan_content: Some(text), .. })) if text == "## Plan")
        );
        assert!(
            matches!(parse_request(&wrapped("_x.ai/exit_plan_mode", json!({"sessionId":"sid","toolCallId":"tool-plan"}), json!("p")), "sid"), Ok(Some(Interaction::Plan { id, .. })) if id == json!("p"))
        );
    }

    #[test]
    fn rejects_malformed_and_cross_session_requests() {
        let malformed = json!({"sessionId":"sid","toolCallId":"t","mode":"default","questions":[]});
        assert!(
            parse_request(
                &json!({"id":1,"method":"_x.ai/ask_user_question","params":malformed}),
                "sid"
            )
            .is_err()
        );
        let mut invalid = body();
        invalid["questions"][0]["options"] = json!([{"label":7,"description":"bad"}]);
        assert!(
            parse_request(
                &json!({"id":1,"method":"_x.ai/ask_user_question","params":invalid}),
                "sid"
            )
            .is_err()
        );
        assert!(
            parse_request(
                &json!({"id":1,"method":"_x.ai/ask_user_question","params":body()}),
                "other"
            )
            .is_err()
        );
        assert!(
            parse_request(
                &wrapped("_x.ai/exit_plan_mode", json!({"sessionId":"sid"}), json!(1)),
                "sid"
            )
            .is_err()
        );
    }

    #[test]
    fn validates_answers_preserves_ids_and_rejects_stale_replies() {
        let request = parse_request(
            &wrapped("_x.ai/ask_user_question", body(), json!(501)),
            "sid",
        )
        .unwrap()
        .unwrap();
        let mut pending = Outstanding::default();
        pending.insert(request).unwrap();
        let mut answers = BTreeMap::new();
        answers.insert("Choose".to_owned(), vec!["A".to_owned(), "B".to_owned()]);
        answers.insert("One".to_owned(), vec!["free text".to_owned()]);
        let (response_id, result) = pending
            .answer_question(
                json!(501),
                "sid",
                "tool-1",
                QuestionReplyOutcome::Accepted,
                Some(answers.clone()),
            )
            .unwrap();
        assert_eq!(response_id, json!(501));
        assert_eq!(
            result,
            json!({"outcome":"accepted","answers":{"Choose":["A","B"],"One":["free text"]}})
        );
        assert_eq!(
            json!({"jsonrpc":"2.0","id":response_id,"result":result}),
            json!({"jsonrpc":"2.0","id":501,"result":{"outcome":"accepted","answers":{"Choose":["A","B"],"One":["free text"]}}})
        );
        assert!(
            pending
                .answer_question(
                    json!(501),
                    "sid",
                    "tool-1",
                    QuestionReplyOutcome::Accepted,
                    Some(answers.clone())
                )
                .is_err()
        );

        let single = parse_request(&json!({"id":"string-id","method":"_x.ai/ask_user_question","params":{
            "sessionId":"sid","toolCallId":"single","mode":"default","questions":[{"question":"Q","options":[],"multiSelect":false}]
        }}), "sid").unwrap().unwrap();
        pending.insert(single).unwrap();
        let mut bad = BTreeMap::new();
        bad.insert("Q".to_owned(), vec!["A".to_owned(), "B".to_owned()]);
        assert!(
            pending
                .answer_question(
                    json!("string-id"),
                    "sid",
                    "single",
                    QuestionReplyOutcome::Accepted,
                    Some(bad)
                )
                .is_err()
        );
        let mut free = BTreeMap::new();
        free.insert("Q".to_owned(), vec!["user-provided text".to_owned()]);
        assert_eq!(
            pending
                .answer_question(
                    json!("string-id"),
                    "sid",
                    "single",
                    QuestionReplyOutcome::Accepted,
                    Some(free)
                )
                .unwrap()
                .0,
            json!("string-id")
        );
    }

    #[test]
    fn rejects_cross_session_or_unknown_replies_and_clears_resolved_requests() {
        let request = parse_request(
            &wrapped(
                "_x.ai/exit_plan_mode",
                json!({"sessionId":"sid","toolCallId":"plan"}),
                json!(9),
            ),
            "sid",
        )
        .unwrap()
        .unwrap();
        let mut pending = Outstanding::default();
        pending.insert(request).unwrap();
        assert!(
            pending
                .answer_plan(json!(9), "other", "plan", PlanReplyOutcome::Approved, None)
                .is_err()
        );
        assert!(
            pending
                .answer_plan(json!(10), "sid", "plan", PlanReplyOutcome::Approved, None)
                .is_err()
        );
        assert_eq!(
            pending
                .answer_plan(
                    json!(9),
                    "sid",
                    "plan",
                    PlanReplyOutcome::Cancelled,
                    Some("change this".into())
                )
                .unwrap(),
            (
                json!(9),
                json!({"outcome":"cancelled","feedback":"change this"})
            )
        );
        assert!(
            pending
                .answer_plan(json!(9), "sid", "plan", PlanReplyOutcome::Approved, None)
                .is_err()
        );

        pending
            .insert(
                parse_request(
                    &wrapped(
                        "_x.ai/exit_plan_mode",
                        json!({"sessionId":"sid","toolCallId":"resolved"}),
                        json!(11),
                    ),
                    "sid",
                )
                .unwrap()
                .unwrap(),
            )
            .unwrap();
        pending.remove_tool_call("resolved");
        assert!(
            pending
                .answer_plan(
                    json!(11),
                    "sid",
                    "resolved",
                    PlanReplyOutcome::Approved,
                    None
                )
                .is_err()
        );
    }
}
