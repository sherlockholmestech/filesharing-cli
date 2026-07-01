use anyhow::{Context, Result};
use futures_util::StreamExt;
use reqwest::{
    Response,
    header::{CONTENT_RANGE, RANGE},
};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;

use crate::{http, progress, token};

const MAX_URL_LEN: usize = 8 * 1024;
const PIXELDRAIN_DOWNLOAD_API_BASE: &str = "https://pixeldrain.com/api/file";
const GOFILE_CONTENTS_API_BASE: &str = "https://api.gofile.io/contents";
const ROOTZ_DOWNLOAD_API_BASE: &str = "https://rootz.so/api/files/download";
const FICHIER_DOWNLOAD_TOKEN_API: &str = "https://api.1fichier.com/v1/download/get_token.cgi";

pub async fn download(url: &str, output: Option<&Path>, token: Option<&str>) -> Result<PathBuf> {
    validate_download_url(url)?;

    let resolved = resolve_download_url(url, token).await?;
    fetch_to_file(&resolved.url, output, resolved.file_name.as_deref()).await
}

// ── URL resolution ────────────────────────────────────────────────────────────

struct Resolved {
    url: String,
    file_name: Option<String>,
}

async fn resolve_download_url(url: &str, token: Option<&str>) -> Result<Resolved> {
    let url = url.trim();

    // pixeldrain.com/u/{id} or /l/{id}
    if let Some(id) = strip_segment(url, &["pixeldrain.com/u/", "pixeldrain.com/l/"]) {
        return Ok(Resolved {
            url: format!("{}/{}?download", PIXELDRAIN_DOWNLOAD_API_BASE, id),
            file_name: None,
        });
    }

    // gofile.io/d/{code}
    if let Some(code) = strip_segment(url, &["gofile.io/d/"]) {
        return resolve_gofile(code, token).await;
    }

    // rootz.so/d/{shortId}
    if let Some(id) = strip_segment(url, &["rootz.so/d/"]) {
        return resolve_rootz(id).await;
    }

    // 1fichier and its mirror domains
    if is_1fichier_url(url) {
        return resolve_1fichier(url, token).await;
    }

    // catbox.moe and litterbox.catbox.moe — direct file URLs
    if url.contains("catbox.moe") || url.contains("litterbox.catbox.moe") {
        // These are direct file URLs, download as-is
        return Ok(Resolved {
            url: url.to_string(),
            file_name: None,
        });
    }

    // fuckingfast and vikingfile have no documented download API.
    if url.contains("fuckingfast.net") {
        anyhow::bail!(
            "fuckingfast does not provide a documented download API. \
             Use the direct file URL if you have one."
        );
    }
    if url.contains("vikingfile.com") {
        anyhow::bail!(
            "vikingfile does not provide a documented download API. \
             Use the direct file URL if you have one."
        );
    }

    // Generic URL — download as-is.
    Ok(Resolved {
        url: url.to_string(),
        file_name: None,
    })
}

/// Extract the first path segment after any of the given prefixes.
/// Matches against the URL host (after `://` and optional `www.`) to avoid
/// false positives where the prefix appears later in the path.
fn strip_segment<'a>(url: &'a str, prefixes: &[&str]) -> Option<&'a str> {
    let after_scheme = url.split("://").nth(1).unwrap_or(url);
    let bare = after_scheme.strip_prefix("www.").unwrap_or(after_scheme);

    for prefix in prefixes {
        if let Some(rest) = bare.strip_prefix(prefix) {
            let segment = rest.split(['/', '?', '#']).next().unwrap_or(rest);
            if is_valid_share_id(segment) {
                return Some(segment);
            }
        }
    }
    None
}

fn is_valid_share_id(segment: &str) -> bool {
    if segment.is_empty() || segment.len() > 256 {
        return false;
    }

    segment
        .as_bytes()
        .iter()
        .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'-' || *byte == b'_')
}

