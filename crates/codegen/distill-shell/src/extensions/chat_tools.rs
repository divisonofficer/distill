//! Bounded, read-only tools for account-scoped general chat.
use distill_sampling_types::ToolSpec;
use futures::StreamExt as _;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{ffi::OsString, process::Stdio, time::Duration};
use tokio::io::AsyncReadExt as _;

const MAX_BODY: usize = 256 * 1024;
const LIMIT: usize = 6;
const TIMEOUT: Duration = Duration::from_secs(20);

pub(super) struct ChatTools {
    orx: Option<OsString>,
    brave: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Search {
    query: String,
    year: Option<u16>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ReadPaper {
    id: String,
    #[serde(default)]
    offset: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PaperFigures {
    id: String,
    figure: Option<u16>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Chart {
    title: String,
    values: Vec<Point>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Point {
    label: String,
    value: f64,
}

pub(super) struct Output {
    pub text: String,
    pub chart: Option<String>,
}

impl ChatTools {
    pub(super) fn load() -> Self {
        Self {
            orx: std::env::var_os("POSTUDIO_OPENRESEARCH_BIN").filter(|path| !path.is_empty()),
            brave: std::env::var("POSTUDIO_BRAVE_SEARCH_API_KEY")
                .ok()
                .filter(|key| !key.trim().is_empty()),
        }
    }

    pub(super) fn specs(&self) -> Vec<ToolSpec> {
        let mut specs = vec![spec(
            "bar_chart",
            "Render a bar chart in chat from known nonnegative numeric values. Never invent data.",
            json!({
                "type":"object", "additionalProperties":false, "required":["title","values"],
                "properties":{"title":{"type":"string"},"values":{"type":"array","minItems":1,"maxItems":12,
                    "items":{"type":"object","additionalProperties":false,"required":["label","value"],"properties":{"label":{"type":"string"},"value":{"type":"number","minimum":0}}}}}
            }),
        )];
        let search = json!({"type":"object","additionalProperties":false,"required":["query"],"properties":{"query":{"type":"string","maxLength":512}}});
        if self.orx.is_some() {
            let mut paper_schema = search.clone();
            paper_schema["properties"]["year"] =
                json!({"type":"integer","minimum":1991,"maximum":2100});
            specs.push(spec("search_papers", "Search alphaXiv for scholarly papers using OpenResearch. Use English topic terms and the year filter if requested. Results are discovery metadata; use get_paper to read selected papers.", paper_schema));
            specs.push(spec("get_paper_figures", "Find original numbered figures and captions in an arXiv paper's official HTML. Omit figure to list available figures; provide a figure number to show that original figure in chat. You receive captions, not pixels; do not claim visual inspection. Chat adds selected figures automatically, so do not emit duplicate image markup.", json!({"type":"object","additionalProperties":false,"required":["id"],"properties":{"id":{"type":"string","maxLength":80},"figure":{"type":"integer","minimum":1,"maximum":200}}})));
            specs.push(spec("get_paper", "Read a bounded excerpt of alphaXiv's extracted paper body, not a PDF render. Pass an arXiv id from search results or provided by the user. To continue use nextOffset. No page numbers are available. Prefer reading 2–3 relevant papers rather than repeatedly searching for a detailed comparison.", json!({"type":"object","additionalProperties":false,"required":["id"],"properties":{"id":{"type":"string","maxLength":80},"offset":{"type":"integer","minimum":0,"maximum":500000}}})));
        }
        if self.brave.is_some() {
            specs.push(spec(
                "web_search",
                "Search the public web for current information. Use returned URLs when citing.",
                search,
            ));
        }
        specs
    }

    pub(super) async fn execute(&self, name: &str, args: &str) -> Output {
        let error = |message: &str| Output {
            text: json!({"error":message}).to_string(),
            chart: None,
        };
        if args.len() > 8192 {
            return error("Tool arguments are too large");
        }
        if name == "bar_chart" {
            return match serde_json::from_str(args).ok().and_then(render_chart) {
                Some(chart) => Output { text: json!({"rendered":true,"instruction":"The chart will be appended to your final answer; explain it without reproducing it."}).to_string(), chart: Some(chart) },
                None => error("Invalid chart data"),
            };
        }
        if name == "get_paper_figures" {
            if self.orx.is_none() {
                return error("Paper tools are not configured");
            }
            let Ok(args) = serde_json::from_str::<PaperFigures>(args) else {
                return error("Invalid figure arguments");
            };
            if !valid_arxiv_id(&args.id)
                || args
                    .figure
                    .is_some_and(|figure| !(1..=200).contains(&figure))
            {
                return error("Use an arXiv ID and a supported figure number");
            }
            return match fetch_paper_figures(args).await {
                Some(output) => output,
                None => error(
                    "No usable figure was found in the paper's arXiv HTML. A PDF-only paper may require a separate extraction tool.",
                ),
            };
        }
        if name == "get_paper" {
            let Some(bin) = &self.orx else {
                return error("OpenResearch is not configured");
            };
            let Ok(args) = serde_json::from_str::<ReadPaper>(args) else {
                return error("Invalid paper arguments");
            };
            if !valid_arxiv_id(&args.id) || args.offset > 500_000 {
                return error("Use a valid arXiv ID and body offset");
            }
            return match read_paper(bin, args).await {
                Ok(result) => Output {
                    text: result.to_string(),
                    chart: None,
                },
                Err(()) => error(
                    "Extracted body unavailable or timed out. Do not claim the full text or PDF was read.",
                ),
            };
        }
        let Some(Search { query, year }) =
            serde_json::from_str::<Search>(args).ok().filter(|search| {
                !search.query.trim().is_empty()
                    && search.query.len() <= 512
                    && !search.query.chars().any(char::is_control)
                    && search
                        .year
                        .is_none_or(|year| name == "search_papers" && (1991..=2100).contains(&year))
            })
        else {
            return error("Invalid search query");
        };
        let result = match name {
            "search_papers" => match &self.orx {
                Some(bin) => paper_search(bin, &query, year).await,
                None => return error("OpenResearch is not configured"),
            },
            "web_search" => match &self.brave {
                Some(key) => web_search(key, &query).await,
                None => return error("Web search is not configured"),
            },
            _ => return error("Unknown tool"),
        };
        match result {
            Ok(results) => Output {
                text: json!({"query":query,"count":results.len(),"results":results}).to_string(),
                chart: None,
            },
            Err(()) => error("Search failed or timed out; do not claim that results were verified"),
        }
    }
}

/// Render provenance from the tool payload, not from the model's own claims.
pub(super) fn search_receipt(label: &str, result: &str) -> String {
    let payload: Value = serde_json::from_str(result).unwrap_or_default();
    let Some(results) = payload.get("results").and_then(Value::as_array) else {
        return if label == "논문 본문 읽기" {
            format!("**{label}: 본문을 가져오지 못함**")
        } else {
            format!("**{label}: 검색을 완료하지 못함**")
        };
    };
    let mut receipt = if label == "논문 본문 읽기" {
        format!("**{label}: 본문 발췌 확인**")
    } else {
        format!("**{label}: 결과 {}건**", results.len())
    };
    for item in results {
        let Some(link) = item
            .get("url")
            .and_then(Value::as_str)
            .and_then(public_link)
        else {
            continue;
        };
        let link = link.replace('<', "%3C").replace('>', "%3E");
        let title: String = text(item, &["title"], 160)
            .chars()
            .filter(|c| c.is_alphanumeric() || " -_.,()/+:".contains(*c))
            .collect();
        let citation: String = text(item, &["citation"], 150)
            .chars()
            .filter(|c| c.is_alphanumeric() || " -_.,()/+:·".contains(*c))
            .collect();
        let date: String = text(item, &["published"], 10)
            .chars()
            .filter(|c| c.is_ascii_digit() || *c == '-')
            .collect();
        let title = if title.trim().is_empty() {
            "원문"
        } else {
            title.trim()
        };
        use std::fmt::Write as _;
        let _ = write!(receipt, "\n- [{title}](<{link}>)");
        if !citation.trim().is_empty() {
            let _ = write!(receipt, " · {}", citation.trim());
        } else if !date.is_empty() {
            let _ = write!(receipt, " · {date}");
        }
    }
    receipt
}

fn spec(name: &str, description: &str, parameters: Value) -> ToolSpec {
    ToolSpec {
        name: name.into(),
        description: Some(description.into()),
        parameters,
    }
}

fn text(item: &Value, keys: &[&str], limit: usize) -> String {
    keys.iter()
        .find_map(|key| item[*key].as_str())
        .unwrap_or("")
        .chars()
        .take(limit)
        .collect()
}

fn public_link(url: &str) -> Option<&str> {
    let parsed = url::Url::parse(url).ok()?;
    (url.len() <= 2048
        && matches!(parsed.scheme(), "http" | "https")
        && parsed.host_str().is_some()
        && parsed.username().is_empty()
        && parsed.password().is_none())
    .then_some(url)
}

fn paper(item: &Value) -> Option<Value> {
    let id = text(item, &["id", "arxivId", "paperId"], 96);
    let url = item["url"]
        .as_str()
        .and_then(public_link)
        .map(str::to_owned)
        .or_else(|| {
            (!id.is_empty()
                && id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'/'))
                && !id.contains(".."))
            .then(|| format!("https://arxiv.org/abs/{id}"))
        })?;
    Some(
        json!({"id":id,"title": text(item,&["title"],300),"published":text(item,&["publicationDate","published","date"],64),"excerpt":text(item,&["abstract","summary"],1200),"url":url}),
    )
}

fn valid_arxiv_id(id: &str) -> bool {
    if id.len() > 80 {
        return false;
    }
    let base = if let Some((base, version)) = id.rsplit_once('v') {
        if !version.is_empty()
            && version.bytes().all(|b| b.is_ascii_digit())
            && (base.contains('/') || base.contains('.'))
        {
            base
        } else {
            id
        }
    } else {
        id
    };
    let digits =
        |value: &str, size| value.len() == size && value.bytes().all(|b| b.is_ascii_digit());
    if let Some((archive, number)) = base.split_once('/') {
        return archive.starts_with(|c: char| c.is_ascii_alphabetic())
            && archive
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.'))
            && digits(number, 7);
    }
    base.split_once('.').is_some_and(|(prefix, suffix)| {
        digits(prefix, 4) && (digits(suffix, 4) || digits(suffix, 5))
    })
}

fn paper_excerpt(raw: &str, args: ReadPaper) -> Result<Value, ()> {
    let (header, body) = raw.split_once("\n\n").ok_or(())?;
    if !header.starts_with("alphaXiv: ") {
        return Err(());
    }
    let body = body.trim();
    let total = body.chars().count();
    if args.offset >= total {
        return Err(());
    }
    let excerpt: String = body.chars().skip(args.offset).take(12_000).collect();
    let end = args.offset + excerpt.chars().count();
    let title = body
        .lines()
        .next()
        .map(|line| line.trim().trim_start_matches('#').trim())
        .filter(|line| (4..=250).contains(&line.chars().count()))
        .map(str::to_owned)
        .unwrap_or_else(|| format!("arXiv {}", args.id));
    Ok(
        json!({"source":"alphaXiv extracted full text", "id":args.id, "offset":args.offset, "nextOffset":(end < total).then_some(end),"totalChars":total,"excerpt":excerpt,"results":[{"title":title,"url":format!("https://arxiv.org/abs/{}", args.id)}]}),
    )
}

#[derive(serde::Serialize)]
struct ArxivFigure {
    number: u16,
    caption: String,
    images: Vec<String>,
}

fn extract_arxiv_figures(html: &str, paper_url: &url::Url, id: &str) -> Vec<ArxivFigure> {
    use std::sync::LazyLock;
    static FIGURE: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"(?is)<figure\b[^>]*>(.*?)</figure>").expect("figure regex")
    });
    static CAPTION: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"(?is)<figcaption\b[^>]*>(.*?)</figcaption>").expect("caption regex")
    });
    static IMG: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r#"(?is)<img\b[^>]*?\ssrc\s*=\s*["']([^"']+)["']"#).expect("image regex")
    });
    static TAG: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"<[^>]+>").expect("tag regex"));
    static NUMBER: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"(?i)^\s*Figure\s+(\d{1,3})\b").expect("figure number regex")
    });
    let unversioned = id
        .rsplit_once('v')
        .filter(|(_, version)| !version.is_empty() && version.bytes().all(|b| b.is_ascii_digit()))
        .map_or(id, |(base, _)| base);
    let prefix = format!("/html/{unversioned}");
    let mut figures = Vec::new();
    for figure in FIGURE.captures_iter(html).take(200) {
        let (_, [body]) = figure.extract();
        let Some(caption) = CAPTION.captures(body).and_then(|matched| matched.get(1)) else {
            continue;
        };
        let plain = TAG
            .replace_all(caption.as_str(), "")
            .replace("&amp;", "&")
            .replace("&nbsp;", " ")
            .replace("&#160;", " ");
        let Some(number) = NUMBER
            .captures(&plain)
            .and_then(|matched| matched.get(1))
            .and_then(|number| number.as_str().parse::<u16>().ok())
        else {
            continue;
        };
        let images: Vec<String> = IMG
            .captures_iter(body)
            .filter_map(|matched| {
                let raw = matched.get(1)?.as_str();
                if raw.len() > 600 {
                    return None;
                }
                let image = paper_url.join(raw).ok()?;
                if image.scheme() != "https"
                    || image.host_str() != Some("arxiv.org")
                    || !image.username().is_empty()
                    || image.password().is_some()
                    || image.port().is_some()
                    || image.query().is_some()
                    || image.fragment().is_some()
                {
                    return None;
                }
                let path = image.path();
                let tail = path.strip_prefix(&prefix)?;
                let (version, filename) = tail.split_once('/')?;
                if !(version.is_empty()
                    || version.strip_prefix('v').is_some_and(|digits| {
                        !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
                    }))
                {
                    return None;
                }
                if filename.is_empty()
                    || !filename.bytes().all(|b| {
                        b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'/')
                    })
                    || !filename.rsplit_once('.').is_some_and(|(_, ext)| {
                        ["png", "jpg", "jpeg", "webp"]
                            .iter()
                            .any(|format| ext.eq_ignore_ascii_case(format))
                    })
                {
                    return None;
                }
                Some(image.into())
            })
            .take(3)
            .collect();
        if !images.is_empty()
            && !figures
                .iter()
                .any(|figure: &ArxivFigure| figure.number == number)
        {
            let caption = plain
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .take(450)
                .collect();
            figures.push(ArxivFigure {
                number,
                caption,
                images,
            });
        }
    }
    figures
}

