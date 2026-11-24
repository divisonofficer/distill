pub(crate) use crate::research_sessions::NodeEndpoint;
use std::io;

use anyhow::{Context, bail};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

#[cfg(unix)]
use tokio::net::UnixStream;

const MAX_MESSAGE: usize = 1024 * 1024;
const MAX_RESPONSE: usize = 1024 * 1024;
const MCP_PATH: &str = "/mcp/research";

#[derive(Deserialize)]
struct RpcRequest {
    jsonrpc: String,
    #[serde(default)]
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Serialize)]
struct RpcError {
    code: i64,
    message: &'static str,
}

pub(crate) async fn run_stdio(endpoint: NodeEndpoint, grant_id: String) -> anyhow::Result<()> {
    let token =
        std::env::var("POSTUDIO_RESEARCH_TOKEN").context("POSTUDIO_RESEARCH_TOKEN is required")?;
    if token.is_empty() || grant_id.is_empty() {
        bail!("research MCP grant credentials are empty");
    }
    #[expect(
        clippy::disallowed_methods,
        reason = "talks only to this node's loopback API (the node passes its own listener); proxies and redirects are disabled"
    )]
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(30))
        .build()?;
    let mut input = BufReader::new(tokio::io::stdin());
    let mut output = tokio::io::stdout();
    let mut line = Vec::new();
    loop {
        line.clear();
        let mut ended = false;
        while line.len() <= MAX_MESSAGE {
            let available = input.fill_buf().await?;
            if available.is_empty() {
                ended = true;
                break;
            }
            let count = available
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(available.len(), |index| index + 1);
            line.extend_from_slice(&available[..count]);
            input.consume(count);
            if line.last() == Some(&b'\n') {
                break;
            }
        }
        if ended && line.is_empty() {
            break;
        }
        if line.len() > MAX_MESSAGE {
            write_error(&mut output, None, -32700, "Message exceeds size limit").await?;
            break;
        }
        if line.last() == Some(&b'\n') {
            line.pop();
        }
        if line.last() == Some(&b'\r') {
            line.pop();
        }
        let req = match serde_json::from_slice::<RpcRequest>(&line) {
            Ok(req) if req.jsonrpc == "2.0" => req,
            _ => {
                write_error(&mut output, None, -32700, "Invalid JSON-RPC request").await?;
                continue;
            }
        };
        if req.id.is_none() {
            continue;
        }
        let response_id = req.id.clone();
        let response = match req.method.as_str() {
            "notifications/initialized" => continue,
            "ping" => Ok(json!({"jsonrpc":"2.0","id":req.id,"result":{}})),
            "initialize" | "tools/list" | "tools/call" => {
                let payload = serde_json::to_vec(
                    &json!({"jsonrpc":"2.0","id":req.id,"method":req.method,"params":req.params}),
                )?;
                forward(&client, &endpoint, &grant_id, &token, payload).await
            }
            _ => Ok(rpc_error(req.id, -32601, "Method not found")),
        };
        let value = response
            .unwrap_or_else(|_| rpc_error(response_id, -32603, "Research MCP transport failed"));
        let bytes = serde_json::to_vec(&value)?;
        if bytes.len() > MAX_RESPONSE {
            write_error(&mut output, None, -32603, "Response exceeds size limit").await?;
        } else {
            output.write_all(&bytes).await?;
            output.write_all(b"\n").await?;
            output.flush().await?;
        }
    }
    Ok(())
}

