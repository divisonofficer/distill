//! Session-scoped artifact directory listing.

use std::path::{Component, Path};

use axum::Router;
use axum::extract::{Path as AxumPath, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde::{Deserialize, Serialize};

use crate::keeper::Keeper;

#[derive(Deserialize)]
struct ListQuery {
    path: Option<String>,
}

#[derive(Serialize)]
struct Listing {
    entries: Vec<Entry>,
}

#[derive(Serialize)]
struct Entry {
    name: String,
    path: String,
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    mime: Option<&'static str>,
}

pub fn routes(keeper: Keeper) -> Router {
    Router::new()
        .route("/api/sessions/{id}/artifacts", get(list))
        .with_state(keeper)
}

async fn list(
    State(keeper): State<Keeper>,
    AxumPath(id): AxumPath<String>,
    Query(query): Query<ListQuery>,
) -> Response {
    let not_found = || StatusCode::NOT_FOUND.into_response();
    let Some(cwd) = keeper.cwd_of(&id) else {
        return not_found();
    };
    let root = match canonicalize(cwd).await {
        Ok(path) => path,
        Err(_) => return not_found(),
    };
    let relative = query.path.as_deref().unwrap_or("");
    let target = match resolve_directory(&root, relative).await {
        Some(path) => path,
        None => return not_found(),
    };
    if !matches!(tokio::fs::symlink_metadata(&target).await, Ok(meta) if meta.is_dir()) {
        return not_found();
    }
    let mut dir = match tokio::fs::read_dir(&target).await {
        Ok(dir) => dir,
        Err(_) => return not_found(),
    };
    let mut entries = Vec::new();
    while entries.len() < 500 {
        let item = match dir.next_entry().await {
            Ok(Some(item)) => item,
            Ok(None) => break,
            Err(_) => return not_found(),
        };
        let name = item.file_name();
        let Some(name) = name
            .to_str()
            .filter(|n| !n.is_empty() && *n != "." && *n != "..")
        else {
            continue;
        };
        if !matches!(canonicalize(item.path()).await, Ok(path) if path.starts_with(&root)) {
            continue;
        }
        let Ok(meta) = tokio::fs::symlink_metadata(item.path()).await else {
            continue;
        };
        if meta.file_type().is_symlink() {
            continue;
        }
        let child_relative = if relative.is_empty() {
            name.to_owned()
        } else {
            format!("{relative}/{name}")
        };
        if meta.is_dir() {
            entries.push(Entry {
                name: name.to_owned(),
                path: child_relative,
                kind: "directory",
                mime: None,
            });
        } else if meta.is_file() {
            entries.push(Entry {
                name: name.to_owned(),
                path: child_relative,
                kind: "file",
                mime: mime(item.path().as_path()),
            });
        }
    }
    entries.sort_by(|a, b| {
        (a.kind != "directory")
            .cmp(&(b.kind != "directory"))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    (StatusCode::OK, axum::Json(Listing { entries })).into_response()
}

async fn canonicalize(path: impl AsRef<Path>) -> Result<std::path::PathBuf, ()> {
    let path = path.as_ref().to_path_buf();
    tokio::task::spawn_blocking(move || dunce::canonicalize(path))
        .await
        .map_err(|_| ())?
        .map_err(|_| ())
}

async fn resolve_directory(root: &Path, relative: &str) -> Option<std::path::PathBuf> {
    if !safe_relative(relative) {
        return None;
    }
    let target = if relative.is_empty() {
        root.to_path_buf()
    } else {
        root.join(relative)
    };
    let target = canonicalize(target).await.ok()?;
    if !target.starts_with(root) || !tokio::fs::metadata(&target).await.ok()?.is_dir() {
        return None;
    }
    Some(target)
}

fn safe_relative(value: &str) -> bool {
    if value.is_empty() {
        return true;
    }
    let path = Path::new(value);
    !path.is_absolute()
        && path.components().all(|c| matches!(c, Component::Normal(_)))
        && !value.contains('\\')
}

fn mime(path: &Path) -> Option<&'static str> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        "svg" => Some("image/svg+xml"),
        "html" | "htm" => Some("text/html"),
        "pdf" => Some("application/pdf"),
        "txt" => Some("text/plain"),
        "json" => Some("application/json"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn directory_resolution_allows_root_and_nested_and_rejects_escape() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("session");
        tokio::fs::create_dir_all(root.join("plots")).await.unwrap();
        tokio::fs::write(root.join("plots/a.png"), b"png")
            .await
            .unwrap();
        let canonical_root = canonicalize(&root).await.unwrap();
        assert_eq!(
            resolve_directory(&canonical_root, "").await.as_deref(),
            Some(canonical_root.as_path())
        );
        assert_eq!(
            resolve_directory(&canonical_root, "plots").await,
            Some(canonical_root.join("plots"))
        );
        assert!(resolve_directory(&canonical_root, "../").await.is_none());
        assert!(resolve_directory(&canonical_root, "/etc").await.is_none());
        assert!(
            resolve_directory(&canonical_root, "missing")
                .await
                .is_none()
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let outside = temp.path().join("outside");
            tokio::fs::create_dir(&outside).await.unwrap();
            symlink(&outside, root.join("escape")).unwrap();
            assert!(resolve_directory(&canonical_root, "escape").await.is_none());
        }
    }

    #[test]
    fn relative_paths_reject_escape_and_absolute_paths() {
        for path in ["../secret", "/tmp/x", "a/../../b", "a\\b"] {
            assert!(!safe_relative(path), "{path}");
        }
        for path in ["", "plots", "plots/run 1"] {
            assert!(safe_relative(path), "{path}");
        }
    }

    #[test]
    fn mime_is_limited_to_known_preview_types() {
        assert_eq!(mime(Path::new("plot.PNG")), Some("image/png"));
        assert_eq!(mime(Path::new("index.html")), Some("text/html"));
        assert_eq!(mime(Path::new("secret.bin")), None);
    }
}