async fn fetch_paper_figures(args: PaperFigures) -> Option<Output> {
    use std::fmt::Write as _;
    let paper_url = url::Url::parse(&format!("https://arxiv.org/html/{}", args.id)).ok()?;
    let client = distill_extra_ca::build_reqwest_client(|builder| {
        builder
            .timeout(TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
    })
    .ok()?;
    let response = client
        .get(paper_url.clone())
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.ok()?;
        if bytes.len().saturating_add(chunk.len()) > 4 * 1024 * 1024 {
            return None;
        }
        bytes.extend_from_slice(&chunk);
    }
    let figures = extract_arxiv_figures(std::str::from_utf8(&bytes).ok()?, &paper_url, &args.id);
    if let Some(number) = args.figure {
        let figure = figures.iter().find(|figure| figure.number == number)?;
        let mut card = format!("\n\n#### 원문 그림 {number} · arXiv {}\n\n", args.id);
        for image in &figure.images {
            let _ = writeln!(card, "![arXiv 원문 그림 {number}](<{image}>)\n");
        }
        let caption: String = figure
            .caption
            .chars()
            .filter(|c| c.is_alphanumeric() || " -_.,()/+:·%".contains(*c))
            .collect();
        let _ = writeln!(card, "{caption}\n\n[arXiv 원문 HTML](<{paper_url}>)");
        return Some(Output { text: json!({"id":args.id,"shown":number,"caption":figure.caption,"instruction":"The original image is attached to the answer; you saw only this caption, not the pixels."}).to_string(), chart: Some(card) });
    }
    if figures.is_empty() {
        return None;
    }
    let catalog: Vec<_> = figures
        .iter()
        .take(24)
        .map(|figure| json!({"number":figure.number,"caption":figure.caption}))
        .collect();
    Some(Output {
        text: json!({"id":args.id,"figures":catalog,"total":figures.len()}).to_string(),
        chart: None,
    })
}

