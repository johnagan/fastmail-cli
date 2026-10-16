use super::*;

#[derive(
    Debug, Clone, Copy, Serialize, Deserialize, clap::ValueEnum, async_graphql::Enum, PartialEq, Eq,
)]
pub enum DataType {
    Email,
    Mailbox,
    Thread,
    Identity,
    Submission,
    Contact,
    Addressbook,
}

impl DataType {
    pub fn name(self) -> &'static str {
        match self {
            Self::Email => "Email",
            Self::Mailbox => "Mailbox",
            Self::Thread => "Thread",
            Self::Identity => "Identity",
            Self::Submission => "EmailSubmission",
            Self::Contact => "ContactCard",
            Self::Addressbook => "AddressBook",
        }
    }
    pub fn capability(self) -> &'static str {
        match self {
            Self::Identity | Self::Submission => "urn:ietf:params:jmap:submission",
            Self::Contact | Self::Addressbook => "urn:ietf:params:jmap:contacts",
            _ => "urn:ietf:params:jmap:mail",
        }
    }
}

fn patch_key(key: &str) -> String {
    key.replace('~', "~0").replace('/', "~1")
}

fn merge_set(result: &mut Value, response: Value) {
    if result["oldState"].is_null() {
        result["oldState"] = response["oldState"].clone();
    }
    result["newState"] = response["newState"].clone();
    for key in ["updated", "notUpdated", "notDestroyed"] {
        if let Some(map) = response[key].as_object() {
            result[key].as_object_mut().unwrap().extend(map.clone());
        }
    }
    if let Some(destroyed) = response["destroyed"].as_array() {
        result["destroyed"]
            .as_array_mut()
            .unwrap()
            .extend(destroyed.clone());
    }
}

impl JmapClient {
    /// Bulk patches preserve unmentioned keywords and mailbox memberships.
    /// Return the complete /set result so callers can handle per-object failures.
    pub async fn patch_emails(
        &self,
        ids: &[String],
        keywords: &HashMap<String, bool>,
        add_mailboxes: &[String],
        remove_mailboxes: &[String],
        move_to: Option<&str>,
    ) -> Result<Value> {
        if ids.is_empty() {
            return Err(Error::Config("Provide at least one email ID".into()));
        }
        let mut patch = json!({});
        for (keyword, enabled) in keywords {
            patch[format!("keywords/{}", patch_key(keyword))] =
                if *enabled { json!(true) } else { Value::Null };
        }
        if let Some(mailbox) = move_to {
            if !add_mailboxes.is_empty() || !remove_mailboxes.is_empty() {
                return Err(Error::Config(
                    "Use moveTo or membership changes, not both".into(),
                ));
            }
            patch["mailboxIds"] = json!({mailbox: true});
        } else {
            for (mailboxes, value) in [
                (add_mailboxes, json!(true)),
                (remove_mailboxes, Value::Null),
            ] {
                for mailbox in mailboxes {
                    patch[format!("mailboxIds/{}", patch_key(mailbox))] = value.clone();
                }
            }
        }
        if patch.as_object().unwrap().is_empty() {
            return Err(Error::Config("Provide a keyword or mailbox change".into()));
        }
        self.email_set_batches(ids, Some(patch)).await
    }

    pub async fn destroy_emails(&self, ids: &[String]) -> Result<Value> {
        if ids.is_empty() {
            return Err(Error::Config("Provide at least one email ID".into()));
        }
        self.email_set_batches(ids, None).await
    }

