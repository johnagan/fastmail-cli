use crate::config::Credentials;
use crate::http::stream_client;
use crate::jmap::{EmailChanges, Jmap};
use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

const MAX_BACKOFF: u64 = 60;
/// Fastmail sends a keep-alive every 30 seconds. Silence for this long means the connection is dead.
const SILENCE_LIMIT: Duration = Duration::from_secs(90);

/// Where the last-seen Email state is kept. It holds no secrets.
fn state_path() -> Result<PathBuf> {
    if let Some(p) = std::env::var_os("FASTMAIL_STATE") {
        return Ok(PathBuf::from(p));
    }
    let home = std::env::var_os("HOME").context("HOME is not set; set FASTMAIL_STATE")?;
    Ok(PathBuf::from(home).join(".config/fastmail/state"))
}

fn read_state(path: &PathBuf) -> Option<String> {
    std::fs::read_to_string(path).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

fn write_state(path: &PathBuf, state: &str) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, state).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))?;
    Ok(())
}

/// Server-sent events parser. Feed it text as it arrives; it returns complete (event, data) pairs.
#[derive(Default)]
struct Sse {
    buf: String,
}

impl Sse {
    fn feed(&mut self, chunk: &str) -> Vec<(String, String)> {
        self.buf.push_str(&chunk.replace("\r\n", "\n"));
        let mut events = Vec::new();
        while let Some(end) = self.buf.find("\n\n") {
            let block: String = self.buf.drain(..end + 2).collect();
            let mut name = String::from("message");
            let mut data: Vec<&str> = Vec::new();
            for line in block.lines() {
                if let Some(v) = line.strip_prefix("event:") {
                    name = v.trim().to_string();
                } else if let Some(v) = line.strip_prefix("data:") {
                    data.push(v.strip_prefix(' ').unwrap_or(v));
                }
            }
            if !data.is_empty() {
                events.push((name, data.join("\n")));
            }
        }
        events
    }
}

/// Prints each new email as one JSON line, then records the new state.
async fn catch_up(jmap: &Jmap, path: &PathBuf) -> Result<()> {
    let Some(mut since) = read_state(path) else {
        let state = jmap.email_state().await?;
        write_state(path, &state)?;
        return Ok(());
    };
    loop {
        let changes: EmailChanges = match jmap.email_changes(&since).await {
            Ok(c) => c,
            Err(err) if format!("{err:#}").contains("cannotCalculateChanges") => {
                eprintln!("watch: server can no longer list changes since the saved state; resetting");
                let state = jmap.email_state().await?;
                write_state(path, &state)?;
                return Ok(());
            }
            Err(err) => return Err(err),
        };
        if !changes.created.is_empty() {
            let boxes = jmap.mailboxes().await?;
            for email in jmap.email_summaries(&changes.created).await? {
                let line = arrival(&email, &boxes);
                println!("{line}");
            }
            std::io::stdout().flush().ok();
        }
        write_state(path, &changes.new_state)?;
        since = changes.new_state;
        if !changes.has_more {
            return Ok(());
        }
    }
}

fn arrival(email: &Value, boxes: &[Value]) -> Value {
    let names: Vec<Value> = email["mailboxIds"]
        .as_object()
        .map(|ids| {
            ids.keys()
                .filter_map(|id| boxes.iter().find(|b| b["id"] == *id))
                .map(|b| b["name"].clone())
                .collect()
        })
        .unwrap_or_default();
    json!({
        "id": email["id"],
        "subject": email["subject"],
        "from": email["from"].as_array().and_then(|f| f.first()).map(|f| f["email"].clone()),
        "received": email["receivedAt"],
        "mailboxes": names,
    })
}

/// One connection: catch up, then stream state changes until the server closes it.
async fn stream_once(http: &reqwest::Client, token: &str, username: &str, path: &PathBuf, backoff: &mut u64) -> Result<()> {
    let jmap = Jmap::connect(http, token).await?;
    catch_up(&jmap, path).await?;
    *backoff = 1;

    let url = jmap
        .event_url()
        .replace("{types}", "Email")
        .replace("{closeafter}", "no")
        .replace("{ping}", "30");
    let mut resp = http
        .get(&url)
        .bearer_auth(token)
        .header("Accept", "text/event-stream")
        .send()
        .await
        .context("opening the push stream")?;
    if !resp.status().is_success() {
        return Err(anyhow!("push stream returned HTTP {}", resp.status()));
    }
    tracing::info!("watching {username}");

    let mut sse = Sse::default();
    loop {
        let next = tokio::time::timeout(SILENCE_LIMIT, resp.chunk())
            .await
            .context("no data from the push stream for 90 seconds")?;
        let Some(chunk) = next.context("reading the push stream")? else { break };
        for (name, data) in sse.feed(&String::from_utf8_lossy(&chunk)) {
            if name != "state" {
                continue;
            }
            let v: Value = serde_json::from_str(&data).context("parsing a state event")?;
            if v["changed"].as_object().is_some_and(|accounts| {
                accounts.values().any(|types| types.get("Email").is_some())
            }) {
                catch_up(&jmap, path).await?;
            }
        }
    }
    Ok(())
}

/// Runs until interrupted. Reconnects with backoff, and catches up on every reconnect.
pub async fn run(creds: &Credentials) -> Result<()> {
    let token = creds
        .jmap_token
        .as_deref()
        .context("no JMAP token: set FASTMAIL_JMAP_TOKEN or jmap.token in the config file")?;
    let http = stream_client()?;
    let path = state_path()?;
    let mut backoff = 1;
    loop {
        if let Err(err) = stream_once(&http, token, &creds.username, &path, &mut backoff).await {
            tracing::warn!("watch: {err:#}; reconnecting in {backoff}s");
        } else {
            tracing::warn!("watch: connection closed; reconnecting in {backoff}s");
        }
        tokio::time::sleep(Duration::from_secs(backoff)).await;
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_state_events_split_across_chunks() {
        let mut sse = Sse::default();
        assert!(sse.feed("event: state\ndata: {\"changed\"").is_empty());
        let events = sse.feed(": {}}\n\nevent: ping\ndata: {}\n\n");
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].0, "state");
        assert_eq!(events[1].0, "ping");
    }
}