pub(super) fn figure_receipt(result: &str) -> String {
    let payload: Value = serde_json::from_str(result).unwrap_or_default();
    if let Some(number) = payload.get("shown").and_then(Value::as_u64) {
        format!("**논문 원문 그림: 그림 {number} 표시**")
    } else if let Some(total) = payload.get("total").and_then(Value::as_u64) {
        format!("**논문 원문 그림: 그림 {total}개 확인**")
    } else {
        "**논문 원문 그림: 그림을 가져오지 못함**".into()
    }
}

fn arxiv_citation(html: &str) -> (Option<String>, Option<String>) {
    static META: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r#"<meta\s+name="citation_([a-z_]+)"\s+content="([^"]*)""#)
            .expect("valid citation metadata regex")
    });
    static JOURNAL_REF: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r#"<td\s+class="tablecell jref"[^>]*>\s*([^<]+)</td>"#)
            .expect("valid journal reference regex")
    });
    let mut title = None;
    let mut authors = Vec::new();
    let mut year = None;
    let mut venue = None;
    let clean = |value: &str| {
        value
            .replace("&amp;", "&")
            .replace("&quot;", "\"")
            .replace("&#39;", "'")
            .trim()
            .chars()
            .take(250)
            .collect::<String>()
    };
    for capture in META.captures_iter(html) {
        let (_, [name, value]) = capture.extract();
        match name {
            "title" => title = Some(clean(value)),
            "author" if authors.len() < 2 => authors.push(clean(value)),
            "date" => {
                year = value
                    .split(['/', '-'])
                    .next()
                    .filter(|year| year.len() == 4 && year.bytes().all(|b| b.is_ascii_digit()))
                    .map(str::to_owned)
            }
            "conference_title" | "journal_title" if !value.trim().is_empty() => {
                venue = Some(clean(value))
            }
            _ => {}
        }
    }
    if venue.is_none() {
        venue = JOURNAL_REF
            .captures(html)
            .and_then(|m| m.get(1))
            .map(|m| clean(m.as_str()));
    }
    let surname = authors.first().and_then(|author| {
        author
            .split_once(',')
            .map(|(last, _)| last.trim())
            .or_else(|| author.split_whitespace().last())
    });
    let mut parts = Vec::new();
    if let Some(last) = surname.filter(|last| !last.is_empty()) {
        parts.push(if authors.len() > 1 {
            format!("{last} et al.")
        } else {
            last.into()
        });
    }
    if let Some(year) = year {
        parts.push(year);
    }
    if title.is_none() && parts.is_empty() && venue.is_none() {
        return (None, None);
    }
    parts.push(
        venue
            .filter(|venue| !venue.is_empty())
            .unwrap_or_else(|| "arXiv 프리프린트".into()),
    );
    (
        title.filter(|title| !title.is_empty()),
        Some(parts.join(" · ")),
    )
}

