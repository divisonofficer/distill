//! Literature and skill access for agents: OpenResearch's `orx discover`, `orx paper` and
//! `orx skill`, run read-only in the node's isolated research home.
//! Exposed to agent sessions as research MCP tools (see `research_mcp.rs`).

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::research::{OrxCallError, ResearchService};

/// `orx discover` strategies, in the order the lit-review skill introduces them.
pub(crate) const STRATEGIES: [&str; 5] = ["keyword", "embedding", "openalex", "biorxiv", "pubmed"];
/// `orx paper --source` values.
pub(crate) const PAPER_SOURCES: [&str; 4] = ["alphaxiv", "openalex", "biorxiv", "pubmed"];
/// Characters of paper text returned per `get_paper` call.
pub(crate) const PAPER_PAGE_CHARS: usize = 24_000;

const SEARCH_LIMIT_BYTES: usize = 1024 * 1024;
const PAPER_LIMIT_BYTES: usize = 8 * 1024 * 1024;
const SKILL_LIMIT_BYTES: usize = 512 * 1024;
const ABSTRACT_CHARS: usize = 1_200;
const SNIPPET_CHARS: usize = 300;
const SNIPPETS: usize = 3;
const PAPER_CACHE_ENTRIES: usize = 8;
const PAPER_CACHE_TTL: Duration = Duration::from_secs(15 * 60);

/// Prepended to every skill text an agent reads: the modules document the `orx` CLI,
/// which agents in Postudio reach only through the research tools.
pub(crate) const POSTUDIO_SKILL_NOTE: &str = "Postudio note: this OpenResearch module documents the `orx` CLI. In Postudio, do not run `orx` yourself. Use `search_papers` for `orx discover …`, `get_paper` for `orx paper …`, `read_skill` for `orx skill …`, and the project tools (`list_experiments`, `create_experiment`, `request_run`, `get_run`, `metrics`, `link_references`) for experiment work when they are available. A run starts only after the owner approves `request_run`. If a command has no matching tool here, say that it is unavailable instead of improvising.\n\n";

/// `(key, fetched at, text)`.
type PaperEntry = (String, Instant, Arc<str>);

/// Short-lived cache so paging through one paper does not refetch it every call.
#[derive(Clone, Default)]
pub(crate) struct PaperCache {
    entries: Arc<Mutex<Vec<PaperEntry>>>,
}

impl PaperCache {
    fn get(&self, key: &str) -> Option<Arc<str>> {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        entries.retain(|(_, at, _)| at.elapsed() < PAPER_CACHE_TTL);
        entries
            .iter()
            .find(|(k, _, _)| k == key)
            .map(|(_, _, text)| text.clone())
    }
    fn put(&self, key: String, text: Arc<str>) {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        entries.retain(|(k, at, _)| k != &key && at.elapsed() < PAPER_CACHE_TTL);
        if entries.len() >= PAPER_CACHE_ENTRIES {
            entries.remove(0);
        }
        entries.push((key, Instant::now(), text));
    }
}

/// One page of a paper's text, addressed by character offset.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PaperPage {
    pub text: String,
    pub offset: usize,
    pub next_offset: Option<usize>,
    pub total_chars: usize,
}

pub(crate) fn valid_query(query: &str) -> bool {
    !query.trim().is_empty() && query.len() <= 512 && !query.chars().any(char::is_control)
}

/// arXiv ids/URLs, DOIs, OpenAlex `W…` ids and PMIDs: no whitespace or control characters.
pub(crate) fn valid_paper_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 256
        && !id.starts_with('-')
        && !id.chars().any(|c| c.is_whitespace() || c.is_control())
}

/// `overview`, a bundled module (`orx-lit-review`), or a module resource (`orx-compute/hf`).
pub(crate) fn valid_skill_name(name: &str) -> bool {
    if name == "overview" {
        return true;
    }
    let mut parts = name.split('/');
    let module_ok = parts.next().is_some_and(|module| {
        module.len() <= 48
            && module.strip_prefix("orx-").is_some_and(|rest| {
                !rest.is_empty()
                    && rest
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            })
    });
    module_ok
        && name.len() <= 160
        && parts.all(|part| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && part.len() <= 64
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
                && part.as_bytes()[0].is_ascii_alphanumeric()
        })
}

fn truncate(text: &str, chars: usize) -> String {
    match text.char_indices().nth(chars) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text.to_owned(),
    }
}