async fn forward(
    client: &reqwest::Client,
    endpoint: &NodeEndpoint,
    grant: &str,
    token: &str,
    payload: Vec<u8>,
) -> anyhow::Result<Value> {
    match endpoint {
        NodeEndpoint::Http(base) => {
            let url = format!("{}{}", base.trim_end_matches('/'), MCP_PATH);
            let response = client
                .post(url)
                .header("authorization", format!("Bearer {token}"))
                .header("x-postudio-research-grant", grant)
                .header("content-type", "application/json")
                .body(payload)
                .send()
                .await?;
            if response
                .content_length()
                .is_some_and(|length| length > MAX_RESPONSE as u64)
            {
                bail!("MCP response exceeds size limit");
            }
            let mut stream = response.bytes_stream();
            let mut bytes = Vec::new();
            while let Some(chunk) = futures_util::StreamExt::next(&mut stream).await {
                let chunk = chunk?;
                if bytes.len() + chunk.len() > MAX_RESPONSE {
                    bail!("MCP response exceeds size limit");
                }
                bytes.extend_from_slice(&chunk);
            }
            Ok(serde_json::from_slice(&bytes)?)
        }
        #[cfg(unix)]
        NodeEndpoint::Unix(path) => {
            let mut stream = UnixStream::connect(path).await?;
            let request = format!(
                "POST {MCP_PATH} HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {token}\r\nX-Postudio-Research-Grant: {grant}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                payload.len()
            );
            stream.write_all(request.as_bytes()).await?;
            stream.write_all(&payload).await?;
            let mut response = Vec::new();
            stream
                .take((MAX_RESPONSE + 8192) as u64)
                .read_to_end(&mut response)
                .await?;
            let split = response
                .windows(4)
                .position(|w| w == b"\r\n\r\n")
                .context("Invalid HTTP response")?
                + 4;
            let header = std::str::from_utf8(&response[..split])?;
            if header
                .to_ascii_lowercase()
                .contains("transfer-encoding: chunked")
            {
                bail!("chunked Unix responses are unsupported");
            }
            let expected = header
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|v| v.trim().parse::<usize>().ok())
                })
                .context("Missing content length")?;
            let body = &response[split..];
            if expected > MAX_RESPONSE || body.len() != expected {
                bail!("Invalid Unix response length");
            }
            Ok(serde_json::from_slice(body)?)
        }
        #[cfg(not(unix))]
        NodeEndpoint::Unix(_) => bail!("Unix sockets unavailable"),
    }
}

fn rpc_error(id: Option<Value>, code: i64, message: &'static str) -> Value {
    let mut response = json!({"jsonrpc":"2.0","error":RpcError{code,message}});
    if let Some(id) = id {
        response["id"] = id;
    }
    response
}

async fn write_error<W: AsyncWriteExt + Unpin>(
    out: &mut W,
    id: Option<Value>,
    code: i64,
    message: &'static str,
) -> io::Result<()> {
    let data = serde_json::to_vec(&rpc_error(id, code, message)).unwrap_or_default();
    out.write_all(&data).await?;
    out.write_all(b"\n").await?;
    out.flush().await
}

pub(crate) fn private_routes(
    service: crate::research::ResearchService,
    bindings: crate::research_sessions::ResearchBindings,
) -> Router {
    Router::new()
        .route(MCP_PATH, post(private_handler))
        .layer(DefaultBodyLimit::max(MAX_MESSAGE))
        .with_state((service, bindings))
}

async fn private_handler(
    State((service, bindings)): State<(
        crate::research::ResearchService,
        crate::research_sessions::ResearchBindings,
    )>,
    headers: HeaderMap,
    Json(request): Json<Value>,
) -> Response {
    let id = request.get("id").cloned();
    let fail = |code, message| Json(rpc_error(id.clone(), code, message)).into_response();
    if request["jsonrpc"] != "2.0" || !request["method"].is_string() {
        return fail(-32600, "Invalid request");
    }
    let method = request["method"].as_str().unwrap_or_default();
    let grant = match headers
        .get("x-postudio-research-grant")
        .and_then(|v| v.to_str().ok())
    {
        Some(v) => v,
        None => return fail(-32600, "Invalid grant"),
    };
    let token = match headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
    {
        Some(v) => v,
        None => return fail(-32600, "Invalid grant"),
    };
    let allow_pending = matches!(
        method,
        "initialize" | "notifications/initialized" | "tools/list" | "ping"
    );
    let scope = match bindings.authorize(grant, token, allow_pending).await {
        Ok(scope) => scope,
        Err(_) => return fail(-32600, "Unauthorized research grant"),
    };
    let result = match method {
        "initialize" => {
            json!({"protocolVersion":request["params"]["protocolVersion"],"capabilities":{"tools":{}},"serverInfo":{"name":"postudio-research","version":"1"}})
        }
        "notifications/initialized" => return StatusCode::ACCEPTED.into_response(),
        "ping" => json!({}),
        "tools/list" => json!({"tools":tool_definitions()}),
        "tools/call" => match call_tool(&service, &scope, &request["params"]).await {
            Ok(value) => value,
            Err((code, message)) => return fail(code, message),
        },
        _ => return fail(-32601, "Method not found"),
    };
    if id.is_none() {
        return StatusCode::ACCEPTED.into_response();
    }
    Json(json!({"jsonrpc":"2.0","id":id,"result":result})).into_response()
}