async fn fetch_arxiv_citation(id: &str) -> Option<(Option<String>, Option<String>)> {
    let client = distill_extra_ca::build_reqwest_client(|builder| {
        builder
            .timeout(Duration::from_secs(5))
            .redirect(reqwest::redirect::Policy::none())
    })
    .ok()?;
    let response = client
        .get(format!("https://arxiv.org/abs/{id}"))
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.ok()?;
        if bytes.len() + chunk.len() > 512 * 1024 {
            return None;
        }
        bytes.extend_from_slice(&chunk);
    }
    Some(arxiv_citation(std::str::from_utf8(&bytes).ok()?))
}

async fn read_paper(bin: &OsString, args: ReadPaper) -> Result<Value, ()> {
    let id = args.id.clone();
    let offset = args.offset;
    let (body, metadata) = tokio::join!(read_paper_body(bin, args), async {
        if offset == 0 {
            fetch_arxiv_citation(&id).await
        } else {
            None
        }
    });
    let mut body = body?;
    if let Some((title, citation)) = metadata
        && let Some(result) = body
            .get_mut("results")
            .and_then(Value::as_array_mut)
            .and_then(|items| items.first_mut())
            .and_then(Value::as_object_mut)
    {
        if let Some(title) = title {
            result.insert("title".into(), title.into());
        }
        if let Some(citation) = citation {
            result.insert("citation".into(), citation.into());
        }
    }
    Ok(body)
}