// ── Gofile ────────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct GofileContents {
    status: String,
    data: Option<GofileData>,
}

#[derive(Deserialize)]
struct GofileData {
    children: Option<std::collections::HashMap<String, GofileChild>>,
}

#[derive(Deserialize)]
struct GofileChild {
    #[serde(rename = "type")]
    kind: String,
    name: Option<String>,
    link: Option<String>,
}

async fn resolve_gofile(code: &str, token: Option<&str>) -> Result<Resolved> {
    let response = http::send_retrying(
        || {
            let mut req = http::client().get(format!("{}/{}", GOFILE_CONTENTS_API_BASE, code));
            if let Some(t) = token::normalize_ref(token) {
                req = req.header("Authorization", format!("Bearer {}", t));
            }
            req
        },
        "Gofile API request",
    )
    .await
    .with_context(|| format!("Gofile API request failed for folder {code}"))?;

    let status = response.status();
    if !status.is_success() {
        let body = http::read_error_body(response).await;
        anyhow::bail!(
            "Gofile API request failed (HTTP {}) for folder {}: {}",
            status,
            code,
            body.trim()
        );
    }

    let resp: GofileContents = response
        .json()
        .await
        .context("Could not parse Gofile response")?;

    if resp.status != "ok" {
        anyhow::bail!("Gofile returned non-ok status — folder may be private (try --token)");
    }

    let children = resp
        .data
        .and_then(|d| d.children)
        .context("No children in Gofile response")?;

    let file = children
        .values()
        .find(|c| c.kind == "file")
        .context("No files found in Gofile folder")?;

    let url = file
        .link
        .clone()
        .context("Gofile file entry has no download link")?;

    Ok(Resolved {
        url,
        file_name: file.name.clone(),
    })
}

// ── Rootz ─────────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct RootzDownload {
    success: bool,
    data: Option<RootzDownloadData>,
    error: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RootzDownloadData {
    url: String,
    file_name: Option<String>,
}

async fn resolve_rootz(id: &str) -> Result<Resolved> {
    let response = http::send_retrying(
        || http::client().get(format!("{}/{}", ROOTZ_DOWNLOAD_API_BASE, id)),
        "Rootz API request",
    )
    .await
    .with_context(|| format!("Rootz API request failed for share ID {id}"))?;

    let status = response.status();
    if !status.is_success() {
        let body = http::read_error_body(response).await;
        anyhow::bail!(
            "Rootz API request failed (HTTP {}) for share ID {}: {}",
            status,
            id,
            body.trim()
        );
    }

    let resp: RootzDownload = response
        .json()
        .await
        .context("Could not parse Rootz response")?;

    if !resp.success {
        anyhow::bail!(
            "Rootz download failed: {}. \
             Note: the download API requires the file UUID, not the short ID — \
             this may not work for all share links.",
            resp.error.as_deref().unwrap_or("unknown error")
        );
    }

    let data = resp.data.context("No data in Rootz response")?;
    Ok(Resolved {
        url: data.url,
        file_name: data.file_name,
    })
}

// ── 1fichier ──────────────────────────────────────────────────────────────────
//
// 1fichier hosts files on several domains.  All share the same /?<id> URL
// shape.  Downloading via the API requires a Premium account API key; the
// token returned is valid for ~5 minutes.

pub const FICHIER_DOMAINS: &[&str] = &[
    "1fichier.com/?",
    "alterupload.com/?",
    "cjoint.net/?",
    "desfichiers.com/?",
    "dfichiers.com/?",
    "megadl.fr/?",
    "mesfichiers.org/?",
    "piecejointe.net/?",
    "pjointe.com/?",
    "tenvoi.com/?",
    "dl4free.com/?",
];

pub fn is_1fichier_url(url: &str) -> bool {
    let after_scheme = url.split("://").nth(1).unwrap_or(url);
    let bare = after_scheme.strip_prefix("www.").unwrap_or(after_scheme);
    FICHIER_DOMAINS.iter().any(|d| bare.starts_with(d))
}

