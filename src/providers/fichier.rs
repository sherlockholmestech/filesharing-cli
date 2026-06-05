use anyhow::{Context, Result};
use reqwest::Client;
use serde::Deserialize;
use std::path::Path;

use crate::{http, progress, token};

const FICHIER_UPLOAD_SERVER_API: &str = "https://api.1fichier.com/v1/upload/get_upload_server.cgi";

pub struct OneFichier {
    client: Client,
    token: Option<String>,
}

// ── upload server ─────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct UploadServerResponse {
    url: String,
    id: String,
}

// ── end.pl upload report ──────────────────────────────────────────────────────

#[derive(Deserialize)]
struct EndPlResponse {
    /// 1 = files landed in the account "incoming" folder (only accessible to
    /// the authenticated owner).  Still contains valid download links.
    incoming: Option<u8>,
    links: Option<Vec<FileLink>>,
}

#[derive(Deserialize)]
struct FileLink {
    download: Option<String>,
}

// ─────────────────────────────────────────────────────────────────────────────

impl OneFichier {
    pub fn new(token: Option<String>) -> Self {
        Self {
            client: http::client().clone(),
            token: token::normalize(token),
        }
    }

    pub async fn upload(&self, file_path: &Path, folder_id: Option<&str>) -> Result<String> {
        // ── Step 1: pick an upload node ───────────────────────────────────────
        //
        // POST (per the official cURL example) to get_upload_server.cgi returns
        // the hostname of the node that will accept the file and a unique upload
        // ID used to track progress and retrieve the report.
        let server_resp = http::send_retrying(
            || {
                self.client
                    .post(FICHIER_UPLOAD_SERVER_API)
                    .json(&serde_json::json!({}))
            },
            "1fichier server selector",
        )
        .await
        .context("Could not reach 1fichier server selector")?;

        let server_status = server_resp.status();
        if !server_status.is_success() {
            let body = http::read_error_body(server_resp).await;
            anyhow::bail!(
                "1fichier server selector failed (HTTP {}): {}",
                server_status,
                body.trim()
            );
        }

        let server: UploadServerResponse = server_resp
            .json()
            .await
            .context("Could not parse upload-server response")?;

        // server.url is a bare hostname, e.g. "node.1fichier.com"
        let upload_host = server.url;
        let upload_id = server.id;

        // ── Step 2: stream the file ───────────────────────────────────────────
        let file_name = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .context("File has no valid name")?
            .to_string();

        let file = tokio::fs::File::open(file_path)
            .await
            .with_context(|| format!("Could not open: {}", file_path.display()))?;
        let file_size = file.metadata().await?.len();

        let bar = progress::new_bar(file_size, &file_name);
        let body = reqwest::Body::wrap(progress::wrap_body(file, file_size, bar.clone()));

        // The API parameter is "file[]" (supports up to 500 files per upload).
        let file_part = reqwest::multipart::Part::stream_with_length(body, file_size)
            .file_name(file_name.clone());

        let mut form = reqwest::multipart::Form::new().part("file[]", file_part);
        if let Some(fid) = folder_id {
            // "did" — numeric destination folder ID; ignored when unauthenticated.
            form = form.text("did", fid.to_string());
        }

        // Use a no-redirect client so we can read the Location header from the
        // 302 response before requesting end.pl.
        let no_redir = http::no_redirect_client()?;

        let upload_url = format!("https://{}/upload.cgi?id={}", upload_host, upload_id);
        let mut upload_req = no_redir.post(&upload_url).multipart(form);
        if let Some(token) = &self.token {
            upload_req = upload_req.header("Authorization", format!("Bearer {}", token));
        }

        let upload_resp = upload_req.send().await.context("Upload request failed");
        bar.finish_and_clear();
        let upload_resp = upload_resp?;

        let upload_status = upload_resp.status();

        let xid: String = if upload_status.as_u16() == 302 {
            // Location: /end.pl?xid=UPLOAD_ID — extract xid, fall back to the
            // id we already have in the rare case the header is absent.
            upload_resp
                .headers()
                .get("Location")
                .and_then(|v| v.to_str().ok())
                .and_then(|loc| query_param(loc, "xid"))
                .unwrap_or_else(|| upload_id.clone())
        } else if upload_status.is_success() {
            // Unusual: some nodes return 200 directly.
            upload_id
        } else {
            let body = http::read_error_body(upload_resp).await;
            anyhow::bail!("Upload failed (HTTP {}): {}", upload_status, body.trim());
        };

        // ── Step 3: fetch the upload report as JSON from end.pl ───────────────
        //
        // The request must go to the same upload node.  Adding the "JSON: 1"
        // request header switches the response from HTML to JSON.
        let end_url = format!("https://{}/end.pl?xid={}", upload_host, xid);
        let end_resp = http::send_retrying(
            || self.client.get(&end_url).header("JSON", "1"),
            "1fichier upload report request",
        )
        .await
        .context("Failed to fetch upload report (end.pl)")?;

        let end_status = end_resp.status();
        if !end_status.is_success() {
            let body = http::read_error_body(end_resp).await;
            anyhow::bail!("end.pl returned HTTP {}: {}", end_status, body.trim());
        }

        let report: EndPlResponse = end_resp
            .json()
            .await
            .context("Could not parse upload report JSON from end.pl")?;

        if let Some(link) = report
            .links
            .and_then(|ls| ls.into_iter().next())
            .and_then(|l| l.download)
        {
            return Ok(link);
        }

        if report.incoming == Some(1) {
            anyhow::bail!(
                "Upload succeeded but no public link was returned; file was placed in incoming"
            );
        }

        anyhow::bail!("Upload succeeded but the response contained no download link")
    }
}

// ── helpers ───────────────────────────────────────────────────────────────────

/// Extract the value of a query parameter from a URL or path+query string.
fn query_param(url: &str, key: &str) -> Option<String> {
    let query = url.split('?').nth(1)?;
    for pair in query.split('&') {
        let mut kv = pair.splitn(2, '=');
        if kv.next()? == key {
            return Some(kv.next().unwrap_or("").to_string());
        }
    }
    None
}
