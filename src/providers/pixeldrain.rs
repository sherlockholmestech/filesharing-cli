use anyhow::{Context, Result};
use reqwest::Client;
use serde::Deserialize;
use std::path::Path;

use crate::{http, progress, token};

const PIXELDRAIN_UPLOAD_API_BASE: &str = "https://pixeldrain.com/api/file";
const PIXELDRAIN_SHARE_BASE: &str = "https://pixeldrain.com/u";

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
            client: http::client().clone(),
            token: token::normalize(token),
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
        let body = reqwest::Body::wrap(progress::wrap_body(file, file_size, bar.clone()));

        let url = format!("{}/{}", PIXELDRAIN_UPLOAD_API_BASE, file_name);
        let mut req = self
            .client
            .put(&url)
            .header("Content-Length", file_size)
            .body(body);

        // Basic auth: empty username, API key as password.
        if let Some(token) = &self.token {
            req = req.basic_auth("", Some(token));
        }

        let response = req
            .send()
            .await
            .context("Pixeldrain upload request failed")?;
        bar.finish_and_clear();

        let status = response.status();
        if !status.is_success() {
            if status == reqwest::StatusCode::UNAUTHORIZED {
                anyhow::bail!(
                    "Pixeldrain requires an API key. Pass it with --token YOUR_API_KEY\n\
                     You can also set FSC_PIXELDRAIN_TOKEN or FSC_TOKEN\n\
                     Get one at: https://pixeldrain.com/user/api_keys"
                );
            }
            let body = http::read_error_body(response).await;
            anyhow::bail!("Upload failed (HTTP {}): {}", status, body.trim());
        }

        let resp: UploadResponse = response.json().await.context("Could not parse response")?;

        Ok(format!("{}/{}", PIXELDRAIN_SHARE_BASE, resp.id))
    }
}