fn tool_definitions() -> Value {
    json!({"tools":[
        {"name":"list_experiments","description":"List experiments in this project.","inputSchema":{"type":"object","properties":{},"additionalProperties":false}},
        {"name":"create_experiment","description":"Create experiment inside this project.","inputSchema":{"type":"object","required":["title"],"properties":{"title":{"type":"string"},"description":{"type":"string"},"runCommand":{"type":"string"},"parentId":{"type":"string"}},"additionalProperties":false}},
        {"name":"request_run","description":"Request owner approval to run this scoped experiment. Never starts run.","inputSchema":{"type":"object","required":["expectedCommit","targetId"],"properties":{"expectedCommit":{"type":"string"},"targetId":{"type":"string"}},"additionalProperties":false}},
        {"name":"get_run","description":"Get run by id in this project.","inputSchema":{"type":"object","required":["runId"],"properties":{"runId":{"type":"string"}},"additionalProperties":false}},
        {"name":"get_request","description":"Get owner approval request.","inputSchema":{"type":"object","required":["approvalId"],"properties":{"approvalId":{"type":"string"}},"additionalProperties":false}},
        {"name":"metrics","description":"Get actual metrics for a scoped run.","inputSchema":{"type":"object","required":["runId"],"properties":{"runId":{"type":"string"}},"additionalProperties":false}},
        {"name":"link_references","description":"Link existing literature references to scoped experiment.","inputSchema":{"type":"object","required":["literatureIds"],"properties":{"literatureIds":{"type":"array","items":{"type":"string"}}},"additionalProperties":false}},
        {"name":"search_papers","description":"Search scholarly literature: alphaXiv full-text keyword (`keyword`, default) or semantic (`embedding`) retrieval, OpenAlex (`openalex`), bioRxiv (`biorxiv`) or PubMed (`pubmed`). Read the `orx-lit-review` skill first for the retrieval policy, then rank candidates yourself and follow up with get_paper.","inputSchema":{"type":"object","required":["query"],"properties":{"query":{"type":"string","maxLength":512},"strategy":{"type":"string","enum":crate::research_library::STRATEGIES},"limit":{"type":"integer","minimum":1,"maximum":25}},"additionalProperties":false}},
        {"name":"get_paper","description":"Read one paper: the alphaXiv report (or full extracted text with full=true) for arXiv ids/URLs, or OpenAlex/bioRxiv/PubMed metadata and abstract. The source is detected from the id unless given. Long text is paged: pass the returned nextOffset as offset to continue.","inputSchema":{"type":"object","required":["id"],"properties":{"id":{"type":"string","maxLength":256},"source":{"type":"string","enum":crate::research_library::PAPER_SOURCES},"full":{"type":"boolean"},"offset":{"type":"integer","minimum":0}},"additionalProperties":false}},
        {"name":"list_skills","description":"List OpenResearch skills (research procedures): the overview plus bundled `orx-*` modules with one-line descriptions. Load the matching skill with read_skill before acting in its area.","inputSchema":{"type":"object","properties":{},"additionalProperties":false}},
        {"name":"read_skill","description":"Read an OpenResearch skill: `overview`, a module such as `orx-lit-review`, or a module resource such as `orx-compute/hf`.","inputSchema":{"type":"object","required":["name"],"properties":{"name":{"type":"string","maxLength":160}},"additionalProperties":false}}
    ]})["tools"].clone()
}

fn tool_error(code: i64, message: &'static str) -> (i64, &'static str) {
    (code, message)
}

