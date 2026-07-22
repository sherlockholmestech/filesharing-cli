use anyhow::{Context, Result};
use reqwest::Client;
use std::path::Path;

use crate::{http, progress};

const TEMP_SH_UPLOAD_URL: &str = "https://temp.sh/upload";

pub struct TempSh {
    client: Client,
}

impl TempSh {
    pub fn new() -> Self {
        Self {
            client: http::client().clone(),
        }
    }

    pub async fn upload(&self, file_path: &Path) -> Result<String> {
        let file_name = file_path
            .file_name()
            .and_then(|name| name.to_str())
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
        let form = reqwest::multipart::Form::new().part("file", file_part);

        let response = self
            .client
            .post(TEMP_SH_UPLOAD_URL)
            .multipart(form)
            .send()
            .await
            .context("temp.sh upload request failed");
        bar.finish_and_clear();
        let response = response?;

        let status = response.status();
        if !status.is_success() {
            let body = http::read_error_body(response).await;
            anyhow::bail!("temp.sh upload failed (HTTP {}): {}", status, body.trim());
        }

        let body = response
            .text()
            .await
            .context("Could not read temp.sh response body")?;
        let url = body.trim();
        if !url.starts_with("http") {
            anyhow::bail!("Unexpected temp.sh response: {url}");
        }

        Ok(url.to_string())
    }
}