async fn read_paper_body(bin: &OsString, args: ReadPaper) -> Result<Value, ()> {
    const BODY_LIMIT: usize = 2 * 1024 * 1024;
    tokio::time::timeout(TIMEOUT, async {
        let mut child = tokio::process::Command::new(bin)
            .args([
                "--no-telemetry",
                "paper",
                "--source",
                "alphaxiv",
                "--full",
                "--",
                &args.id,
            ])
            .env_remove("POSTUDIO_BRAVE_SEARCH_API_KEY")
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .stdout(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|_| ())?;
        let mut bytes = Vec::new();
        child
            .stdout
            .take()
            .ok_or(())?
            .take((BODY_LIMIT + 1) as u64)
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| ())?;
        if bytes.len() > BODY_LIMIT || !child.wait().await.map_err(|_| ())?.success() {
            return Err(());
        }
        paper_excerpt(std::str::from_utf8(&bytes).map_err(|_| ())?, args)
    })
    .await
    .map_err(|_| ())?
}

async fn paper_search(bin: &OsString, query: &str, year: Option<u16>) -> Result<Vec<Value>, ()> {
    tokio::time::timeout(TIMEOUT, async {
        // Fixed executable and argument vector; the model supplies only the search term.
        let mut command = tokio::process::Command::new(bin);
        command.env_remove("POSTUDIO_BRAVE_SEARCH_API_KEY");
        command.args(["--no-telemetry", "discover", "keyword", "--limit", "6"]);
        if let Some(year) = year {
            command
                .arg("--published-after")
                .arg(format!("{year}-01-01"))
                .arg("--published-before")
                .arg(format!("{}-01-01", year + 1));
        }
        let mut child = command
            .args(["--", query])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .stdout(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|_| ())?;
        let mut bytes = Vec::new();
        child
            .stdout
            .take()
            .ok_or(())?
            .take((MAX_BODY + 1) as u64)
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| ())?;
        if bytes.len() > MAX_BODY || !child.wait().await.map_err(|_| ())?.success() {
            return Err(());
        }
        let items: Vec<Value> = serde_json::from_slice(&bytes).map_err(|_| ())?;
        let year_prefix = year.map(|year| format!("{year}-"));
        Ok(items
            .iter()
            .take(LIMIT)
            .filter_map(paper)
            .filter(|item| {
                year_prefix.as_deref().is_none_or(|prefix| {
                    item["published"]
                        .as_str()
                        .is_some_and(|date| date.starts_with(prefix))
                })
            })
            .collect())
    })
    .await
    .map_err(|_| ())?
}