async fn call_tool(
    service: &crate::research::ResearchService,
    scope: &crate::research_sessions::ResearchScope,
    params: &Value,
) -> Result<Value, (i64, &'static str)> {
    let name = params["name"]
        .as_str()
        .ok_or_else(|| tool_error(-32602, "Tool name required"))?;
    let args = &params["arguments"];
    if let Some(result) = library_tool(service, name, args).await {
        return result;
    }
    let marker = |kind: &str, extra: Value| {
        let mut card = json!({"projectId":scope.project_id,"experimentId":extra.pointer("/experiment/id").and_then(Value::as_str).unwrap_or(&scope.experiment_id),"kind":kind});
        if let Some(id) = extra
            .pointer("/approval/id")
            .or_else(|| extra.pointer("/approval/approvalId"))
        {
            card["approvalId"] = id.clone();
        }
        if let Some(id) = extra.pointer("/run/id") {
            card["runId"] = id.clone();
        }
        if let Some(title) = extra.pointer("/experiment/title") {
            card["title"] = title.clone();
        }
        json!({"postudioResearch":card})
    };
    let data = match name {
        "list_experiments" => {
            strict_args(args, &[])?;
            let p = service
                .native_json(
                    axum::http::Method::GET,
                    &format!(
                        "api/projects/{}/experiments",
                        urlencoding::encode(&scope.project_id)
                    ),
                    None,
                )
                .await
                .map_err(|_| tool_error(-32000, "Project unavailable"))?;
            json!({"experiments":p["experiments"]})
        }
        "create_experiment" => {
            strict_args(args, &["title", "description", "runCommand", "parentId"])?;
            let title = args["title"]
                .as_str()
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| tool_error(-32602, "title required"))?;
            if args["parentId"]
                .as_str()
                .is_some_and(|id| id != scope.experiment_id)
            {
                return Err(tool_error(-32602, "Parent outside grant scope"));
            }
            let before = service
                .native_json(
                    axum::http::Method::GET,
                    &format!(
                        "api/projects/{}/experiments",
                        urlencoding::encode(&scope.project_id)
                    ),
                    None,
                )
                .await
                .map_err(|_| tool_error(-32000, "Project unavailable"))?;
            let created = service.create_experiment(&scope.project_id, json!({"title":title,"description":args["description"],"runCommand":args["runCommand"],"parentId":scope.experiment_id})).await.map_err(|_| tool_error(-32000,"Experiment creation failed"))?;
            let experiment = created["experiments"]
                .as_array()
                .and_then(|items| {
                    items.iter().find(|new| {
                        new["title"] == title
                            && !before["experiments"]
                                .as_array()
                                .is_some_and(|old| old.iter().any(|x| x["id"] == new["id"]))
                    })
                })
                .cloned()
                .ok_or_else(|| tool_error(-32000, "Created experiment could not be identified"))?;
            json!({"experiment":experiment})
        }
        "request_run" => {
            strict_args(args, &["expectedCommit", "targetId", "approved"])?;
            if args.get("approved").is_some() {
                return Err(tool_error(-32602, "MCP cannot approve runs"));
            }
            let expected = args["expectedCommit"]
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| tool_error(-32602, "expectedCommit required"))?;
            let target = args["targetId"]
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| tool_error(-32602, "targetId required"))?;
            let approval = service
                .request_run_approval(
                    &scope.project_id,
                    &scope.experiment_id,
                    scope.session_id.as_deref(),
                    target,
                    expected,
                )
                .await
                .map_err(|_| tool_error(-32000, "Approval request failed"))?;
            json!({"approval":approval["approval"]})
        }
        "get_run" => {
            strict_args(args, &["runId"])?;
            let run_id = args["runId"]
                .as_str()
                .ok_or_else(|| tool_error(-32602, "runId required"))?;
            let run = service
                .owned_run(&scope.project_id, run_id)
                .await
                .map_err(|_| tool_error(-32000, "Run unavailable"))?;
            if run["experimentId"].as_str() != Some(&scope.experiment_id) {
                return Err(tool_error(-32000, "Run outside grant scope"));
            }
            json!({"run":run})
        }
        "get_request" => {
            strict_args(args, &["approvalId"])?;
            let approval_id = args["approvalId"]
                .as_str()
                .ok_or_else(|| tool_error(-32602, "approvalId required"))?;
            let refs = service
                .refs_get(&scope.project_id)
                .await
                .map_err(|_| tool_error(-32000, "Request unavailable"))?;
            let approval = refs
                .approvals
                .iter()
                .find(|a| a.id == approval_id && a.experiment_id == scope.experiment_id)
                .ok_or_else(|| tool_error(-32000, "Request unavailable"))?;
            let mut result = json!({"approval":approval});
            if let Some(run_id) = approval.run_id.as_deref() {
                result["run"] = service
                    .owned_run(&scope.project_id, run_id)
                    .await
                    .map_err(|_| tool_error(-32000, "Run unavailable"))?;
            }
            result
        }
        "metrics" => {
            strict_args(args, &["runId"])?;
            let run_id = args["runId"]
                .as_str()
                .ok_or_else(|| tool_error(-32602, "runId required"))?;
            let run = service
                .owned_run(&scope.project_id, run_id)
                .await
                .map_err(|_| tool_error(-32000, "Run unavailable"))?;
            if run["experimentId"].as_str() != Some(&scope.experiment_id) {
                return Err(tool_error(-32000, "Run outside grant scope"));
            }
            let metrics = service
                .metrics(&scope.project_id, run_id)
                .await
                .map_err(|_| tool_error(-32000, "Metrics unavailable"))?;
            json!({"metrics":metrics})
        }
        "link_references" => {
            strict_args(args, &["literatureIds"])?;
            let ids = args["literatureIds"]
                .as_array()
                .filter(|a| a.len() <= 100)
                .ok_or_else(|| tool_error(-32602, "literatureIds required"))?;
            let refs = service
                .refs_modify(&scope.project_id, |refs| {
                    let exp = refs
                        .experiments
                        .entry(scope.experiment_id.clone())
                        .or_default();
                    for id in ids {
                        let id = id.as_str().ok_or("invalid literature ID")?;
                        if !refs.literature.iter().any(|p| p.id == id) {
                            return Err("unknown literature reference".into());
                        }
                        if !exp.literature_ids.iter().any(|existing| existing == id) {
                            exp.literature_ids.push(id.into());
                        }
                    }
                    Ok(())
                })
                .await
                .map_err(|_| tool_error(-32000, "Reference linking failed"))?;
            json!({"refs":refs})
        }
        _ => return Err(tool_error(-32601, "Unknown research tool")),
    };
    let marked = marker("tool", data);
    let text = serde_json::to_string(&marked).unwrap_or_else(|_| "{}".into());
    Ok(json!({"content":[{"type":"text","text":text}],"structuredContent":marked,"isError":false}))
}

