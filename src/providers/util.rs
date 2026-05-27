use anyhow::Result;
use tokio::io::AsyncReadExt;

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