/// Keep what an agent needs to rank candidates; cap the long fields.
pub(crate) fn compact_paper(item: &Value) -> Value {
    let text = |key: &str, chars: usize| {
        item[key]
            .as_str()
            .map(|s| Value::String(truncate(s, chars)))
            .unwrap_or(Value::Null)
    };
    let snippets: Vec<Value> = item["snippets"]
        .as_array()
        .into_iter()
        .flatten()
        .take(SNIPPETS)
        .map(|s| match s {
            Value::String(s) => Value::String(truncate(s, SNIPPET_CHARS)),
            other => Value::String(truncate(&other.to_string(), SNIPPET_CHARS)),
        })
        .collect();
    let mut out = json!({
        "source": item["source"],
        "id": item["id"],
        "title": text("title", 400),
        "publicationDate": item["publicationDate"],
        "abstract": text("abstract", ABSTRACT_CHARS),
    });
    if !snippets.is_empty() {
        out["snippets"] = Value::Array(snippets);
    }
    if !item["votes"].is_null() {
        out["votes"] = item["votes"].clone();
    }
    out
}

/// The "Bundled modules" index that ends `orx skill` output: `(name, description)` pairs.
pub(crate) fn skill_index(overview: &str) -> Vec<(String, String)> {
    let mut in_index = false;
    let mut modules = Vec::new();
    for line in overview.lines() {
        if line.trim_start().starts_with("Bundled modules") {
            in_index = true;
            continue;
        }
        if !in_index {
            continue;
        }
        let trimmed = line.trim();
        let Some((name, description)) = trimmed.split_once(char::is_whitespace) else {
            continue;
        };
        if valid_skill_name(name) && name != "overview" && !name.contains('/') {
            modules.push((name.to_owned(), description.trim().to_owned()));
        }
    }
    modules
}

pub(crate) fn page(text: &str, offset: usize) -> PaperPage {
    let total_chars = text.chars().count();
    let start = text
        .char_indices()
        .nth(offset)
        .map_or(text.len(), |(i, _)| i);
    let rest = &text[start..];
    let end = rest
        .char_indices()
        .nth(PAPER_PAGE_CHARS)
        .map_or(rest.len(), |(i, _)| i);
    let shown = rest[..end].chars().count();
    let next = offset.saturating_add(shown);
    PaperPage {
        text: rest[..end].to_owned(),
        offset: offset.min(total_chars),
        next_offset: (next < total_chars).then_some(next),
        total_chars,
    }
}

impl ResearchService {
    /// `orx discover <strategy>`: compact paper candidates.
    pub(crate) async fn search_literature(
        &self,
        strategy: &str,
        query: &str,
        limit: u8,
    ) -> Result<Value, OrxCallError> {
        if !STRATEGIES.contains(&strategy) || !valid_query(query) || !(1..=25).contains(&limit) {
            return Err(OrxCallError::Invalid);
        }
        let limit_arg = limit.to_string();
        let bytes = self
            .orx_output(
                &[
                    "--no-telemetry",
                    "discover",
                    strategy,
                    "--limit",
                    &limit_arg,
                    "--",
                    query,
                ],
                SEARCH_LIMIT_BYTES,
                Duration::from_secs(25),
            )
            .await?;
        let items: Vec<Value> = serde_json::from_slice(&bytes).map_err(|_| OrxCallError::Failed)?;
        let papers: Vec<Value> = items.iter().take(limit.into()).map(compact_paper).collect();
        Ok(json!({"strategy": strategy, "count": papers.len(), "papers": papers}))
    }

    /// `orx paper`: alphaXiv report/full text, or OpenAlex/bioRxiv/PubMed metadata and abstract.
    pub(crate) async fn paper_page(
        &self,
        id: &str,
        source: Option<&str>,
        full: bool,
        offset: usize,
    ) -> Result<PaperPage, OrxCallError> {
        if !valid_paper_id(id) || source.is_some_and(|s| !PAPER_SOURCES.contains(&s)) {
            return Err(OrxCallError::Invalid);
        }
        let key = format!("{}\0{full}\0{id}", source.unwrap_or(""));
        let text = match self.papers().get(&key) {
            Some(text) => text,
            None => {
                let mut args = vec!["--no-telemetry", "paper"];
                if let Some(source) = source {
                    args.extend(["--source", source]);
                }
                if full {
                    args.push("--full");
                }
                args.extend(["--", id]);
                let bytes = self
                    .orx_output(&args, PAPER_LIMIT_BYTES, Duration::from_secs(45))
                    .await?;
                let text: Arc<str> = String::from_utf8_lossy(&bytes).into_owned().into();
                self.papers().put(key, text.clone());
                text
            }
        };
        Ok(page(&text, offset))
    }