async fn web_search(key: &str, query: &str) -> Result<Vec<Value>, ()> {
    let client = distill_extra_ca::build_reqwest_client(|builder| {
        builder
            .timeout(TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
    })
    .map_err(|_| ())?;
    let response = client
        .get("https://api.search.brave.com/res/v1/web/search")
        .header("X-Subscription-Token", key)
        .query(&[("q", query), ("count", "6")])
        .send()
        .await
        .map_err(|_| ())?
        .error_for_status()
        .map_err(|_| ())?;
    if !response.status().is_success() {
        return Err(());
    }
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| ())?;
        if bytes.len().saturating_add(chunk.len()) > MAX_BODY {
            return Err(());
        }
        bytes.extend_from_slice(&chunk);
    }
    let body: Value = serde_json::from_slice(&bytes).map_err(|_| ())?;
    Ok(body["web"]["results"].as_array().into_iter().flatten().take(LIMIT).filter_map(|item| {
        Some(json!({"title":text(item,&["title"],300),"excerpt":text(item,&["description"],700),"url":public_link(item["url"].as_str()?)?}))
    }).collect())
}

fn render_chart(chart: Chart) -> Option<String> {
    let clean = |text: &str| {
        text.chars()
            .filter(|c| c.is_alphanumeric() || " -_.,()%+".contains(*c))
            .take(80)
            .collect::<String>()
    };
    let title = clean(&chart.title);
    if title.trim().is_empty()
        || chart.values.is_empty()
        || chart.values.len() > 12
        || chart
            .values
            .iter()
            .any(|point| !point.value.is_finite() || point.value < 0.0 || point.value > 1.0e15)
    {
        return None;
    }
    let max = chart
        .values
        .iter()
        .map(|point| point.value)
        .fold(0.0, f64::max);
    let mut table = format!("\n\n### {title}\n\n| 항목 | 값 | 비교 |\n|---|---:|---|\n");
    for point in chart.values {
        let label = clean(&point.label);
        if label.trim().is_empty() {
            return None;
        }
        let bars = if max == 0.0 {
            0
        } else {
            (point.value / max * 20.0).round() as usize
        };
        use std::fmt::Write as _;
        let _ = writeln!(
            table,
            "| {label} | {} | {} |",
            point.value,
            "█".repeat(bars)
        );
    }
    Some(table)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn tool_arguments_are_bounded_and_charts_are_inert() {
        let tools = ChatTools {
            orx: None,
            brave: None,
        };
        assert_eq!(tools.specs().len(), 1);
        let result = tools
            .execute(
                "bar_chart",
                r#"{"title":"<img>","values":[{"label":"x|y","value":2},{"label":"z","value":1}]}"#,
            )
            .await;
        let output = result.chart.unwrap();
        assert!(output.contains("| xy | 2 |"));
        assert!(!output.contains('<'));
        assert!(
            tools
                .execute(
                    "bar_chart",
                    r#"{"title":"T","values":[{"label":"x","value":-1}]}"#
                )
                .await
                .chart
                .is_none()
        );
        assert!(
            tools
                .execute("web_search", r#"{"query":"test"}"#)
                .await
                .text
                .contains("not configured")
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn paper_tool_runs_openresearch_with_year_and_literal_query() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("orx-fixture");
        std::fs::write(&bin, r#"#!/bin/sh
[ "$1 $2 $3 $4 $5 $6 $7 $8 $9 ${10}" = '--no-telemetry discover keyword --limit 6 --published-after 2026-01-01 --published-before 2027-01-01 --' ] || exit 1
[ "$#" = 11 ] && [ "${11}" = '--literal argument' ] || exit 1
printf '%s' '[{"id":"2601.12345","title":"Navigation","publicationDate":"2026-01-01"}]'
"#).unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o700)).unwrap();
        let tools = ChatTools {
            orx: Some(bin.into_os_string()),
            brave: None,
        };
        let result = tools
            .execute(
                "search_papers",
                r#"{"query":"--literal argument","year":2026}"#,
            )
            .await;
        let value: Value = serde_json::from_str(&result.text).unwrap();
        assert_eq!(value["count"], 1);
        assert_eq!(value["results"][0]["published"], "2026-01-01");
    }

    #[test]
    fn receipt_reports_results_and_rejects_non_web_links() {
        let results = json!({"results":[{"title":"[VLN] <b>","published":"2026-01-01T10:00:00Z","url":"https://arxiv.org/abs/2601.12345"},{"title":"unsafe","url":"javascript:alert(1)"}]});
        let receipt = search_receipt("alphaXiv", &results.to_string());
        assert!(receipt.contains("[VLN b](<https://arxiv.org/abs/2601.12345>) · 2026-01-01"));
        assert!(!receipt.contains("javascript:"));
        assert!(search_receipt("alphaXiv", "{}").contains("완료하지 못함"));
        assert!(search_receipt("alphaXiv", r#"{"results":[]}"#).contains("0건"));
    }

    #[test]
    fn original_figures_only_take_raster_images_from_the_same_arxiv_paper() {
        let paper = url::Url::parse("https://arxiv.org/html/2611.18346").unwrap();
        let html = r#"<figure><img src="2611.18346v1/fig1.png"><img src="https://evil.example/fig.png"><img src="2611.18346v1/a.svg"><img src="2611.12345v1/fig.png"><figcaption><b>Figure 1</b>: Example &amp; source</figcaption></figure><figure><img src="2611.18346v1/table.jpg"><figcaption>Table 1: skip</figcaption></figure>"#;
        let figures = extract_arxiv_figures(html, &paper, "2611.18346");
        assert_eq!(figures.len(), 1);
        let figure = figures.first().unwrap();
        assert_eq!(figure.number, 1);
        assert_eq!(figure.caption, "Figure 1: Example & source");
        assert_eq!(
            figure.images,
            ["https://arxiv.org/html/2611.18346v1/fig1.png"]
        );
    }

    #[test]
    fn arxiv_label_uses_declared_venue_and_never_infers_acceptance() {
        let html = r#"<meta name="citation_title" content="VLAN &amp; navigation" /><meta name="citation_author" content="Hu, Jie" /><meta name="citation_author" content="Zheng, Yu" /><meta name="citation_date" content="2026/11/13" />"#;
        let (title, label) = arxiv_citation(html);
        assert_eq!(title.as_deref(), Some("VLAN & navigation"));
        assert_eq!(
            label.as_deref(),
            Some("Hu et al. · 2026 · arXiv 프리프린트")
        );
        assert_eq!(
            arxiv_citation(&format!(
                "{html}<meta name=\"citation_conference_title\" content=\"Example Conference\" />"
            ))
            .1
            .as_deref(),
            Some("Hu et al. · 2026 · Example Conference")
        );
        assert_eq!(arxiv_citation("no metadata"), (None, None));
    }

    #[test]
    fn paper_body_uses_unicode_offsets_and_a_canonical_source_link() {
        assert!(valid_arxiv_id("2611.18346v2"));
        assert!(valid_arxiv_id("cs.LG/9901001v1"));
        for bad in [
            "--full",
            "https://arxiv.org/abs/2611.18346",
            "../9901001",
            "2611.18346?foo",
        ] {
            assert!(!valid_arxiv_id(bad));
        }
        let raw = format!(
            "alphaXiv: https://www.alphaxiv.org/abs/2611.18346\n\n가{}",
            "한".repeat(12001)
        );
        let result = paper_excerpt(
            &raw,
            ReadPaper {
                id: "2611.18346".into(),
                offset: 1,
            },
        )
        .unwrap();
        assert_eq!(result["nextOffset"], 12001);
        assert_eq!(result["excerpt"].as_str().unwrap().chars().count(), 12000);
        assert_eq!(
            result["results"][0]["url"],
            "https://arxiv.org/abs/2611.18346"
        );
        assert!(search_receipt("논문 본문 읽기", &result.to_string()).contains("본문 발췌 확인"));
    }

    #[test]
    fn paper_compaction_keeps_date_and_source() {
        let result = paper(
            &json!({"id":"2601.12345", "title":"Navigation", "publicationDate":"2026-01-01"}),
        )
        .unwrap();
        assert_eq!(result["published"], "2026-01-01");
        assert_eq!(result["url"], "https://arxiv.org/abs/2601.12345");
    }
}
