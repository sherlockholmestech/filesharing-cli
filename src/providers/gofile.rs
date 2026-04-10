use anyhow::{Context, Result};
use reqwest::Client;
use serde::Deserialize;
use std::path::Path;

use crate::progress;

pub struct Gofile {
    client: Client,
    token: Option<String>,
}

#[derive(Deserialize)]
struct UploadResponse {
    status: String,
    data: Option<UploadData>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UploadData {
    download_page: Option<String>,
}

impl Gofile {
    pub fn new(token: Option<String>) -> Self {
        Self {
            client: Client::new(),
            token,
        }
    }

    pub async fn upload(&self, file_path: &Path, folder_id: Option<&str>) -> Result<String> {
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
        let body = reqwest::Body::wrap(progress::wrap_body(file, bar.clone()));

        let file_part = reqwest::multipart::Part::stream_with_length(body, file_size)
            .file_name(file_name);

        let mut form = reqwest::multipart::Form::new().part("file", file_part);
        if let Some(fid) = folder_id {
            form = form.text("folderId", fid.to_string());
        }

        let mut req = self
            .client
            .post("https://upload.gofile.io/uploadfile")
            .multipart(form);

        if let Some(token) = &self.token {
            req = req.header("Authorization", format!("Bearer {}", token));
        }

        let response = req.send().await.context("Request failed")?;
        bar.finish_and_clear();

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Upload failed (HTTP {}): {}", status, body.trim());
        }

        let resp: UploadResponse = response
            .json()
            .await
            .context("Could not parse response")?;

        if resp.status != "ok" {
            anyhow::bail!("Gofile returned non-ok status: {}", resp.status);
        }

        resp.data
            .and_then(|d| d.download_page)
            .context("Response contained no download URL")
    }
}
