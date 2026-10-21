//! Map Local: builds the answer from an inline body, a file, or a file
//! under a directory root.

use std::path::{Component, Path, PathBuf};

use http::HeaderMap;
use http::header::{self, HeaderValue};
use pm_model::MapLocalSource;

use super::compile::percent_decode;
use super::eval::{LocalAnswer, ResponseDraft, guess_content_type, header_map};

/// Files larger than this aren't served (Map Local buffers the answer).
pub const LOCAL_FILE_MAX: u64 = 256 * 1024 * 1024;

/// The response for `local`, given the request's path. A missing file
/// answers 404 with a readable reason instead of failing the flow.
pub(crate) async fn answer(local: &LocalAnswer, request_path: &str) -> ResponseDraft {
    let (body, guessed) = match &local.source {
        MapLocalSource::Inline { body } => (Ok(body.clone().into_bytes()), None),
        MapLocalSource::File { path } => (read(Path::new(path)).await, Some(path.clone())),
        MapLocalSource::Directory { root } => match find_in(Path::new(root), request_path).await {
            Some(file) => {
                let name = file.display().to_string();
                (read(&file).await, Some(name))
            }
            None => (
                Err(format!(
                    "Proxymate Map Local: no file for {request_path} under {root}\n"
                )),
                None,
            ),
        },
    };
    let mut headers: HeaderMap = header_map(&local.headers);
    match body {
        Ok(body) => {
            let content_type = local
                .content_type
                .clone()
                .or_else(|| guessed.map(|path| guess_content_type(&path).to_owned()));
            if let Some(ct) = content_type
                && let Ok(ct) = HeaderValue::from_str(&ct)
            {
                headers.insert(header::CONTENT_TYPE, ct);
            }
            ResponseDraft {
                status: local.status,
                headers,
                body: Some(body),
            }
        }
        Err(message) => {
            headers.insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/plain; charset=utf-8"),
            );
            ResponseDraft {
                status: 404,
                headers,
                body: Some(message.into_bytes()),
            }
        }
    }
}

async fn read(path: &Path) -> Result<Vec<u8>, String> {
    let meta = tokio::fs::metadata(path)
        .await
        .map_err(|e| format!("Proxymate Map Local: can't read {}: {e}\n", path.display()))?;
    if meta.len() > LOCAL_FILE_MAX {
        return Err(format!(
            "Proxymate Map Local: {} is larger than {} MB\n",
            path.display(),
            LOCAL_FILE_MAX / 1024 / 1024
        ));
    }
    tokio::fs::read(path)
        .await
        .map_err(|e| format!("Proxymate Map Local: can't read {}: {e}\n", path.display()))
}

/// The file under `root` for `request_path`: the whole path first, then
/// with leading segments dropped. `..` never leaves the root.
pub(crate) async fn find_in(root: &Path, request_path: &str) -> Option<PathBuf> {
    let mut segments: Vec<String> = request_path
        .split('/')
        .filter(|s| !s.is_empty())
        .map(percent_decode)
        .collect();
    if segments.iter().any(|s| !is_plain_segment(s)) {
        return None;
    }
    if request_path.ends_with('/') || segments.is_empty() {
        segments.push("index.html".into());
    }
    for skip in 0..segments.len() {
        let candidate = segments[skip..].iter().fold(root.to_path_buf(), |p, s| p.join(s));
        if tokio::fs::metadata(&candidate).await.is_ok_and(|m| m.is_file()) {
            return Some(candidate);
        }
    }
    None
}

fn is_plain_segment(segment: &str) -> bool {
    let mut components = Path::new(segment).components();
    matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none() && !segment.contains('\\')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn directory_paths_fall_back_to_shorter_suffixes() {
        let root = std::env::temp_dir().join(format!("pm-local-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("v1")).unwrap();
        std::fs::write(root.join("v1/users.json"), b"[]").unwrap();
        std::fs::write(root.join("index.html"), b"<p>").unwrap();

        assert_eq!(
            find_in(&root, "/api/v1/users.json").await,
            Some(root.join("v1/users.json"))
        );
        assert_eq!(find_in(&root, "/v1/users.json").await, Some(root.join("v1/users.json")));
        assert_eq!(find_in(&root, "/").await, Some(root.join("index.html")));
        assert_eq!(find_in(&root, "/nope.json").await, None);
        assert_eq!(find_in(&root, "/../etc/passwd").await, None);
        assert_eq!(find_in(&root, "/%2e%2e/etc/passwd").await, None);
        let _ = std::fs::remove_dir_all(root);
    }
}
