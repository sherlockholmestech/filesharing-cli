use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tokio::io::AsyncReadExt;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::{http, progress, settings, token};

const ROOTZ_SMALL_UPLOAD_API: &str = "https://rootz.so/api/files/upload";
const ROOTZ_MULTIPART_INIT_API: &str = "https://rootz.so/api/files/multipart/init";
const ROOTZ_MULTIPART_BATCH_URLS_API: &str = "https://rootz.so/api/files/multipart/batch-urls";
const ROOTZ_MULTIPART_COMPLETE_API: &str = "https://rootz.so/api/files/multipart/complete";
const ROOTZ_SHARE_BASE: &str = "https://rootz.so/d";

const MULTIPART_THRESHOLD: u64 = 4 * 1024 * 1024; // 4 MB

pub struct Rootz {
    client: Client,
    token: Option<String>,
}

// ── small upload ──────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct SmallUploadResponse {
    success: bool,
    data: Option<SmallUploadData>,
    error: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SmallUploadData {
    short_id: String,
}

// ── multipart upload ──────────────────────────────────────────────────────────

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InitResponse {
    upload_id: String,
    key: String,
    chunk_size: u64,
    total_parts: u32,
}

#[derive(Deserialize)]
struct BatchUrlsResponse {
    success: bool,
    urls: Option<HashMap<String, String>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CompleteRequest<'a> {
    key: &'a str,
    upload_id: &'a str,
    parts: Vec<PartInfo>,
    file_name: &'a str,
    file_size: u64,
    content_type: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    folder_id: Option<&'a str>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PartInfo {
    part_number: u32,
    etag: String,
}

#[derive(Deserialize)]
struct CompleteResponse {
    success: bool,
    file: Option<CompleteFile>,
    error: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CompleteFile {
    short_id: String,
}

// ─────────────────────────────────────────────────────────────────────────────

impl Rootz {
    pub fn new(token: Option<String>) -> Self {
        Self {
            client: http::client().clone(),
            token: token::normalize(token),
        }
    }

    pub async fn upload(&self, file_path: &Path, folder_id: Option<&str>) -> Result<String> {
        let file_name = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .context("File has no valid name")?
            .to_string();

        let file_size = tokio::fs::metadata(file_path).await?.len();

        if file_size < MULTIPART_THRESHOLD {
            self.upload_small(file_path, &file_name, file_size, folder_id)
                .await
        } else {
            self.upload_multipart(file_path, &file_name, file_size, folder_id)
                .await
        }
    }

    async fn upload_small(
        &self,
        file_path: &Path,
        file_name: &str,
        file_size: u64,
        folder_id: Option<&str>,
    ) -> Result<String> {
        let file = tokio::fs::File::open(file_path).await?;
        let bar = progress::new_bar(file_size, file_name);
        let body = reqwest::Body::wrap(progress::wrap_body(file, file_size, bar.clone()));

        let file_part = reqwest::multipart::Part::stream_with_length(body, file_size)
            .file_name(file_name.to_string());

        let mut form = reqwest::multipart::Form::new().part("file", file_part);
        if let Some(fid) = folder_id {
            form = form.text("folderId", fid.to_string());
        }

        let mut req = self.client.post(ROOTZ_SMALL_UPLOAD_API).multipart(form);
        if let Some(token) = &self.token {
            req = req.header("Authorization", format!("Bearer {}", token));
        }

        let response = req.send().await.context("Rootz upload request failed")?;
        bar.finish_and_clear();

        let status = response.status();
        if !status.is_success() {
            let body = http::read_error_body(response).await;
            anyhow::bail!("Rootz upload failed (HTTP {}): {}", status, body.trim());
        }

        let resp: SmallUploadResponse =
            response.json().await.context("Could not parse response")?;

        if !resp.success {
            anyhow::bail!(
                "Upload failed: {}",
                resp.error.as_deref().unwrap_or("unknown error")
            );
        }

        let short_id = resp.data.context("No data in response")?.short_id;
        Ok(format!("{}/{}", ROOTZ_SHARE_BASE, short_id))
    }

