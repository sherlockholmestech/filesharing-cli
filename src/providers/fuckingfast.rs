use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use reqwest::Client;
use std::path::Path;

use crate::progress;

pub struct FuckingFast {
    client: Client,
    token: Option<String>,
}

impl FuckingFast {
    pub fn new(token: Option<String>) -> Self {
        Self {
            client: Client::new(),
            token,
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

        let base = match parent_id {
            Some(pid) => format!("https://w.fuckingfast.net/{}/{}", pid, file_name),
            None => format!("https://w.fuckingfast.net/{}", file_name),
        };

        let url = match note {
            Some(text) => format!("{}?note={}", base, STANDARD.encode(text)),
            None => base,
        };

        let file = tokio::fs::File::open(file_path)
            .await
            .with_context(|| format!("Could not open file: {}", file_path.display()))?;
        let file_size = file.metadata().await?.len();

        let bar = progress::new_bar(file_size, file_name);
        let body = reqwest::Body::wrap(progress::wrap_body(file, bar.clone()));

        let mut req = self
            .client
            .put(&url)
            .header("Content-Length", file_size)
            .body(body);
        if let Some(token) = &self.token {
            req = req.header("Authorization", format!("Bearer {}", token));
        }

        let response = req.send().await.context("Request failed")?;
        bar.finish_and_clear();

        let status = response.status();
        let body = response.text().await.context("Could not read response")?;

        if !status.is_success() {
            anyhow::bail!("Upload failed (HTTP {}): {}", status, body.trim());
        }

        extract_url(&body).with_context(|| format!("Unexpected response: {}", body.trim()))
    }
}

fn extract_url(body: &str) -> Option<String> {
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(body) {
        for key in &["url", "shortUrl", "download_url", "link"] {
            if let Some(v) = json.get(key).and_then(|v| v.as_str()) {
                return Some(v.to_string());
            }
        }
    }
    let trimmed = body.trim();
    if trimmed.starts_with("http") {
        return Some(trimmed.to_string());
    }
    None
}
