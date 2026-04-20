use reqwest::{Client, redirect::Policy};
use std::sync::OnceLock;
use std::time::Duration;

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

fn build_client(redirect_policy: Policy) -> reqwest::Result<Client> {
    Client::builder()
        .connect_timeout(Duration::from_secs(
            crate::settings::http_connect_timeout_secs(),
        ))
        .timeout(Duration::from_secs(
            crate::settings::http_request_timeout_secs(),
        ))
        .pool_idle_timeout(Duration::from_secs(
            crate::settings::http_pool_idle_timeout_secs(),
        ))
        .redirect(redirect_policy)
        .user_agent(concat!("fsc/", env!("CARGO_PKG_VERSION")))
        .build()
}
