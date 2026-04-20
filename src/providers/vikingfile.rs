use anyhow::{Context, Result};
use reqwest::Client;
use serde::Deserialize;
use std::path::Path;
use std::sync::Arc;
use tokio::io::AsyncReadExt;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::{http, progress, settings, token};

const VIKING_GET_UPLOAD_URL_API: &str = "https://vikingfile.com/api/get-upload-url";
const VIKING_COMPLETE_UPLOAD_API: &str = "https://vikingfile.com/api/complete-upload";

pub struct VikingFile {
    client: Client,
    token: Option<String>,
}

// ── get-upload-url ────────────────────────────────────────────────────────────

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UploadUrlResponse {
    upload_id: String,
    key: String,
    part_size: u64,
    number_parts: u32,
    urls: Vec<String>,
}

// ── complete-upload ───────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct CompleteResponse {
    url: String,
}

struct PartInfo {
    part_number: u32,
    etag: String,
}

// ─────────────────────────────────────────────────────────────────────────────

impl VikingFile {
    pub fn new(token: Option<String>) -> Self {
        Self {
            client: http::client().clone(),
            token: token::normalize(token),
        }
    }

    pub async fn upload(&self, file_path: &Path, folder: Option<&str>) -> Result<String> {
        let file_name = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .context("File has no valid name")?
            .to_string();

        let file_size = tokio::fs::metadata(file_path).await?.len();

        // Step 1 — get presigned part URLs.
        let resp = self
            .client
            .post(VIKING_GET_UPLOAD_URL_API)
            .form(&[("size", file_size.to_string())])
            .send()
            .await
            .context("get-upload-url request failed")?;

        let status = resp.status();
        if !status.is_success() {
            let body = http::read_error_body(resp).await;
            anyhow::bail!("get-upload-url failed (HTTP {}): {}", status, body.trim());
        }

        let body_text = resp
            .text()
            .await
            .context("Could not read get-upload-url response")?;
        let init: UploadUrlResponse = serde_json::from_str(&body_text).with_context(|| {
            format!(
                "Could not parse get-upload-url response: {}",
                body_text.trim()
            )
        })?;

        if init.part_size == 0 {
            anyhow::bail!("VikingFile returned invalid part size 0");
        }
        if init.number_parts == 0 || init.urls.is_empty() {
            anyhow::bail!("VikingFile returned no presigned part URLs");
        }
        if init.urls.len() != init.number_parts as usize {
            anyhow::bail!(
                "VikingFile returned mismatched part metadata: {} urls for {} parts",
                init.urls.len(),
                init.number_parts
            );
        }

        // Step 2 — upload parts in parallel.
        let parallelism = settings::upload_parallelism(file_size);
        let sem = Arc::new(Semaphore::new(parallelism));
        let bar = progress::new_bar(file_size, &file_name);
        let mut file = tokio::fs::File::open(file_path).await?;
        let mut tasks: JoinSet<Result<PartInfo>> = JoinSet::new();

        for (idx, url) in init.urls.iter().enumerate() {
            let part_number = (idx + 1) as u32;
            let url = url.clone();
            let chunk = read_chunk(&mut file, init.part_size as usize).await?;
            let chunk_len = chunk.len() as u64;

            let permit = Arc::clone(&sem).acquire_owned().await?;
            let client = self.client.clone();
            let bar = bar.clone();

            tasks.spawn(async move {
                let _permit = permit;
                let body = reqwest::Body::wrap(progress::wrap_vec_body(chunk, bar));

                let response = client
                    .put(&url)
                    .header("Content-Length", chunk_len)
                    .body(body)
                    .send()
                    .await
                    .with_context(|| format!("Part {} upload failed", part_number))?;

                if !response.status().is_success() {
                    anyhow::bail!("Part {} failed (HTTP {})", part_number, response.status());
                }

                let etag = response
                    .headers()
                    .get("ETag")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
                    .trim_matches('"')
                    .to_string();

                Ok(PartInfo { part_number, etag })
            });
        }

        let mut parts: Vec<PartInfo> = Vec::with_capacity(init.number_parts as usize);
        while let Some(res) = tasks.join_next().await {
            parts.push(res.context("Upload task panicked")??);
        }
        parts.sort_unstable_by_key(|p| p.part_number);

        bar.finish_and_clear();

        // Step 3 — complete.
        // The API uses PHP-style indexed form encoding: parts[0][PartNumber]=1&parts[0][ETag]=...
        let user = self.token.clone().unwrap_or_default();
        let mut form = reqwest::multipart::Form::new()
            .text("key", init.key)
            .text("uploadId", init.upload_id)
            .text("name", file_name)
            .text("user", user);

        if let Some(path) = folder {
            form = form.text("path", path.to_string());
        }
        for (i, part) in parts.iter().enumerate() {
            form = form
                .text(
                    format!("parts[{}][PartNumber]", i),
                    part.part_number.to_string(),
                )
                .text(format!("parts[{}][ETag]", i), part.etag.clone());
        }

        let response = self
            .client
            .post(VIKING_COMPLETE_UPLOAD_API)
            .multipart(form)
            .send()
            .await
            .context("complete-upload request failed")?;

        let status = response.status();
        if !status.is_success() {
            let body = http::read_error_body(response).await;
            anyhow::bail!("Complete upload failed (HTTP {}): {}", status, body.trim());
        }

        let complete: CompleteResponse = response
            .json()
            .await
            .context("Could not parse complete-upload response")?;

        Ok(complete.url)
    }
}

async fn read_chunk(file: &mut tokio::fs::File, capacity: usize) -> Result<Vec<u8>> {
    let mut buf = vec![0u8; capacity];
    let mut total = 0;
    while total < capacity {
        match file.read(&mut buf[total..]).await? {
            0 => break,
            n => total += n,
        }
    }
    buf.truncate(total);
    Ok(buf)
}