    /// `orx skill [name]`: the overview (no name) or one bundled module or resource.
    pub(crate) async fn skill_text(&self, name: Option<&str>) -> Result<String, OrxCallError> {
        let mut args = vec!["--no-telemetry", "skill"];
        match name {
            None | Some("overview") => {}
            Some(name) if valid_skill_name(name) => args.push(name),
            Some(_) => return Err(OrxCallError::Invalid),
        }
        let bytes = self
            .orx_output(&args, SKILL_LIMIT_BYTES, Duration::from_secs(15))
            .await?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_names_allow_modules_and_resources_only() {
        for ok in [
            "overview",
            "orx-lit-review",
            "orx-compute/hf",
            "orx-figures/assets/line.py",
        ] {
            assert!(valid_skill_name(ok), "{ok}");
        }
        for bad in [
            "",
            "lit-review",
            "orx-",
            "orx-Lit",
            "orx-a/../etc",
            "orx-a/./b",
            "orx-a//b",
            "orx-a/.hidden",
            "/orx-a",
            "orx-a b",
        ] {
            assert!(!valid_skill_name(bad), "{bad}");
        }
    }

    #[test]
    fn paper_ids_reject_flags_and_whitespace() {
        assert!(valid_paper_id("2611.13772"));
        assert!(valid_paper_id("https://arxiv.org/abs/2611.13772"));
        assert!(valid_paper_id("10.1101/2024.01.01.123456"));
        assert!(!valid_paper_id("--full"));
        assert!(!valid_paper_id("a b"));
        assert!(!valid_paper_id(""));
        assert!(!valid_paper_id(&"x".repeat(257)));
    }

    #[test]
    fn index_parses_bundled_modules() {
        let overview = "# OpenResearch CLI\nUse orx-lit-review first.\n\nBundled modules (orx skill <name>):\n  orx-create           Initialize a project.\n  orx-lit-review       Explain concepts\n  not-a-skill          Ignored\n";
        assert_eq!(
            skill_index(overview),
            vec![
                ("orx-create".into(), "Initialize a project.".into()),
                ("orx-lit-review".into(), "Explain concepts".into()),
            ]
        );
    }

    #[test]
    fn compact_paper_caps_long_fields() {
        let item = json!({"source":"alphaxiv","id":"1","title":"T","abstract":"a".repeat(5000),"publicationDate":"2026-01-01","snippets":["s".repeat(900), {"text":"x"}, "c", "d"],"votes":3});
        let compact = compact_paper(&item);
        assert_eq!(
            compact["abstract"].as_str().unwrap().chars().count(),
            ABSTRACT_CHARS + 1
        );
        assert_eq!(compact["snippets"].as_array().unwrap().len(), SNIPPETS);
        assert_eq!(compact["votes"], 3);
    }

    #[test]
    fn pages_walk_text_by_characters() {
        let text = "가".repeat(PAPER_PAGE_CHARS + 10);
        let first = page(&text, 0);
        assert_eq!(first.text.chars().count(), PAPER_PAGE_CHARS);
        assert_eq!(first.next_offset, Some(PAPER_PAGE_CHARS));
        assert_eq!(first.total_chars, PAPER_PAGE_CHARS + 10);
        let second = page(&text, PAPER_PAGE_CHARS);
        assert_eq!(second.text.chars().count(), 10);
        assert_eq!(second.next_offset, None);
        let past = page(&text, usize::MAX);
        assert!(past.text.is_empty());
        assert_eq!(past.next_offset, None);
    }

    #[test]
    fn cache_returns_fresh_entries_and_bounds_size() {
        let cache = PaperCache::default();
        for i in 0..(PAPER_CACHE_ENTRIES + 2) {
            cache.put(format!("k{i}"), Arc::from(format!("v{i}")));
        }
        assert!(cache.get("k0").is_none());
        assert_eq!(
            cache
                .get(&format!("k{}", PAPER_CACHE_ENTRIES + 1))
                .as_deref(),
            Some(&*format!("v{}", PAPER_CACHE_ENTRIES + 1))
        );
    }
}
