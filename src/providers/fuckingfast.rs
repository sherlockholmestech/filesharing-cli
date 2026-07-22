use anyhow::{Context, Result};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use reqwest::{Client, Url};
use std::path::Path;

use crate::{http, progress, token};

const FF_BASE_URL: &str = "https://w.fuckingfast.net";

pub struct FuckingFast {
    client: Client,
    token: Option<String>,
}

impl FuckingFast {
    pub fn new(token: Option<String>) -> Self {
        Self {
            client: http::client().clone(),
            token: token::normalize(token),
        }
    }

    pub async fn upload(
        &self,
        file_path: &Path,
        note: Option<&str>,
        parent_id: Option<&str>,
    ) -> Result<String> {
        let file_name = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .context("File has no valid name")?;

        let url = upload_url(file_name, note, parent_id)?;

        let file = tokio::fs::File::open(file_path)
            .await
            .with_context(|| format!("Could not open file: {}", file_path.display()))?;
        let file_size = file.metadata().await?.len();

        let bar = progress::new_bar(file_size, file_name);
        let body = reqwest::Body::wrap(progress::wrap_body(file, file_size, bar.clone()));

        let mut req = self
            .client
            .put(url)
            .header("Content-Length", file_size)
            .body(body);
        if let Some(token) = &self.token {
            req = req.header("Authorization", format!("Bearer {}", token));
        }

        let response = req
            .send()
            .await
            .context("FuckingFast upload request failed");
        bar.finish_and_clear();
        let response = response?;

        let status = response.status();
        if !status.is_success() {
            let body = http::read_error_body(response).await;
            anyhow::bail!(
                "FuckingFast upload failed (HTTP {}): {}",
                status,
                body.trim()
            );
        }

        let body = response
            .text()
            .await
            .context("Could not read FuckingFast response body")?;

        extract_url(&body).with_context(|| format!("Unexpected response: {}", body.trim()))
    }
}

fn extract_url(body: &str) -> Option<String> {
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(body) {
        let data = json.get("data").unwrap_or(&json);
        for key in &["url", "shortUrl", "download_url", "link"] {
            if let Some(v) = data.get(key).and_then(|v| v.as_str()) {
                return Some(v.to_string());
            }
        }
        if let Some(id) = data.get("id").and_then(|v| v.as_str()) {
            return Some(format!("https://fuckingfast.net/{id}"));
        }
    }
    let trimmed = body.trim();
    if trimmed.starts_with("http") {
        return Some(trimmed.to_string());
    }
    None
}

fn upload_url(file_name: &str, note: Option<&str>, parent_id: Option<&str>) -> Result<Url> {
    let mut url = Url::parse(FF_BASE_URL).context("Invalid FuckingFast upload URL")?;
    {
        let mut segments = url
            .path_segments_mut()
            .map_err(|_| anyhow::anyhow!("Invalid FuckingFast upload URL"))?;
        if let Some(parent_id) = parent_id {
            segments.push(parent_id);
        }
        segments.push(file_name);
    }
    if let Some(note) = note {
        url.query_pairs_mut()
            .append_pair("note", &STANDARD.encode(note));
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::{extract_url, upload_url};

    #[test]
    fn extracts_url_from_current_upload_response() {
        let body = r#"{"code":201,"data":{"id":"f752gf1jw1ne","name":"sample.txt"}}"#;

        assert_eq!(
            extract_url(body).as_deref(),
            Some("https://fuckingfast.net/f752gf1jw1ne")
        );
    }

    #[test]
    fn upload_url_encodes_path_and_note() {
        let url = upload_url("sample #1.txt", Some("note? yes"), Some("parent/id")).unwrap();

        assert_eq!(
            url.as_str(),
            "https://w.fuckingfast.net/parent%2Fid/sample%20%231.txt?note=bm90ZT8geWVz"
        );
    }
}
