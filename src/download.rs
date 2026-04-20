use anyhow::{Context, Result};
use futures_util::StreamExt;
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

/// Extract the first path segment after any of the given prefixes (scheme-agnostic).
fn strip_segment<'a>(url: &'a str, prefixes: &[&str]) -> Option<&'a str> {
    let bare = url
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_start_matches("www.");
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
    let client = http::client();
    let mut req = client.get(format!("{}/{}", GOFILE_CONTENTS_API_BASE, code));
    if let Some(t) = token::normalize_ref(token) {
        req = req.header("Authorization", format!("Bearer {}", t));
    }

    let response = req
        .send()
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
    let response = http::client()
        .get(format!("{}/{}", ROOTZ_DOWNLOAD_API_BASE, id))
        .send()
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

const FICHIER_DOMAINS: &[&str] = &[
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

fn is_1fichier_url(url: &str) -> bool {
    let bare = url
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_start_matches("www.");
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

    let response = http::client()
        .post(FICHIER_DOWNLOAD_TOKEN_API)
        .header("Authorization", format!("Bearer {}", token))
        .json(&serde_json::json!({ "url": url }))
        .send()
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
    let response = http::client()
        .get(url)
        .send()
        .await
        .with_context(|| format!("Download request failed for URL: {url}"))?;

    let status = response.status();
    if !status.is_success() {
        let body = http::read_error_body(response).await;
        anyhow::bail!("Download failed (HTTP {}): {}", status, body.trim());
    }

    let file_name = match output {
        Some(path) => {
            if path.is_dir() {
                anyhow::bail!("Output path points to a directory: {}", path.display());
            }
            path.to_path_buf()
        }
        None => {
            let name = name_hint
                .and_then(sanitize_file_name)
                .or_else(|| {
                    content_disposition_filename(response.headers())
                        .and_then(|value| sanitize_file_name(&value))
                })
                .or_else(|| url_filename(url).and_then(|value| sanitize_file_name(&value)))
                .unwrap_or_else(|| "download".to_string());
            PathBuf::from(name)
        }
    };

    let total = response.content_length();
    let bar = match total {
        Some(n) => progress::new_bar(n, &file_name.to_string_lossy()),
        None => progress::new_bar_unknown(&file_name.to_string_lossy()),
    };

    let mut dest = tokio::fs::File::create(&file_name)
        .await
        .with_context(|| format!("Could not create output file: {}", file_name.display()))?;

    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.with_context(|| format!("Error reading response body from {url}"))?;
        bar.inc(chunk.len() as u64);
        dest.write_all(&chunk)
            .await
            .with_context(|| format!("Failed writing to {}", file_name.display()))?;
    }

    dest.flush()
        .await
        .with_context(|| format!("Failed flushing file {}", file_name.display()))?;
    bar.finish_and_clear();

    Ok(file_name)
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
    use super::{sanitize_file_name, strip_segment, validate_download_url};

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
}
