use anyhow::{Context, Result};
use reqwest::Client;
use serde::Deserialize;
use std::path::Path;

use crate::{http, progress};

const TMPFILES_UPLOAD_URL: &str = "https://tmpfiles.org/api/v1/upload";

pub struct TmpFiles {
    client: Client,
}

#[derive(Deserialize)]
struct UploadResponse {
    status: String,
    data: Option<UploadData>,
}

#[derive(Deserialize)]
struct UploadData {
    url: String,
}

impl TmpFiles {
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
            .post(TMPFILES_UPLOAD_URL)
            .multipart(form)
            .send()
            .await
            .context("tmpfiles.org upload request failed");
        bar.finish_and_clear();
        let response = response?;

        let status = response.status();
        if !status.is_success() {
            let body = http::read_error_body(response).await;
            anyhow::bail!(
                "tmpfiles.org upload failed (HTTP {}): {}",
                status,
                body.trim()
            );
        }

        let response: UploadResponse = response
            .json()
            .await
            .context("Could not parse tmpfiles.org response")?;

        if response.status != "success" {
            anyhow::bail!("tmpfiles.org returned status '{}'", response.status);
        }

        response
            .data
            .map(|data| data.url)
            .context("tmpfiles.org response contained no file URL")
    }
}