#[derive(Deserialize)]
struct FichierTokenResponse {
    url: Option<String>,
    status: String,
    message: Option<String>,
}

async fn resolve_1fichier(url: &str, token: Option<&str>) -> Result<Resolved> {
    let token = token::normalize_ref(token).ok_or_else(|| {
        anyhow::anyhow!(
            "1fichier download requires a Premium API key (--token YOUR_KEY).\n\
             You can also set FSC_1FICHIER_TOKEN or FSC_TOKEN\n\
             Obtain one at: https://1fichier.com/console/params.pl"
        )
    })?;

    let response = http::send_retrying(
        || {
            http::client()
                .post(FICHIER_DOWNLOAD_TOKEN_API)
                .header("Authorization", format!("Bearer {}", token))
                .json(&serde_json::json!({ "url": url }))
        },
        "1fichier token request",
    )
    .await
    .context("1fichier token request failed")?;

    let status = response.status();
    if !status.is_success() {
        let body = http::read_error_body(response).await;
        anyhow::bail!(
            "1fichier token request failed (HTTP {}): {}",
            status,
            body.trim()
        );
    }

    let resp: FichierTokenResponse = response
        .json()
        .await
        .context("Could not parse 1fichier token response")?;

    if resp.status != "OK" {
        anyhow::bail!(
            "1fichier download failed: {}",
            resp.message.as_deref().unwrap_or("unknown error")
        );
    }

    let download_url = resp
        .url
        .context("1fichier token response contained no download URL")?;

    Ok(Resolved {
        url: download_url,
        file_name: None,
    })
}

// ── HTTP fetch ────────────────────────────────────────────────────────────────

async fn fetch_to_file(
    url: &str,
    output: Option<&Path>,
    name_hint: Option<&str>,
) -> Result<PathBuf> {
    let mut initial_response = None;

    let file_name = match output {
        Some(path) => {
            if path.is_dir() {
                anyhow::bail!("Output path points to a directory: {}", path.display());
            }
            let file_name = path.to_path_buf();
            if partial_len(&partial_path(&file_name)).await? == 0 {
                initial_response = Some(request_download_range(url, 0).await?);
            }
            file_name
        }
        None => {
            let response = request_download_range(url, 0).await?;
            let name = name_hint
                .and_then(sanitize_file_name)
                .or_else(|| {
                    content_disposition_filename(response.headers())
                        .and_then(|value| sanitize_file_name(&value))
                })
                .or_else(|| url_filename(url).and_then(|value| sanitize_file_name(&value)))
                .unwrap_or_else(|| "download".to_string());
            initial_response = Some(response);
            PathBuf::from(name)
        }
    };

    let part_file_name = partial_path(&file_name);

    for attempt in 0..=crate::settings::http_retry_attempts() {
        let existing_len = partial_len(&part_file_name).await?;
        let resume_from = existing_len;
        let response = match initial_response.take().filter(|_| resume_from == 0) {
            Some(response) => response,
            None => request_download_range(url, resume_from).await?,
        };

        match write_download_response(url, response, &file_name, &part_file_name, resume_from).await
        {
            Ok(()) => {
                tokio::fs::rename(&part_file_name, &file_name)
                    .await
                    .with_context(|| {
                        format!(
                            "Failed moving {} to {}",
                            part_file_name.display(),
                            file_name.display()
                        )
                    })?;
                return Ok(file_name);
            }
            Err(err) if attempt < crate::settings::http_retry_attempts() => {
                eprintln!(
                    "{}",
                    crate::style::warn(format!(
                        "Download interrupted, retrying from byte {}: {}",
                        partial_len(&part_file_name).await.unwrap_or(0),
                        err
                    ))
                );
            }
            Err(err) => return Err(err),
        }
    }

    unreachable!("download retry loop always returns")
}