    async fn upload_multipart(
        &self,
        file_path: &Path,
        file_name: &str,
        file_size: u64,
        folder_id: Option<&str>,
    ) -> Result<String> {
        // Step 1 — initialise.
        let mut init_body = serde_json::json!({
            "fileName": file_name,
            "fileSize": file_size,
            "fileType": "application/octet-stream",
        });
        if let Some(fid) = folder_id {
            init_body["folderId"] = serde_json::json!(fid);
        }

        let mut init_req = self.client.post(ROOTZ_MULTIPART_INIT_API).json(&init_body);
        if let Some(token) = &self.token {
            init_req = init_req.header("Authorization", format!("Bearer {}", token));
        }

        let init: InitResponse = init_req
            .send()
            .await
            .context("Multipart init request failed")?
            .json()
            .await
            .context("Could not parse init response")?;

        if init.chunk_size == 0 {
            anyhow::bail!("Rootz returned invalid chunk size 0");
        }
        if init.total_parts == 0 {
            anyhow::bail!("Rootz returned zero parts for multipart upload");
        }

        // Step 2 — fetch all presigned URLs at once.
        let urls_resp: BatchUrlsResponse = self
            .client
            .post(ROOTZ_MULTIPART_BATCH_URLS_API)
            .json(&serde_json::json!({
                "key": init.key,
                "uploadId": init.upload_id,
                "totalParts": init.total_parts,
            }))
            .send()
            .await
            .context("Batch URL request failed")?
            .json()
            .await
            .context("Could not parse batch URLs response")?;

        if !urls_resp.success {
            anyhow::bail!("Failed to obtain presigned upload URLs");
        }
        let urls = urls_resp.urls.context("No URLs in batch response")?;

        // Step 3 — upload parts in parallel.
        //
        // Chunks are read sequentially (single file handle), but the PUT
        // requests run concurrently, bounded by a semaphore.  Parallelism
        // follows the thresholds from the Rootz documentation.
        let parallelism = settings::upload_parallelism(file_size);
        let sem = Arc::new(Semaphore::new(parallelism));
        let bar = progress::new_bar(file_size, file_name);
        let mut file = tokio::fs::File::open(file_path).await?;
        let mut tasks: JoinSet<Result<PartInfo>> = JoinSet::new();

        for part_number in 1..=init.total_parts {
            let url = urls
                .get(&part_number.to_string())
                .with_context(|| format!("Missing presigned URL for part {}", part_number))?
                .clone();

            let chunk = read_chunk(&mut file, init.chunk_size as usize).await?;
            let chunk_len = chunk.len() as u64;

            // Acquire before spawning so we never have more than `parallelism`
            // concurrent uploads in flight.
            let permit = Arc::clone(&sem).acquire_owned().await?;
            let client = self.client.clone();
            let bar = bar.clone();

            tasks.spawn(async move {
                let _permit = permit; // released when this task finishes
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

        // Collect results — any task error surfaces here.
        let mut parts: Vec<PartInfo> = Vec::with_capacity(init.total_parts as usize);
        while let Some(res) = tasks.join_next().await {
            parts.push(res.context("Upload task panicked")??);
        }
        parts.sort_unstable_by_key(|p| p.part_number);

        bar.finish_and_clear();

        // Step 4 — finalise.
        let complete_body = CompleteRequest {
            key: &init.key,
            upload_id: &init.upload_id,
            parts,
            file_name,
            file_size,
            content_type: "application/octet-stream",
            folder_id,
        };

        let mut complete_req = self
            .client
            .post(ROOTZ_MULTIPART_COMPLETE_API)
            .json(&complete_body);
        if let Some(token) = &self.token {
            complete_req = complete_req.header("Authorization", format!("Bearer {}", token));
        }

        let complete: CompleteResponse = complete_req
            .send()
            .await
            .context("Multipart complete request failed")?
            .json()
            .await
            .context("Could not parse complete response")?;

        if !complete.success {
            anyhow::bail!(
                "Multipart complete failed: {}",
                complete.error.as_deref().unwrap_or("unknown error")
            );
        }

        let short_id = complete
            .file
            .context("No file in complete response")?
            .short_id;
        Ok(format!("{}/{}", ROOTZ_SHARE_BASE, short_id))
    }
}

/// Reads up to `capacity` bytes from `file`, returning however many were available.
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