/// Literature and skill tools: read-only, not tied to the grant's project, so their
/// results carry no research card marker.
async fn library_tool(
    service: &crate::research::ResearchService,
    name: &str,
    args: &Value,
) -> Option<Result<Value, (i64, &'static str)>> {
    use crate::research_library::{POSTUDIO_SKILL_NOTE, skill_index};
    let orx = |error: crate::research::OrxCallError| match error {
        crate::research::OrxCallError::Invalid => tool_error(-32602, error.message()),
        _ => tool_error(-32000, error.message()),
    };
    let run = async {
        let data = match name {
            "search_papers" => {
                strict_args(args, &["query", "strategy", "limit"])?;
                let query = args["query"]
                    .as_str()
                    .ok_or_else(|| tool_error(-32602, "query required"))?;
                let strategy = match &args["strategy"] {
                    Value::Null => "keyword",
                    value => value
                        .as_str()
                        .ok_or_else(|| tool_error(-32602, "strategy must be a string"))?,
                };
                let limit = match &args["limit"] {
                    Value::Null => 10,
                    value => value
                        .as_u64()
                        .and_then(|n| u8::try_from(n).ok())
                        .ok_or_else(|| tool_error(-32602, "limit must be 1-25"))?,
                };
                service
                    .search_literature(strategy, query, limit)
                    .await
                    .map_err(orx)?
            }
            "get_paper" => {
                strict_args(args, &["id", "source", "full", "offset"])?;
                let id = args["id"]
                    .as_str()
                    .ok_or_else(|| tool_error(-32602, "id required"))?;
                let source = match &args["source"] {
                    Value::Null => None,
                    value => Some(
                        value
                            .as_str()
                            .ok_or_else(|| tool_error(-32602, "source must be a string"))?,
                    ),
                };
                let full = match &args["full"] {
                    Value::Null => false,
                    value => value
                        .as_bool()
                        .ok_or_else(|| tool_error(-32602, "full must be a boolean"))?,
                };
                let offset = match &args["offset"] {
                    Value::Null => 0,
                    value => value
                        .as_u64()
                        .and_then(|n| usize::try_from(n).ok())
                        .ok_or_else(|| {
                            tool_error(-32602, "offset must be a non-negative integer")
                        })?,
                };
                let page = service
                    .paper_page(id, source, full, offset)
                    .await
                    .map_err(orx)?;
                json!({"id":id,"source":source,"full":full,"offset":page.offset,"nextOffset":page.next_offset,"totalChars":page.total_chars,"text":page.text})
            }
            "list_skills" => {
                strict_args(args, &[])?;
                let overview = service.skill_text(None).await.map_err(orx)?;
                let mut skills = vec![
                    json!({"name":"overview","description":"OpenResearch cardinal rules, command map and typical workflow. Read first."}),
                ];
                skills.extend(
                    skill_index(&overview)
                        .into_iter()
                        .map(|(name, description)| json!({"name":name,"description":description})),
                );
                json!({"skills":skills})
            }
            "read_skill" => {
                strict_args(args, &["name"])?;
                let skill = args["name"]
                    .as_str()
                    .ok_or_else(|| tool_error(-32602, "name required"))?;
                let text = service.skill_text(Some(skill)).await.map_err(orx)?;
                json!({"name":skill,"text":format!("{POSTUDIO_SKILL_NOTE}{text}")})
            }
            _ => return Ok(None),
        };
        Ok(Some(data))
    };
    match run.await {
        Ok(Some(data)) => {
            let text = serde_json::to_string(&data).unwrap_or_else(|_| "{}".into());
            Some(Ok(
                json!({"content":[{"type":"text","text":text}],"structuredContent":data,"isError":false}),
            ))
        }
        Ok(None) => None,
        Err(error) => Some(Err(error)),
    }
}

