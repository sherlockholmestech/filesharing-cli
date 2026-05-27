use anyhow::{Context, Result};
use reqwest::Client;
use std::path::Path;

use crate::{http, progress, token};

const LITTERBOX_UPLOAD_URL: &str = "https://litterbox.catbox.moe/resources/internals/api.php";

pub struct Litterbox {
    client: Client,
    token: Option<String>,
}

impl Litterbox {
    pub fn new(token: Option<String>) -> Self {
        Self {
            client: http::client().clone(),
            token: token::normalize(token),
        }
    }

    pub async fn upload(&self, file_path: &Path, time: Option<&str>) -> Result<String> {
        let file_name = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .context("File has no valid name")?
            .to_string();

        let file = tokio::fs::File::open(file_path)
            .await
            .with_context(|| format!("Could not open file: {}", file_path.display()))?;
        let file_size = file.metadata().await?.len();

        let bar = progress::new_bar(file_size, &file_name);
        let body = reqwest::Body::wrap(progress::wrap_body(file, file_size, bar.clone()));

        let file_part =
            reqwest::multipart::Part::stream_with_length(body, file_size).file_name(file_name);

        let mut form = reqwest::multipart::Form::new()
            .text("reqtype", "fileupload")
            .part("fileToUpload", file_part);

        if let Some(time_val) = time {
            if !["1h", "12h", "24h", "72h"].contains(&time_val) {
                anyhow::bail!(
                    "Invalid Litterbox time value '{}'. Must be one of: 1h, 12h, 24h, 72h",
                    time_val
                );
            }
            form = form.text("time", time_val.to_string());
        } else {
            form = form.text("time", "24h".to_string());
        }

        let mut req = self.client.post(LITTERBOX_UPLOAD_URL).multipart(form);

        if let Some(userhash) = &self.token {
            req = req.header("Authorization", format!("Bearer {}", userhash));
        }

        let response = req
            .send()
            .await
            .context("Litterbox upload request failed")?;
        bar.finish_and_clear();

        let status = response.status();
        if !status.is_success() {
            let body = http::read_error_body(response).await;
            anyhow::bail!("Litterbox upload failed (HTTP {}): {}", status, body.trim());
        }

        let body = response
            .text()
            .await
            .context("Could not read Litterbox response body")?;

        extract_url(&body).with_context(|| format!("Unexpected response: {}", body.trim()))
    }
}

fn extract_url(body: &str) -> Option<String> {
    let trimmed = body.trim();
    if trimmed.starts_with("http") {
        return Some(trimmed.to_string());
    }
    None
}
