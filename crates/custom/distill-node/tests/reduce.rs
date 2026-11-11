use std::fs;
use std::path::{Path, PathBuf};

use distill_node::reduce::Reducer;
use distill_node::sep::Sep;
use serde_json::Value;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// Replay one `acp/*.jsonl` fixture through a reducer and collect the events.
fn run(path: &Path) -> Vec<Sep> {
    let text = fs::read_to_string(path).unwrap();
    let lines: Vec<(String, Value)> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let row: Value = serde_json::from_str(line).unwrap();
            (row["dir"].as_str().unwrap().to_owned(), row["msg"].clone())
        })
        .collect();
    // Session id: the loaded one if the fixture loads, else the first `session/new` result.
    let session_id = lines
        .iter()
        .find(|(_, msg)| msg["method"] == "session/load")
        .or_else(|| {
            lines
                .iter()
                .find(|(_, msg)| msg["result"]["sessionId"].is_string())
        })
        .map(|(_, msg)| {
            msg["params"]["sessionId"]
                .as_str()
                .or_else(|| msg["result"]["sessionId"].as_str())
                .unwrap()
                .to_owned()
        })
        .unwrap();
    let mut reducer = Reducer::new(session_id);
    let mut out = Vec::new();
    for (dir, msg) in &lines {
        match dir.as_str() {
            "c2a" => out.extend(reducer.on_client(msg)),
            "a2c" => out.extend(reducer.on_agent(msg)),
            other => panic!("unknown dir {other}"),
        }
    }
    out
}

#[test]
fn acp_fixtures_match_sep_expectations() {
    let acp_dir = fixtures().join("acp");
    let mut count = 0;
    for entry in fs::read_dir(&acp_dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
        let expected_path = fixtures().join("sep").join(format!("{stem}.json"));
        let expected: Vec<Sep> = serde_json::from_str(
            &fs::read_to_string(&expected_path)
                .unwrap_or_else(|e| panic!("{}: {e}", expected_path.display())),
        )
        .unwrap();
        assert_eq!(run(&path), expected, "fixture {stem}");
        count += 1;
    }
    assert_eq!(count, 7);
}