async fn request_download_range(url: &str, start: u64) -> Result<Response> {
    let response = http::send_retrying(
        || {
            let mut req = http::client().get(url);
            if start > 0 {
                req = req.header(RANGE, format!("bytes={start}-"));
            }
            req
        },
        "Download request",
    )
    .await
    .with_context(|| format!("Download request failed for URL: {url}"))?;

    let status = response.status();
    if status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE && start > 0 {
        let response = http::send_retrying(|| http::client().get(url), "Download restart request")
            .await
            .with_context(|| format!("Download restart request failed for URL: {url}"))?;
        let status = response.status();
        if !status.is_success() {
            let body = http::read_error_body(response).await;
            anyhow::bail!("Download failed (HTTP {}): {}", status, body.trim());
        }
        return Ok(response);
    }
    if !status.is_success() {
        let body = http::read_error_body(response).await;
        anyhow::bail!("Download failed (HTTP {}): {}", status, body.trim());
    }

    Ok(response)
}

async fn write_download_response(
    url: &str,
    response: Response,
    file_name: &Path,
    part_file_name: &Path,
    requested_resume_from: u64,
) -> Result<()> {
    let status = response.status();
    let resume_from = if requested_resume_from > 0 {
        if status == reqwest::StatusCode::PARTIAL_CONTENT {
            let actual_start = content_range_start(response.headers());
            if actual_start != Some(requested_resume_from) {
                tokio::fs::remove_file(part_file_name)
                    .await
                    .with_context(|| {
                        format!(
                            "Could not discard invalid partial file {}",
                            part_file_name.display()
                        )
                    })?;
                anyhow::bail!(
                    "Download server returned unexpected Content-Range start {:?}; expected {}",
                    actual_start,
                    requested_resume_from
                );
            }
            requested_resume_from
        } else {
            eprintln!(
                "{}",
                crate::style::warn(
                    "Download server ignored resume request; restarting from byte 0"
                )
            );
            0
        }
    } else {
        0
    };

    let total = if status == reqwest::StatusCode::PARTIAL_CONTENT {
        content_range_total(response.headers()).or_else(|| {
            response
                .content_length()
                .map(|remaining| remaining + requested_resume_from)
        })
    } else {
        response.content_length()
    };

    let bar = match total {
        Some(n) => {
            let bar = progress::new_bar(n, &file_name.to_string_lossy());
            bar.set_position(resume_from);
            bar
        }
        None => {
            let bar = progress::new_bar_unknown(&file_name.to_string_lossy());
            bar.inc(resume_from);
            bar
        }
    };

    let result = stream_to_part(url, response, part_file_name, resume_from > 0, &bar).await;
    bar.finish_and_clear();
    result
}

async fn stream_to_part(
    url: &str,
    response: Response,
    part_file_name: &Path,
    append: bool,
    bar: &indicatif::ProgressBar,
) -> Result<()> {
    let mut dest = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(append)
        .truncate(!append)
        .open(part_file_name)
        .await
        .with_context(|| format!("Could not create output file: {}", part_file_name.display()))?;

    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.with_context(|| format!("Error reading response body from {url}"))?;
        bar.inc(chunk.len() as u64);
        dest.write_all(&chunk)
            .await
            .with_context(|| format!("Failed writing to {}", part_file_name.display()))?;
    }

    dest.flush()
        .await
        .with_context(|| format!("Failed flushing file {}", part_file_name.display()))
}