    pub async fn empty_trash(&mut self) -> Result<Value> {
        let trash = self.find_mailbox("trash").await?;
        let ids = self
            .all_query_ids(DataType::Email, json!({"inMailbox":trash.id}))
            .await?;
        let mut result = json!({"accountId": self.account_id()?, "updated": {}, "notUpdated": {}, "destroyed": [], "notDestroyed": {}});
        let core = &self.session()?.capabilities["urn:ietf:params:jmap:core"];
        let limit = core["maxObjectsInSet"]
            .as_u64()
            .unwrap_or(100)
            .min(core["maxObjectsInGet"].as_u64().unwrap_or(100))
            .clamp(1, 100) as usize;
        for batch in ids.chunks(limit) {
            let records = self.raw_call("Email/get", json!({"accountId":self.account_id()?,"ids":batch,"properties":["id","mailboxIds"]})).await?;
            let state = records["state"]
                .as_str()
                .ok_or_else(|| Error::Server("Missing email state".into()))?;
            let mut update = serde_json::Map::new();
            let mut destroy = Vec::new();
            for email in records["list"].as_array().into_iter().flatten() {
                let id = email["id"]
                    .as_str()
                    .ok_or_else(|| Error::Server("Missing email ID".into()))?;
                if email["mailboxIds"][&trash.id] != true {
                    continue;
                }
                if email["mailboxIds"].as_object().is_some_and(|m| m.len() > 1) {
                    update.insert(
                        id.to_owned(),
                        json!({format!("mailboxIds/{}",patch_key(&trash.id)):null}),
                    );
                } else {
                    destroy.push(id.to_owned());
                }
            }
            if !update.is_empty() || !destroy.is_empty() {
                merge_set(&mut result,self.raw_call("Email/set",json!({"accountId":self.account_id()?,"ifInState":state,"update":update,"destroy":destroy})).await?);
            }
        }
        Ok(result)
    }

    async fn email_set_batches(&self, ids: &[String], patch: Option<Value>) -> Result<Value> {
        let limit = self.session()?.capabilities["urn:ietf:params:jmap:core"]["maxObjectsInSet"]
            .as_u64()
            .unwrap_or(100)
            .clamp(1, 500) as usize;
        let mut result = json!({"accountId": self.account_id()?, "updated": {}, "notUpdated": {}, "destroyed": [], "notDestroyed": {}});
        for batch in ids.chunks(limit) {
            let mut args = json!({"accountId": self.account_id()?});
            if let Some(patch) = &patch {
                args["update"] =
                    Value::Object(batch.iter().map(|id| (id.clone(), patch.clone())).collect());
            } else {
                args["destroy"] = json!(batch);
            }
            let response = self.raw_call("Email/set", args).await?;
            merge_set(&mut result, response);
        }
        Ok(result)
    }

    /// Like call, but preserve per-record failures for bulk-operation reporting.
    async fn raw_call(&self, method: &str, args: Value) -> Result<Value> {
        let responses = self.request(vec![json!([method, args, "0"])]).await?;
        Self::parse_response(responses.first().unwrap_or(&Value::Null), method)
    }

    pub async fn records(&self, kind: DataType, ids: Option<&[String]>) -> Result<Value> {
        let mut args = json!({"accountId": self.account_for(kind.capability())?, "ids": ids});
        if kind == DataType::Email {
            args["fetchAllBodyValues"] = json!(true);
        }
        self.call(&format!("{}/get", kind.name()), args).await
    }

    pub async fn changes(&self, kind: DataType, since: &str, max: Option<u32>) -> Result<Value> {
        let mut args =
            json!({"accountId": self.account_for(kind.capability())?, "sinceState": since});
        if let Some(max) = max {
            args["maxChanges"] = json!(max);
        }
        self.call(&format!("{}/changes", kind.name()), args).await
    }

    pub async fn query_records(
        &self,
        kind: DataType,
        mut args: Value,
        changes: bool,
    ) -> Result<Value> {
        if !matches!(
            kind,
            DataType::Mailbox | DataType::Email | DataType::Submission | DataType::Contact
        ) {
            return Err(Error::Config(
                "Queries support mailbox, email, submission, and contact".into(),
            ));
        }
        if !args.is_object() {
            return Err(Error::Config("Query arguments must be an object".into()));
        }
        args["accountId"] = json!(self.account_for(kind.capability())?);
        self.call(
            &format!(
                "{}/{}",
                kind.name(),
                if changes { "queryChanges" } else { "query" }
            ),
            args,
        )
        .await
    }

