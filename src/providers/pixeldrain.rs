use anyhow::{Context, Result};
use reqwest::Client;
use serde::Deserialize;
use std::path::Path;

use crate::progress;

pub struct Pixeldrain {
    client: Client,
    token: Option<String>,
}

#[derive(Deserialize)]
struct UploadResponse {
    id: String,
}

impl Pixeldrain {
    pub fn new(token: Option<String>) -> Self {
        Self {
            client: Client::new(),
            token,
        }
    }

    pub async fn upload(&self, file_path: &Path) -> Result<String> {
        let file_name = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .context("File has no valid name")?;

        let file = tokio::fs::File::open(file_path)
            .await
            .with_context(|| format!("Could not open file: {}", file_path.display()))?;
        let file_size = file.metadata().await?.len();

        let bar = progress::new_bar(file_size, file_name);
        let body = reqwest::Body::wrap(progress::wrap_body(file, bar.clone()));

        let url = format!("https://pixeldrain.com/api/file/{}", file_name);
        let mut req = self.client.put(&url).header("Content-Length", file_size).body(body);

        // Basic auth: empty username, API key as password.
        if let Some(token) = &self.token {
            req = req.basic_auth("", Some(token));
        }

        let response = req.send().await.context("Request failed")?;
        bar.finish_and_clear();

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            if status == reqwest::StatusCode::UNAUTHORIZED {
                anyhow::bail!(
                    "Pixeldrain requires an API key. Pass it with --token YOUR_API_KEY\n\
                     Get one at: https://pixeldrain.com/user/api_keys"
                );
            }
            anyhow::bail!("Upload failed (HTTP {}): {}", status, body.trim());
        }

        let resp: UploadResponse = response
            .json()
            .await
            .context("Could not parse response")?;

        Ok(format!("https://pixeldrain.com/u/{}", resp.id))
    }
}
