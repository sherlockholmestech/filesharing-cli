use anyhow::{Context, Result};
use indicatif::ProgressBar;
use reqwest::Client;
use tokio::io::AsyncReadExt;

use crate::http;

/// Reads up to `capacity` bytes from `file`, returning however many were available.
pub async fn read_chunk(file: &mut tokio::fs::File, capacity: usize) -> Result<Vec<u8>> {
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

pub async fn upload_presigned_part(
    client: &Client,
    url: &str,
    part_number: u32,
    chunk: Vec<u8>,
    bar: &ProgressBar,
) -> Result<String> {
    let chunk_len = chunk.len() as u64;
    let operation = format!("Part {} upload", part_number);

    let response = http::send_retrying(
        || {
            client
                .put(url)
                .header("Content-Length", chunk_len)
                .body(chunk.clone())
        },
        &operation,
    )
    .await
    .with_context(|| format!("Part {} upload failed", part_number))?;

    if !response.status().is_success() {
        anyhow::bail!("Part {} failed (HTTP {})", part_number, response.status());
    }

    bar.inc(chunk_len);

    Ok(response
        .headers()
        .get("ETag")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .trim_matches('"')
        .to_string())
}
