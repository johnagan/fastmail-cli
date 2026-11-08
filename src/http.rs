use anyhow::{Context, Result};
use reqwest::{Client, RequestBuilder, Response};
use std::time::Duration;

pub fn client() -> Result<Client> {
    Client::builder()
        .timeout(Duration::from_secs(60))
        .user_agent(concat!("fastmail/", env!("CARGO_PKG_VERSION")))
        .build()
        .context("building HTTP client")
}

/// A client for long-lived streams. No overall timeout, so the connection can stay open for hours.
pub fn stream_client() -> Result<Client> {
    Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .user_agent(concat!("fastmail/", env!("CARGO_PKG_VERSION")))
        .build()
        .context("building streaming HTTP client")
}

/// Sends a request, retrying rate limits (429) and server errors with backoff.
pub async fn send(req: RequestBuilder) -> Result<Response> {
    let mut attempt = 1;
    loop {
        let copy = req.try_clone().context("request cannot be retried")?;
        let resp = copy.send().await.context("request failed (network or timeout)")?;
        let status = resp.status();
        if (status.as_u16() == 429 || status.is_server_error()) && attempt < 4 {
            tracing::debug!(%status, attempt, "retrying request");
            tokio::time::sleep(Duration::from_secs(1 << (attempt - 1))).await;
            attempt += 1;
            continue;
        }
        return Ok(resp);
    }
}

/// Percent-encodes everything except unreserved characters and `/`.
pub fn encode_path(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for b in input.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Encodes a single query or path segment, including `/`.
pub fn encode_segment(input: &str) -> String {
    encode_path(input).replace('/', "%2F")
}