    pub async fn all_query_ids(&self, kind: DataType, filter: Value) -> Result<Vec<String>> {
        for _ in 0..3 {
            let mut ids = Vec::new();
            let mut seen = std::collections::BTreeSet::new();
            let mut anchor: Option<String> = None;
            loop {
                let property = match kind {
                    DataType::Contact => "created",
                    DataType::Submission => "emailId",
                    DataType::Mailbox => "name",
                    _ => "receivedAt",
                };
                let mut args = json!({"filter":filter,"limit":100,"sort":[{"property":property,"isAscending":true}]});
                if let Some(id) = &anchor {
                    args["anchor"] = json!(id);
                    args["anchorOffset"] = json!(1);
                } else {
                    args["position"] = json!(0);
                }
                let result = match self.query_records(kind, args, false).await {
                    Err(Error::Jmap { error_type, .. }) if error_type == "anchorNotFound" => break,
                    result => result?,
                };
                let page = result["ids"]
                    .as_array()
                    .ok_or_else(|| Error::Server("Missing query IDs".into()))?;
                if page.is_empty() {
                    return Ok(ids);
                }
                let next = page
                    .last()
                    .and_then(Value::as_str)
                    .ok_or_else(|| Error::Server("Invalid anchor ID".into()))?
                    .to_owned();
                if anchor.as_ref() == Some(&next) {
                    return Err(Error::Server("Snapshot query made no progress".into()));
                }
                anchor = Some(next);
                for id in page {
                    let id = id
                        .as_str()
                        .ok_or_else(|| Error::Server("Invalid query ID".into()))?
                        .to_owned();
                    if seen.insert(id.clone()) {
                        ids.push(id);
                    }
                }
            }
        }
        Err(Error::Server(
            "Snapshot anchors were repeatedly removed; retry synchronization".into(),
        ))
    }

    pub async fn update_identity(&self, id: &str, patch: Value) -> Result<Value> {
        self.call("Identity/set", json!({"accountId": self.account_for(DataType::Identity.capability())?, "update": {id: patch}})).await
    }

    pub async fn cancel_submission(&self, ids: &[String]) -> Result<Value> {
        if ids.is_empty() {
            return Err(Error::Config("Provide at least one submission ID".into()));
        }
        let account = self.account_for(DataType::Submission.capability())?;
        let limit = self.session()?.capabilities["urn:ietf:params:jmap:core"]["maxObjectsInSet"]
            .as_u64()
            .unwrap_or(100)
            .clamp(1, 500) as usize;
        let mut result = json!({"accountId":account,"updated":{},"notUpdated":{},"destroyed":[],"notDestroyed":{}});
        for batch in ids.chunks(limit) {
            let update: serde_json::Map<_, _> = batch
                .iter()
                .map(|id| (id.clone(), json!({"undoStatus":"canceled"})))
                .collect();
            merge_set(
                &mut result,
                self.raw_call(
                    "EmailSubmission/set",
                    json!({"accountId":account,"update":update}),
                )
                .await?,
            );
        }
        Ok(result)
    }

    pub async fn copy_emails(
        &self,
        from_account: &str,
        emails: Value,
        destroy_original: bool,
    ) -> Result<Value> {
        self.call("Email/copy", json!({"accountId": self.account_id()?, "fromAccountId": from_account, "create": emails, "onSuccessDestroyOriginal": destroy_original})).await
    }

    pub async fn parse_email(&self, bytes: Vec<u8>) -> Result<Value> {
        let blob = self.upload_blob(bytes, "message/rfc822").await?;
        self.call(
            "Email/parse",
            json!({"accountId": self.account_id()?, "blobIds": [blob], "fetchAllBodyValues": true}),
        )
        .await
    }

    pub async fn export_email(&self, id: &str) -> Result<Vec<u8>> {
        let email = self.get_email(id).await?;
        let blob = email
            .blob_id
            .ok_or_else(|| Error::Server("Email has no blobId".into()))?;
        self.download_blob(&blob).await
    }

    pub async fn push_subscriptions(&self) -> Result<Value> {
        self.call("PushSubscription/get", json!({"ids": null}))
            .await
    }

    pub async fn set_push_subscriptions(&self, args: Value) -> Result<Value> {
        self.call("PushSubscription/set", args).await
    }
}

#[cfg(test)]
#[path = "operations_tests.rs"]
mod tests;
