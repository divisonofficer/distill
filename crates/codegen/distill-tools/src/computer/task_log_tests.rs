use pretty_assertions::assert_eq;

use super::*;

#[tokio::test]
async fn a_log_at_the_budget_is_complete() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("exact.log");
    let body = "y".repeat(64);
    tokio::fs::write(&path, &body).await.unwrap();

    assert_eq!(read_tail(&path, /*max_bytes*/ 64).await, (body, false));
}

#[tokio::test]
async fn stops_at_the_budget_and_reports_more() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("huge.log");
    tokio::fs::write(&path, "X".repeat(200_000)).await.unwrap();

    let (tail, incomplete) = read_tail(&path, /*max_bytes*/ 500).await;
    assert_eq!(tail, "X".repeat(500));
    assert!(incomplete);
}

#[tokio::test]
async fn reads_the_tail_instead_of_the_prefix() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("tail.log");
    tokio::fs::write(&path, format!("HEAD{}TAIL", "x".repeat(100)))
        .await
        .unwrap();

    assert_eq!(
        read_tail(&path, /*max_bytes*/ 8).await,
        ("xxxxTAIL".to_string(), true)
    );
}

/// Large enough to take several reads: a short read must be consumed, not
/// mistaken for the end of the file.
#[tokio::test]
async fn reads_a_log_within_the_budget_whole() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("chunked.log");
    let body = "a".repeat(300_000);
    tokio::fs::write(&path, &body).await.unwrap();

    assert_eq!(read_tail(&path, MAX_SNAPSHOT_BYTES).await, (body, false));
}

#[tokio::test]
async fn drops_a_character_split_by_the_budget() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("cjk.log");
    tokio::fs::write(&path, "日".repeat(10)).await.unwrap();

    assert_eq!(
        read_tail(&path, /*max_bytes*/ 8).await,
        ("日日".to_string(), true)
    );
}

/// The file fits the budget but ends mid character, so the dropped bytes
/// must not read as the whole log.
#[tokio::test]
async fn a_log_ending_mid_character_reads_as_incomplete() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("torn.log");
    let Some(torn) = "日".as_bytes().get(..2) else {
        panic!("日 is 3 bytes");
    };
    tokio::fs::write(&path, torn).await.unwrap();

    assert_eq!(
        read_tail(&path, /*max_bytes*/ 100).await,
        (String::new(), true)
    );
}

#[tokio::test]
async fn a_missing_log_returns_a_bounded_diagnostic() {
    let (output, incomplete) =
        read_tail(Path::new("/nonexistent/task.log"), /*max_bytes*/ 100).await;

    assert!(output.starts_with("[task output unavailable:"));
    assert!(output.len() <= 100);
    assert!(incomplete);
}

#[tokio::test]
async fn an_unreadable_log_returns_a_bounded_diagnostic() {
    let dir = tempfile::TempDir::new().unwrap();

    let (output, incomplete) = read_tail(dir.path(), /*max_bytes*/ 100).await;

    assert!(output.starts_with("[task output unavailable:"));
    assert!(output.len() <= 100);
    assert!(incomplete);
}