fn strict_args(args: &Value, allowed: &[&str]) -> Result<(), (i64, &'static str)> {
    let object = args
        .as_object()
        .ok_or_else(|| tool_error(-32602, "arguments must be object"))?;
    if object.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(tool_error(-32602, "unsupported argument"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rpc_errors_have_jsonrpc_envelope() {
        let value = rpc_error(Some(json!(7)), -32601, "Method not found");
        assert_eq!(value["jsonrpc"], "2.0");
        assert_eq!(value["id"], 7);
        assert_eq!(value["error"]["code"], -32601);
    }

    #[cfg(unix)]
    const FAKE_ORX: &str = r#"#!/bin/sh
# Fake `orx` for library tool tests. Args: --no-telemetry <command> ...
shift
echo "$1" >> "$HOME/calls.log"
case "$1" in
  discover)
    # discover <strategy> --limit <n> -- <query>
    printf '[{"source":"alphaxiv","id":"2611.00001","title":"%s|%s|%s","abstract":"A","publicationDate":"2026-01-01","snippets":["s"],"votes":1}]' "$2" "$4" "$6"
    ;;
  paper)
    printf 'ARGS:%s\n' "$*"
    head -c 30000 /dev/zero | tr '\000' 'x'
    ;;
  skill)
    if [ -z "$2" ]; then
      printf '# OpenResearch CLI\n\nBundled modules (orx skill <name>):\n  orx-create           Initialize a project.\n  orx-lit-review       Explain concepts with evidence.\n'
    else
      printf 'MODULE:%s\n' "$2"
    fi
    ;;
  *) exit 3 ;;
