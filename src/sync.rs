//! Persistent one-way JMAP snapshots and delta reconciliation.
use crate::{
    config::{create_private_dir, write_private_file},
    error::Error,
    jmap::{DataType, JmapClient},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

#[derive(Serialize, Deserialize)]
pub struct SyncCache {
    pub version: u32,
    pub username: String,
    pub account_id: String,
    pub kind: DataType,
    pub state: String,
    pub records: BTreeMap<String, Value>,
}

pub struct CacheLock(std::fs::File);
impl CacheLock {
    pub fn acquire(path: &Path) -> Result<Self> {
        let lock = path.with_extension("lock");
        let mut options = std::fs::OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(&lock)?;
        file.try_lock().with_context(|| {
            format!(
                "Cannot lock {} (another operation may be running)",
                lock.display()
            )
        })?;
        Ok(Self(file))
    }
}
impl Drop for CacheLock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

pub fn save_private_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let tmp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> Result<()> {
        write_private_file(&tmp, &serde_json::to_vec_pretty(value)?)?;
        std::fs::File::open(&tmp)?.sync_all()?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(tmp);
    }
    result
}

fn merge(cache: &mut SyncCache, result: &Value) -> Result<()> {
    let records = result["list"].as_array().context("Missing JMAP records")?;
    for record in records {
        let id = record["id"].as_str().context("Record has no ID")?;
        cache.records.insert(id.into(), record.clone());
    }
    for id in result["notFound"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        cache.records.remove(id);
    }
    Ok(())
}

async fn fetch(client: &JmapClient, cache: &mut SyncCache, ids: &[String]) -> Result<()> {
    let limit = client.session()?.capabilities["urn:ietf:params:jmap:core"]["maxObjectsInGet"]
        .as_u64()
        .unwrap_or(100)
        .clamp(1, 100) as usize;
    for batch in ids.chunks(limit) {
        merge(cache, &client.records(cache.kind, Some(batch)).await?)?;
    }
    Ok(())
}

async fn all_query_ids(client: &JmapClient, kind: DataType) -> Result<Vec<String>> {
    Ok(client.all_query_ids(kind, json!({})).await?)
}

async fn snapshot(client: &JmapClient, kind: DataType) -> Result<SyncCache> {
    // Capture a cursor before enumerating: changes during the snapshot are then
    // replayed, including updates to records already fetched and deletions.
    let initial = client.records(kind, Some(&[])).await?;
    let mut cache = SyncCache {
        version: 1,
        username: client.session()?.username.clone(),
        account_id: client.account_for(kind.capability())?.into(),
        kind,
        state: initial["state"].as_str().context("Missing state")?.into(),
        records: BTreeMap::new(),
    };
    match kind {
        DataType::Email | DataType::Submission | DataType::Contact => {
            let ids = all_query_ids(client, kind).await?;
            fetch(client, &mut cache, &ids).await?;
        }
        DataType::Thread => {
            let ids = all_query_ids(client, DataType::Email).await?;
            let mut threads = BTreeSet::new();
            for ids in ids.chunks(100) {
                for email in client.get_email_summaries(ids).await? {
                    if let Some(id) = email.thread_id {
                        threads.insert(id);
                    }
                }
            }
            fetch(client, &mut cache, &threads.into_iter().collect::<Vec<_>>()).await?;
        }
        _ => merge(&mut cache, &client.records(kind, None).await?)?,
    }
    Ok(cache)
}

async fn reconcile(client: &JmapClient, cache: &mut SyncCache) -> Result<()> {
    loop {
        let result = client.changes(cache.kind, &cache.state, Some(100)).await?;
        let ids: Vec<String> = ["created", "updated"]
            .into_iter()
            .flat_map(|key| result[key].as_array().into_iter().flatten())
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        fetch(client, cache, &ids).await?;
        for id in result["destroyed"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            cache.records.remove(id);
        }
        cache.state = result["newState"]
            .as_str()
            .context("Missing newState")?
            .into();
        if result["hasMoreChanges"] != true {
            break;
        }
    }
    Ok(())
}