async fn partial_len(path: &Path) -> Result<u64> {
    match tokio::fs::metadata(path).await {
        Ok(meta) => Ok(meta.len()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(err) => Err(err).with_context(|| format!("Could not inspect {}", path.display())),
    }
}

fn partial_path(path: &Path) -> PathBuf {
    let mut part = path.to_path_buf();
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("download");
    part.set_file_name(format!("{name}.part"));
    part
}

fn content_range_total(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    let value = headers.get(CONTENT_RANGE)?.to_str().ok()?;
    let total = value.rsplit_once('/')?.1;
    if total == "*" {
        None
    } else {
        total.parse().ok()
    }
}

fn content_range_start(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    let value = headers.get(CONTENT_RANGE)?.to_str().ok()?;
    let range = value.strip_prefix("bytes ")?;
    let (start, _) = range.split_once('-')?;
    start.parse().ok()
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn content_disposition_filename(headers: &reqwest::header::HeaderMap) -> Option<String> {
    let value = headers
        .get(reqwest::header::CONTENT_DISPOSITION)?
        .to_str()
        .ok()?;
    value.split(';').find_map(|part| {
        let key = part.trim().strip_prefix("filename=")?;
        Some(key.trim_matches('"').to_string())
    })
}

fn url_filename(url: &str) -> Option<String> {
    let path = url.split('?').next()?;
    let segment = path.rsplit('/').find(|s| !s.is_empty())?;
    if segment.is_empty() {
        None
    } else {
        Some(segment.to_string())
    }
}

fn sanitize_file_name(raw: &str) -> Option<String> {
    let normalized = raw.trim().replace('\\', "/");
    let leaf = normalized.rsplit('/').next()?.trim();

    if leaf.is_empty() || leaf == "." || leaf == ".." {
        return None;
    }

    let cleaned: String = leaf
        .chars()
        .filter(|ch| !ch.is_control() && *ch != '/' && *ch != '\\')
        .collect();

    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

fn validate_download_url(url: &str) -> Result<()> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        anyhow::bail!("Download URL cannot be empty");
    }
    if trimmed.len() > MAX_URL_LEN {
        anyhow::bail!("Download URL is too long (maximum {MAX_URL_LEN} bytes)");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        content_range_start, content_range_total, partial_path, sanitize_file_name, strip_segment,
        validate_download_url,
    };
    use reqwest::header::{CONTENT_RANGE, HeaderMap};
    use std::path::Path;

    #[test]
    fn strip_segment_rejects_invalid_share_id() {
        assert_eq!(
            strip_segment("https://gofile.io/d/abc_DEF-123", &["gofile.io/d/"]),
            Some("abc_DEF-123")
        );
        assert_eq!(
            strip_segment("https://gofile.io/d/../evil", &["gofile.io/d/"]),
            None
        );
    }

    #[test]
    fn strip_segment_rejects_prefix_in_path() {
        assert_eq!(
            strip_segment("http://evil.com/gofile.io/d/abc", &["gofile.io/d/"]),
            None
        );
        assert_eq!(
            strip_segment("https://evil.com/www.gofile.io/d/abc", &["gofile.io/d/"]),
            None
        );
    }

    #[test]
    fn sanitize_file_name_keeps_leaf_only() {
        assert_eq!(
            sanitize_file_name("folder/sub/file.txt"),
            Some("file.txt".to_string())
        );
        assert_eq!(sanitize_file_name(".."), None);
        assert_eq!(sanitize_file_name(""), None);
    }

    #[test]
    fn validates_download_url_length() {
        assert!(validate_download_url("https://example.com/file").is_ok());
        let long = format!("https://example.com/{}", "a".repeat(9_000));
        assert!(validate_download_url(&long).is_err());
    }

    #[test]
    fn parses_content_range_total() {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_RANGE, "bytes 100-199/1000".parse().unwrap());
        assert_eq!(content_range_total(&headers), Some(1000));

        headers.insert(CONTENT_RANGE, "bytes 100-199/*".parse().unwrap());
        assert_eq!(content_range_total(&headers), None);
    }

    #[test]
    fn parses_content_range_start() {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_RANGE, "bytes 100-199/1000".parse().unwrap());
        assert_eq!(content_range_start(&headers), Some(100));
    }

    #[test]
    fn partial_path_appends_part_suffix() {
        assert_eq!(
            partial_path(Path::new("/tmp/archive.tar.gz")),
            Path::new("/tmp/archive.tar.gz.part")
        );
    }
}
