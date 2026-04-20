use bytes::Bytes;
use futures_util::StreamExt;
use http_body::{Frame, SizeHint};
use http_body_util::StreamBody;
use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
use pin_project_lite::pin_project;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::fs::File;
use tokio_util::io::ReaderStream;

fn draw_hz() -> u8 {
    crate::settings::progress_draw_hz()
}

fn tick_duration() -> Duration {
    Duration::from_millis(crate::settings::progress_tick_ms())
}

fn read_buffer_bytes() -> usize {
    crate::settings::read_buffer_bytes()
}

// ── progress bars ─────────────────────────────────────────────────────────────

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
    bar.enable_steady_tick(tick_duration());
}

pub fn new_bar(total_bytes: u64, file_name: &str) -> ProgressBar {
    let bar = ProgressBar::with_draw_target(
        Some(total_bytes),
        ProgressDrawTarget::stderr_with_hz(draw_hz()),
    );
    configure(&bar, file_name);
    bar
}

pub fn new_bar_unknown(file_name: &str) -> ProgressBar {
    let bar = ProgressBar::with_draw_target(None, ProgressDrawTarget::stderr_with_hz(draw_hz()));
    bar.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.cyan} {msg}\n{bytes}  {bytes_per_sec}")
            .expect("valid template"),
    );
    bar.set_message(file_name.to_string());
    bar.enable_steady_tick(tick_duration());
    bar
}

// ── SizedBody ─────────────────────────────────────────────────────────────────
//
// Wraps any Body and overrides size_hint() with an exact value so that hyper
// sends Content-Length instead of Transfer-Encoding: chunked.  Without this,
// StreamBody reports an unknown size and hyper falls back to chunked encoding
// even when we manually set the Content-Length header.

pin_project! {
    pub struct SizedBody<B> {
        #[pin]
        inner: B,
        size: u64,
    }
}

impl<B> SizedBody<B> {
    fn new(inner: B, size: u64) -> Self {
        Self { inner, size }
    }
}

impl<B> http_body::Body for SizedBody<B>
where
    B: http_body::Body,
{
    type Data = B::Data;
    type Error = B::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        self.project().inner.poll_frame(cx)
    }

    fn size_hint(&self) -> SizeHint {
        SizeHint::with_exact(self.size)
    }
}

// ── body builders ─────────────────────────────────────────────────────────────

/// Stream a whole file through a progress bar.
/// Returns a body with an exact size hint so hyper uses Content-Length.
pub fn wrap_body(
    file: File,
    file_size: u64,
    bar: ProgressBar,
) -> SizedBody<
    StreamBody<impl futures_util::Stream<Item = Result<Frame<Bytes>, io::Error>> + Send + 'static>,
> {
    let stream = ReaderStream::with_capacity(file, read_buffer_bytes()).map(move |result| {
        result.map(|chunk| {
            bar.inc(chunk.len() as u64);
            Frame::data(chunk)
        })
    });
    SizedBody::new(StreamBody::new(stream), file_size)
}

/// Stream a pre-buffered Vec<u8> in 64 KB sub-chunks through a progress bar.
/// Used for presigned-URL multipart parts.
pub fn wrap_vec_body(
    data: Vec<u8>,
    bar: ProgressBar,
) -> SizedBody<
    StreamBody<impl futures_util::Stream<Item = Result<Frame<Bytes>, io::Error>> + Send + 'static>,
> {
    let size = data.len() as u64;
    let read_buf = read_buffer_bytes();
    let chunks: Vec<Bytes> = data.chunks(read_buf).map(Bytes::copy_from_slice).collect();

    let stream = futures_util::stream::iter(chunks).map(move |bytes| {
        bar.inc(bytes.len() as u64);
        Ok(Frame::data(bytes))
    });
    SizedBody::new(StreamBody::new(stream), size)
}
