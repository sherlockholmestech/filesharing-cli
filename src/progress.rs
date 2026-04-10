use bytes::Bytes;
use futures_util::StreamExt;
use http_body::Frame;
use http_body_util::StreamBody;
use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
use std::io;
use std::time::Duration;
use tokio::fs::File;
use tokio_util::io::ReaderStream;

const DRAW_HZ: u8 = 20;
/// Read buffer size — 64 KB gives a good balance between syscall overhead and
/// progress-bar granularity even on fast connections.
const READ_BUF: usize = 64 * 1024;

fn bar_style() -> ProgressStyle {
    ProgressStyle::default_bar()
        .template(
            "{spinner:.cyan} {msg}\n\
             [{bar:40.cyan/blue}] {bytes}/{total_bytes}  {bytes_per_sec}  eta {eta}",
        )
        .expect("valid template")
        .progress_chars("=>-")
}

fn configure(bar: &ProgressBar, file_name: &str) {
    bar.set_style(bar_style());
    bar.set_message(file_name.to_string());
    // Keep ETA / speed updated even during brief pauses between chunks.
    bar.enable_steady_tick(Duration::from_millis(100));
}

pub fn new_bar(total_bytes: u64, file_name: &str) -> ProgressBar {
    let bar = ProgressBar::with_draw_target(
        Some(total_bytes),
        ProgressDrawTarget::stderr_with_hz(DRAW_HZ),
    );
    configure(&bar, file_name);
    bar
}

/// Used when Content-Length is unknown (e.g. download without the header).
pub fn new_bar_unknown(file_name: &str) -> ProgressBar {
    let bar = ProgressBar::with_draw_target(None, ProgressDrawTarget::stderr_with_hz(DRAW_HZ));
    bar.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.cyan} {msg}\n{bytes}  {bytes_per_sec}")
            .expect("valid template"),
    );
    bar.set_message(file_name.to_string());
    bar.enable_steady_tick(Duration::from_millis(100));
    bar
}

/// Stream a whole file through a progress bar — used for single-PUT / multipart-form uploads.
pub fn wrap_body(
    file: File,
    bar: ProgressBar,
) -> StreamBody<impl futures_util::Stream<Item = Result<Frame<Bytes>, io::Error>> + Send + 'static>
{
    let stream = ReaderStream::with_capacity(file, READ_BUF).map(move |result| {
        result.map(|chunk| {
            bar.inc(chunk.len() as u64);
            Frame::data(chunk)
        })
    });
    StreamBody::new(stream)
}

/// Stream an already-buffered `Vec<u8>` in 64 KB sub-chunks through a progress bar.
/// Used for presigned-URL multipart parts where we must know the byte range upfront.
pub fn wrap_vec_body(
    data: Vec<u8>,
    bar: ProgressBar,
) -> StreamBody<impl futures_util::Stream<Item = Result<Frame<Bytes>, io::Error>> + Send + 'static>
{
    let chunks: Vec<Bytes> = data
        .chunks(READ_BUF)
        .map(Bytes::copy_from_slice)
        .collect();

    let stream = futures_util::stream::iter(chunks).map(move |bytes| {
        bar.inc(bytes.len() as u64);
        Ok(Frame::data(bytes))
    });
    StreamBody::new(stream)
}