esac
"#;

    #[cfg(unix)]
    fn fake_service() -> (tempfile::TempDir, crate::research::ResearchService) {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("orx");
        std::fs::write(&bin, FAKE_ORX).unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let service = crate::research::ResearchService::new(
            Some(bin),
            dir.path().join("state"),
            crate::ports::port_policy([]),
        );
        (dir, service)
    }

    fn scope() -> crate::research_sessions::ResearchScope {
        crate::research_sessions::ResearchScope {
            project_id: "p".into(),
            experiment_id: "e".into(),
            session_id: None,
            grant_id: "g".into(),
        }
    }

    async fn call(
        service: &crate::research::ResearchService,
        name: &str,
        arguments: Value,
    ) -> Result<Value, (i64, &'static str)> {
        call_tool(
            service,
            &scope(),
            &json!({"name":name,"arguments":arguments}),
        )
        .await
    }

    #[test]
    fn tool_list_includes_library_tools() {
        let names: Vec<String> = tool_definitions()
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_owned())
            .collect();
        for name in ["search_papers", "get_paper", "list_skills", "read_skill"] {
            assert!(names.iter().any(|n| n == name), "{name}");
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn search_papers_passes_strategy_limit_and_query_after_separator() {
        let (_dir, service) = fake_service();
        let result = call(
            &service,
            "search_papers",
            json!({"query":"--full gaussian"}),
        )
        .await
        .unwrap();
        let data = &result["structuredContent"];
        assert_eq!(data["strategy"], "keyword");
        assert_eq!(data["count"], 1);
        assert_eq!(data["papers"][0]["title"], "keyword|10|--full gaussian");
        assert!(
            data.get("postudioResearch").is_none(),
            "library results carry no research card"
        );
        let embedding = call(
            &service,
            "search_papers",
            json!({"query":"splats","strategy":"embedding","limit":3}),
        )
        .await
        .unwrap();
        assert_eq!(
            embedding["structuredContent"]["papers"][0]["title"],
            "embedding|3|splats"
        );
        for bad in [
            json!({"query":"q","strategy":"google"}),
            json!({"query":"q","limit":0}),
            json!({"query":"q","limit":26}),
            json!({"query":"  "}),
            json!({"query":"q","extra":1}),
        ] {
            assert_eq!(
                call(&service, "search_papers", bad.clone())
                    .await
                    .unwrap_err()
                    .0,
                -32602,
                "{bad}"
            );
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn get_paper_pages_from_one_fetch() {
        let (dir, service) = fake_service();
        let first = call(
            &service,
            "get_paper",
            json!({"id":"2611.00001","source":"alphaxiv","full":true}),
        )
        .await
        .unwrap();
        let first = &first["structuredContent"];
        let text = first["text"].as_str().unwrap();
        assert!(
            text.starts_with("ARGS:paper --source alphaxiv --full -- 2611.00001\n"),
            "{}",
            &text[..80]
        );
        assert_eq!(
            text.chars().count(),
            crate::research_library::PAPER_PAGE_CHARS
        );
        let next = first["nextOffset"].as_u64().unwrap();
        let second = call(
            &service,
            "get_paper",
            json!({"id":"2611.00001","source":"alphaxiv","full":true,"offset":next}),
        )
        .await
        .unwrap();
        let second = &second["structuredContent"];
        assert!(second["nextOffset"].is_null());
        assert_eq!(
            next + second["text"].as_str().unwrap().chars().count() as u64,
            first["totalChars"].as_u64().unwrap()
        );
        let calls = std::fs::read_to_string(
            dir.path()
                .join("state/research/openresearch/home/calls.log"),
        )
        .unwrap();
        assert_eq!(
            calls.lines().filter(|l| *l == "paper").count(),
            1,
            "second page comes from the cache"
        );
        for bad in [
            json!({"id":"-x"}),
            json!({"id":"a b"}),
            json!({"id":"1","source":"scholar"}),
            json!({"id":"1","offset":-1}),
        ] {
            assert_eq!(
                call(&service, "get_paper", bad.clone())
                    .await
                    .unwrap_err()
                    .0,
                -32602,
                "{bad}"
            );
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn skills_list_and_read_with_postudio_note() {
        let (_dir, service) = fake_service();
        let list = call(&service, "list_skills", json!({})).await.unwrap();
        let names: Vec<&str> = list["structuredContent"]["skills"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["overview", "orx-create", "orx-lit-review"]);
        let skill = call(&service, "read_skill", json!({"name":"orx-lit-review"}))
            .await
            .unwrap();
        let text = skill["structuredContent"]["text"].as_str().unwrap();
        assert!(text.starts_with(crate::research_library::POSTUDIO_SKILL_NOTE));
        assert!(text.ends_with("MODULE:orx-lit-review\n"));
        let overview = call(&service, "read_skill", json!({"name":"overview"}))
            .await
            .unwrap();
        assert!(
            overview["structuredContent"]["text"]
                .as_str()
                .unwrap()
                .contains("Bundled modules")
        );
        for bad in ["../etc/passwd", "orx-a/../b", "lit-review"] {
            assert_eq!(
                call(&service, "read_skill", json!({"name":bad}))
                    .await
                    .unwrap_err()
                    .0,
                -32602,
                "{bad}"
            );
        }
    }

    #[tokio::test]
    async fn library_tools_report_missing_openresearch() {
        let dir = tempfile::tempdir().unwrap();
        let service = crate::research::ResearchService::new(
            None,
            dir.path().to_path_buf(),
            crate::ports::port_policy([]),
        );
        let error = call(&service, "search_papers", json!({"query":"q"}))
            .await
            .unwrap_err();
        assert_eq!(error, (-32000, "OpenResearch not configured"));
    }

    #[test]
    fn rpc_request_rejects_wrong_version() {
        let parsed =
            serde_json::from_str::<RpcRequest>(r#"{"jsonrpc":"1.0","method":"ping"}"#).unwrap();
        assert_ne!(parsed.jsonrpc, "2.0");
    }
}
