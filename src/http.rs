use anyhow::{Context, Result};
use reqwest::{Client, RequestBuilder, Response, StatusCode, header, redirect::Policy};
use std::fmt;
use std::sync::OnceLock;
use std::time::Duration;
use tokio::time::sleep;

static CLIENT: OnceLock<Client> = OnceLock::new();

pub fn client() -> &'static Client {
    CLIENT.get_or_init(|| {
        build_client(Policy::limited(crate::settings::http_redirect_limit()))
            .expect("failed to initialize shared HTTP client")
    })
}

pub fn no_redirect_client() -> reqwest::Result<Client> {
    build_client(Policy::none())
}

pub async fn read_error_body(response: reqwest::Response) -> String {
    match response.text().await {
        Ok(body) if !body.trim().is_empty() => body,
        Ok(_) => "<empty response body>".to_string(),
        Err(err) => format!("<failed to read response body: {}>", err),
    }
}

pub async fn send_retrying<F>(mut build_request: F, operation: &str) -> Result<Response>
where
    F: FnMut() -> RequestBuilder,
{
    let max_retries = crate::settings::http_retry_attempts();
    let mut attempt = 0;

    loop {
        match build_request().send().await {
            Ok(response) if should_retry_status(response.status()) && attempt < max_retries => {
                let delay = retry_delay(attempt, response.headers().get(header::RETRY_AFTER));
                warn_retry(
                    operation,
                    attempt,
                    max_retries,
                    delay,
                    format_args!("HTTP {}", response.status()),
                );
                attempt += 1;
                sleep(delay).await;
            }
            Ok(response) => return Ok(response),
            Err(err) if should_retry_error(&err) && attempt < max_retries => {
                let delay = retry_delay(attempt, None);
                warn_retry(
                    operation,
                    attempt,
                    max_retries,
                    delay,
                    format_args!("{}", retry_error_reason(&err)),
                );
                attempt += 1;
                sleep(delay).await;
            }
            Err(err) => return Err(err).with_context(|| format!("{operation} failed")),
        }
    }
}

fn build_client(redirect_policy: Policy) -> reqwest::Result<Client> {
    Client::builder()
        .connect_timeout(Duration::from_secs(
            crate::settings::http_connect_timeout_secs(),
        ))
        .read_timeout(Duration::from_secs(
            crate::settings::http_read_timeout_secs(),
        ))
        .pool_idle_timeout(Duration::from_secs(
            crate::settings::http_pool_idle_timeout_secs(),
        ))
        .redirect(redirect_policy)
        .user_agent(concat!("fsc/", env!("CARGO_PKG_VERSION")))
        .build()
}

fn should_retry_status(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::REQUEST_TIMEOUT
            | StatusCode::TOO_MANY_REQUESTS
            | StatusCode::INTERNAL_SERVER_ERROR
            | StatusCode::BAD_GATEWAY
            | StatusCode::SERVICE_UNAVAILABLE
            | StatusCode::GATEWAY_TIMEOUT
    )
}

fn should_retry_error(err: &reqwest::Error) -> bool {
    err.is_connect() || err.is_timeout() || err.is_body()
}

fn retry_error_reason(err: &reqwest::Error) -> &'static str {
    if err.is_timeout() {
        "timeout"
    } else if err.is_connect() {
        "connection failed"
    } else if err.is_body() {
        "body stream interrupted"
    } else {
        "transient network error"
    }
}

fn warn_retry(
    operation: &str,
    attempt: usize,
    max_retries: usize,
    delay: Duration,
    reason: fmt::Arguments<'_>,
) {
    eprintln!(
        "{}",
        crate::style::warn(format!(
            "{} failed ({}), retrying {}/{} in {:.1}s",
            operation,
            reason,
            attempt + 1,
            max_retries,
            delay.as_secs_f64()
        ))
    );
}

fn retry_delay(attempt: usize, retry_after: Option<&header::HeaderValue>) -> Duration {
    if let Some(seconds) = retry_after
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
    {
        return Duration::from_secs(seconds.clamp(1, 300));
    }

    let base_ms = crate::settings::http_retry_backoff_ms();
    let multiplier = 1u64 << attempt.min(6);
    Duration::from_millis(base_ms.saturating_mul(multiplier).min(30_000))
}