pub async fn synchronize(client: &JmapClient, kind: DataType, path: &Path) -> Result<SyncCache> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    // Only create/tighten the dedicated new directory, not a user's existing cwd.
    if !parent.exists() {
        create_private_dir(parent)?;
    }
    let _lock = CacheLock::acquire(path)?;
    let mut cache = if path.exists() {
        let cache: SyncCache = serde_json::from_slice(&std::fs::read(path)?)?;
        ensure!(
            cache.version == 1
                && cache.kind == kind
                && cache.username == client.session()?.username
                && cache.account_id == client.account_for(kind.capability())?,
            "Cache belongs to a different account/type/version; use another cache file"
        );
        cache
    } else {
        snapshot(client, kind).await?
    };
    match reconcile(client, &mut cache).await {
        Err(error) if error.downcast_ref::<Error>().is_some_and(|e| matches!(e, Error::Jmap { error_type, .. } if error_type == "cannotCalculateChanges")) => {
            cache = snapshot(client, kind).await?;
            reconcile(client, &mut cache).await?;
        }
        other => other?,
    }
    save_private_json(path, &cache)?;
    Ok(cache)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};

    #[tokio::test]
    async fn snapshot_replays_concurrent_changes_and_resumes_from_disk() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).respond_with(|request: &wiremock::Request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            let call = &body["methodCalls"][0]; let args = &call[1];
            let result = match call[0].as_str().unwrap() {
                "Email/get" if args["ids"].as_array().unwrap().is_empty() => json!({"state":"s0", "list":[]}),
                "Email/get" => json!({"list": args["ids"].as_array().unwrap().iter().map(|id| json!({"id":id, "subject":format!("record-{id}")})).collect::<Vec<_>>()}),
                "Email/query" if args["position"] == 0 => json!({"ids":["e1","e2"]}),
                "Email/query" => json!({"ids":[]}),
                "Email/changes" if args["sinceState"] == "s0" => json!({"newState":"s1", "hasMoreChanges":true, "created":["e3"], "updated":["e1"], "destroyed":["e2"]}),
                "Email/changes" if args["sinceState"] == "s1" => json!({"newState":"s2", "hasMoreChanges":false, "created":[], "updated":[], "destroyed":[]}),
                "Email/changes" if args["sinceState"] == "s2" => json!({"newState":"s3", "hasMoreChanges":false, "created":[], "updated":["e3"], "destroyed":["e1"]}),
                _ => panic!("Unexpected call: {call}"),
            };
            ResponseTemplate::new(200).set_body_json(json!({"methodResponses":[[call[0],result,"0"]]}))
        }).mount(&server).await;
        let client = JmapClient::with_test_session(&server.uri());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.json");
        let first = synchronize(&client, DataType::Email, &path).await.unwrap();
        assert_eq!(first.state, "s2");
        assert!(first.records.contains_key("e1"));
        assert!(first.records.contains_key("e3"));
        assert!(!first.records.contains_key("e2"));
        let second = synchronize(&client, DataType::Email, &path).await.unwrap();
        assert_eq!(second.state, "s3");
        assert_eq!(second.records.len(), 1);
        assert!(second.records.contains_key("e3"));
        assert!(
            synchronize(&client, DataType::Mailbox, &path)
                .await
                .is_err()
        );
        let persisted: SyncCache = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(persisted.state, "s3");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[tokio::test]
    async fn incomplete_reconciliation_never_advances_the_saved_cursor() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).respond_with(|request: &wiremock::Request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap(); let call = &body["methodCalls"][0];
            if call[0] == "Email/changes" {
                ResponseTemplate::new(200).set_body_json(json!({"methodResponses":[["Email/changes",{"newState":"new","created":[],"updated":["e1"],"destroyed":[],"hasMoreChanges":false},"0"]]}))
            } else { ResponseTemplate::new(500) }
        }).mount(&server).await;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.json");
        let cache = SyncCache {
            version: 1,
            username: "test@example.com".into(),
            account_id: "acct1".into(),
            kind: DataType::Email,
            state: "old".into(),
            records: BTreeMap::new(),
        };
        save_private_json(&path, &cache).unwrap();
        let before = std::fs::read(&path).unwrap();
        assert!(
            synchronize(
                &JmapClient::with_test_session(&server.uri()),
                DataType::Email,
                &path
            )
            .await
            .is_err()
        );
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert!(CacheLock::acquire(&path).is_ok());
    }

    #[test]
    fn cache_locks_exclude_concurrent_writers_and_release_on_drop() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.json");
        let lock = CacheLock::acquire(&path).unwrap();
        assert!(CacheLock::acquire(&path).is_err());
        drop(lock);
        assert!(CacheLock::acquire(&path).is_ok());
    }

    #[tokio::test]
    async fn discarded_history_rebuilds_a_snapshot_before_saving_a_new_cursor() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).respond_with(|request: &wiremock::Request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap(); let call = &body["methodCalls"][0];
            if call[0] == "Email/changes" && call[1]["sinceState"] == "discarded" { return ResponseTemplate::new(200).set_body_json(json!({"methodResponses":[["error",{"type":"cannotCalculateChanges"},"0"]]})); }
            let result = match call[0].as_str().unwrap() { "Email/get" => json!({"state":"fresh", "list":[]}), "Email/query" => json!({"ids":[]}), "Email/changes" => json!({"newState":"latest", "hasMoreChanges":false,"created":[],"updated":[],"destroyed":[]}), _ => panic!("Unexpected call") };
            ResponseTemplate::new(200).set_body_json(json!({"methodResponses":[[call[0],result,"0"]]}))
        }).mount(&server).await;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.json");
        save_private_json(
            &path,
            &SyncCache {
                version: 1,
                username: "test@example.com".into(),
                account_id: "acct1".into(),
                kind: DataType::Email,
                state: "discarded".into(),
                records: BTreeMap::from([("stale".into(), json!({"id":"stale"}))]),
            },
        )
        .unwrap();
        let cache = synchronize(
            &JmapClient::with_test_session(&server.uri()),
            DataType::Email,
            &path,
        )
        .await
        .unwrap();
        assert_eq!(cache.state, "latest");
        assert!(cache.records.is_empty());
    }
}
