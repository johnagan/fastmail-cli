use crate::http::{encode_segment, send};
use anyhow::{anyhow, bail, Context, Result};
use reqwest::Client;
use serde_json::{json, Value};
use std::collections::HashMap;

const USING: [&str; 2] = ["urn:ietf:params:jmap:core", "urn:ietf:params:jmap:mail"];
const EMAIL_SUMMARY: [&str; 9] = [
    "id", "subject", "from", "to", "receivedAt", "hasAttachment", "preview", "mailboxIds", "keywords",
];

pub struct Jmap {
    http: Client,
    token: String,
    api_url: String,
    download_url: String,
    event_url: String,
    pub account_id: String,
    pub capabilities: Vec<String>,
}

impl Jmap {
    pub async fn connect(http: &Client, token: &str) -> Result<Self> {
        let resp = send(http.get("https://api.fastmail.com/jmap/session").bearer_auth(token)).await?;
        if !resp.status().is_success() {
            bail!("JMAP session returned HTTP {}; check the JMAP token and its scopes", resp.status());
        }
        let session: Value = resp.json().await.context("parsing JMAP session")?;
        Ok(Self {
            http: http.clone(),
            token: token.to_string(),
            api_url: session["apiUrl"].as_str().context("session has no apiUrl")?.to_string(),
            download_url: session["downloadUrl"].as_str().context("session has no downloadUrl")?.to_string(),
            event_url: session["eventSourceUrl"].as_str().context("session has no eventSourceUrl")?.to_string(),
            account_id: session["primaryAccounts"]["urn:ietf:params:jmap:mail"]
                .as_str()
                .context("session has no primary mail account")?
                .to_string(),
            capabilities: session["capabilities"]
                .as_object()
                .map(|o| o.keys().cloned().collect())
                .unwrap_or_default(),
        })
    }

    /// The push endpoint, with its `{types}`, `{closeafter}`, and `{ping}` placeholders still unfilled.
    pub fn event_url(&self) -> &str {
        &self.event_url
    }

    pub fn has(&self, capability: &str) -> bool {
        self.capabilities.iter().any(|c| c == capability)
    }

    /// Sends one request with several method calls. Returns each method's arguments in order.
    pub async fn call(&self, calls: Vec<Value>) -> Result<Vec<Value>> {
        self.call_using(&USING, calls).await
    }

    /// Like `call`, but declares the capabilities the calls need (submission for identities and sending).
    pub async fn call_using(&self, using: &[&str], calls: Vec<Value>) -> Result<Vec<Value>> {
        let body = json!({ "using": using, "methodCalls": calls });
        let resp = send(self.http.post(&self.api_url).bearer_auth(&self.token).json(&body)).await?;
        if !resp.status().is_success() {
            bail!("JMAP API returned HTTP {}", resp.status());
        }
        let parsed: Value = resp.json().await.context("parsing JMAP response")?;
        let mut out = Vec::new();
        for response in parsed["methodResponses"].as_array().cloned().unwrap_or_default() {
            if response[0] == "error" {
                bail!(
                    "JMAP error: {} {}",
                    response[1]["type"].as_str().unwrap_or("unknown"),
                    response[1]["description"].as_str().unwrap_or("")
                );
            }
            out.push(response[1].clone());
        }
        Ok(out)
    }

    pub async fn mailboxes(&self) -> Result<Vec<Value>> {
        let r = self
            .call(vec![json!(["Mailbox/get", {
                "accountId": self.account_id,
                "ids": null,
                "properties": ["name", "role", "parentId", "totalEmails", "unreadEmails"]
            }, "m"])])
            .await?;
        Ok(r[0]["list"].as_array().cloned().unwrap_or_default())
    }

    /// Resolves a mailbox by role (`inbox`, `memos`, ...) or by path (`Business/Projects`), case-insensitively.
    pub async fn mailbox_id(&self, selector: &str) -> Result<String> {
        let boxes = self.mailboxes().await?;
        let by_id: HashMap<&str, &Value> = boxes.iter().filter_map(|b| Some((b["id"].as_str()?, b))).collect();
        let path_of = |b: &Value| -> String {
            let mut parts = vec![b["name"].as_str().unwrap_or("").to_string()];
            let mut parent = b["parentId"].as_str();
            while let Some(p) = parent.and_then(|id| by_id.get(id)) {
                parts.push(p["name"].as_str().unwrap_or("").to_string());
                parent = p["parentId"].as_str();
            }
            parts.reverse();
            parts.join("/")
        };
        for b in &boxes {
            let role = b["role"].as_str().unwrap_or("");
            if role.eq_ignore_ascii_case(selector) || path_of(b).eq_ignore_ascii_case(selector) {
                return Ok(b["id"].as_str().unwrap_or_default().to_string());
            }
        }
        let names: Vec<String> = boxes.iter().map(path_of).collect();
        Err(anyhow!("no mailbox named '{selector}'. Available: {}", names.join(", ")))
    }

    pub async fn search(&self, mailbox: Option<&str>, text: Option<&str>, limit: u32) -> Result<Vec<Value>> {
        let mut conditions = Vec::new();
        if let Some(m) = mailbox {
            conditions.push(json!({ "inMailbox": self.mailbox_id(m).await? }));
        }
        if let Some(t) = text {
            conditions.push(json!({ "text": t }));
        }
        let filter = match conditions.len() {
            0 => Value::Null,
            1 => conditions.remove(0),
            _ => json!({ "operator": "AND", "conditions": conditions }),
        };
        let r = self
            .call(vec![
                json!(["Email/query", {
                    "accountId": self.account_id,
                    "filter": filter,
                    "sort": [{ "property": "receivedAt", "isAscending": false }],
                    "limit": limit
                }, "q"]),
                json!(["Email/get", {
                    "accountId": self.account_id,
                    "#ids": { "resultOf": "q", "name": "Email/query", "path": "/ids" },
                    "properties": EMAIL_SUMMARY
                }, "g"]),
            ])
            .await?;
        Ok(r[1]["list"].as_array().cloned().unwrap_or_default())
    }

    /// Full email with its plain-text body assembled into a `text` field.
    pub async fn get(&self, id: &str) -> Result<Value> {
        let r = self
            .call(vec![json!(["Email/get", {
                "accountId": self.account_id,
                "ids": [id],
                "properties": ["id", "subject", "from", "to", "cc", "receivedAt", "mailboxIds", "keywords", "textBody", "attachments", "bodyValues"],
                "fetchTextBodyValues": true
            }, "g"])])
            .await?;
        let mut email = r[0]["list"].get(0).cloned().context("no email with that id")?;
        let mut text = String::new();
        if let (Some(parts), Some(values)) = (email["textBody"].as_array(), email["bodyValues"].as_object()) {
            for part in parts {
                if let Some(value) = part["partId"].as_str().and_then(|pid| values.get(pid)) {
                    text.push_str(value["value"].as_str().unwrap_or(""));
                }
            }
        }
        if let Some(obj) = email.as_object_mut() {
            obj.remove("bodyValues");
            obj.insert("text".into(), Value::String(text));
        }
        Ok(email)
    }

    pub async fn attachments(&self, id: &str) -> Result<Vec<Value>> {
        let r = self
            .call(vec![json!(["Email/get", {
                "accountId": self.account_id,
                "ids": [id],
                "properties": ["attachments"]
            }, "g"])])
            .await?;
        let email = r[0]["list"].get(0).context("no email with that id")?;
        Ok(email["attachments"].as_array().cloned().unwrap_or_default())
    }

    pub async fn download(&self, blob_id: &str, name: &str, mime: &str) -> Result<Vec<u8>> {
        let url = self
            .download_url
            .replace("{accountId}", &self.account_id)
            .replace("{blobId}", &encode_segment(blob_id))
            .replace("{name}", &encode_segment(name))
            .replace("{type}", &encode_segment(mime));
        let resp = send(self.http.get(&url).bearer_auth(&self.token)).await?;
        if !resp.status().is_success() {
            bail!("download returned HTTP {}", resp.status());
        }
        Ok(resp.bytes().await.context("reading download")?.to_vec())
    }
}

impl Jmap {
    async fn update_email(&self, id: &str, patch: Value) -> Result<()> {
        let r = self
            .call(vec![json!(["Email/set", {
                "accountId": self.account_id,
                "update": { id: patch }
            }, "s"])])
            .await?;
        if let Some(err) = r[0]["notUpdated"].get(id) {
            bail!("JMAP refused the change: {}", err["description"].as_str().or(err["type"].as_str()).unwrap_or("unknown"));
        }
        Ok(())
    }

    pub async fn move_email(&self, id: &str, mailbox: &str) -> Result<()> {
        let target = self.mailbox_id(mailbox).await?;
        self.update_email(id, json!({ "mailboxIds": { target: true } })).await
    }

    /// Sets or clears a keyword such as `$flagged` or `$seen`.
    pub async fn set_keyword(&self, id: &str, keyword: &str, on: bool) -> Result<()> {
        let value = if on { json!(true) } else { Value::Null };
        self.update_email(id, json!({ format!("keywords/{keyword}"): value })).await
    }

    pub async fn trash(&self, id: &str) -> Result<()> {
        self.move_email(id, "trash").await
    }

    /// Creates a plain-text draft in Drafts and returns its id.
    /// Creates a draft in Drafts. `text` is the plain-text part. `html`, when given, is the HTML part.
    pub async fn create_draft(&self, from: &str, to: &[String], cc: &[String], subject: &str, text: &str, html: Option<&str>) -> Result<String> {
        let drafts = self.mailbox_id("drafts").await?;
        let addrs = |v: &[String]| v.iter().map(|e| json!({ "email": e })).collect::<Vec<_>>();
        let (structure, values) = match html {
            Some(h) => (
                json!({ "type": "multipart/alternative", "subParts": [
                    { "partId": "text", "type": "text/plain" },
                    { "partId": "html", "type": "text/html" }
                ] }),
                json!({ "text": { "value": text }, "html": { "value": h } }),
            ),
            None => (
                json!({ "partId": "text", "type": "text/plain" }),
                json!({ "text": { "value": text } }),
            ),
        };
        let r = self
            .call(vec![json!(["Email/set", {
                "accountId": self.account_id,
                "create": { "d": {
                    "mailboxIds": { drafts: true },
                    "keywords": { "$draft": true, "$seen": true },
                    "from": [{ "email": from }],
                    "to": addrs(to),
                    "cc": addrs(cc),
                    "subject": subject,
                    "bodyStructure": structure,
                    "bodyValues": values
                } }
            }, "c"])])
            .await?;
        if let Some(err) = r[0]["notCreated"].get("d") {
            bail!("JMAP refused the draft: {}", err["description"].as_str().or(err["type"].as_str()).unwrap_or("unknown"));
        }
        r[0]["created"]["d"]["id"].as_str().map(str::to_string).context("draft was not created")
    }

    /// Sends an existing draft. Needs the Email submission scope on the token.
    pub async fn send_draft(&self, email_id: &str, from: &str) -> Result<()> {
        if !self.has("urn:ietf:params:jmap:submission") {
            bail!("sending needs the Email submission scope on the JMAP token. Turn it on in Fastmail's API token settings and recreate the token");
        }
        let r = self
            .call_using(
                &[USING[0], USING[1], "urn:ietf:params:jmap:submission"],
                vec![json!(["Identity/get", { "accountId": self.account_id, "ids": null }, "i"])],
            )
            .await?;
        let identities = r[0]["list"].as_array().cloned().unwrap_or_default();
        let identity = identities
            .iter()
            .find(|i| i["email"].as_str() == Some(from))
            .or(identities.first())
            .and_then(|i| i["id"].as_str())
            .context("no sending identity on this account")?
            .to_string();
        let sent = self.mailbox_id("sent").await?;
        let r = self
            .call_using(&[USING[0], USING[1], "urn:ietf:params:jmap:submission"], vec![json!(["EmailSubmission/set", {
                "accountId": self.account_id,
                "create": { "s": { "emailId": email_id, "identityId": identity } },
                "onSuccessUpdateEmail": { "#s": {
                    "mailboxIds": { sent: true },
                    "keywords/$draft": null
                } }
            }, "x"])])
            .await?;
        if let Some(err) = r[0]["notCreated"].get("s") {
            bail!("JMAP refused the send: {}", err["description"].as_str().or(err["type"].as_str()).unwrap_or("unknown"));
        }
        Ok(())
    }

    /// The From address for drafts. Uses the account's first identity when submission is enabled,
    /// otherwise the login address.
    pub async fn identity_email(&self, fallback: &str) -> Result<String> {
        if !self.has("urn:ietf:params:jmap:submission") {
            return Ok(fallback.to_string());
        }
        let r = self
            .call_using(
                &[USING[0], USING[1], "urn:ietf:params:jmap:submission"],
                vec![json!(["Identity/get", { "accountId": self.account_id, "ids": null }, "i"])],
            )
            .await?;
        r[0]["list"]
            .as_array()
            .and_then(|l| l.first())
            .and_then(|i| i["email"].as_str())
            .map(str::to_string)
            .context("no sending identity on this account")
    }
}

/// One page of Email changes since a state string.
pub struct EmailChanges {
    pub created: Vec<String>,
    pub new_state: String,
    pub has_more: bool,
}

impl Jmap {
    /// The current Email state string, used as the starting point for change tracking.
    pub async fn email_state(&self) -> Result<String> {
        let r = self
            .call(vec![json!(["Email/get", { "accountId": self.account_id, "ids": [] }, "s"])])
            .await?;
        r[0]["state"].as_str().map(str::to_string).context("server did not report an Email state")
    }

    pub async fn email_changes(&self, since: &str) -> Result<EmailChanges> {
        let r = self
            .call(vec![json!(["Email/changes", { "accountId": self.account_id, "sinceState": since }, "c"])])
            .await?;
        Ok(EmailChanges {
            created: r[0]["created"]
                .as_array()
                .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
                .unwrap_or_default(),
            new_state: r[0]["newState"].as_str().context("Email/changes gave no newState")?.to_string(),
            has_more: r[0]["hasMoreChanges"].as_bool().unwrap_or(false),
        })
    }

    pub async fn email_summaries(&self, ids: &[String]) -> Result<Vec<Value>> {
        let r = self
            .call(vec![json!(["Email/get", {
                "accountId": self.account_id,
                "ids": ids,
                "properties": ["id", "subject", "from", "receivedAt", "mailboxIds"]
            }, "g"])])
            .await?;
        Ok(r[0]["list"].as_array().cloned().unwrap_or_default())
    }
}

const VACATION: &str = "urn:ietf:params:jmap:vacationresponse";

impl Jmap {
    fn require_vacation(&self) -> Result<()> {
        if !self.has(VACATION) {
            bail!("vacation replies need the Vacation response scope on the JMAP token");
        }
        Ok(())
    }

    /// The account's vacation reply settings. Fastmail keeps one set, with the id "singleton".
    pub async fn vacation(&self) -> Result<Value> {
        self.require_vacation()?;
        let r = self
            .call_using(
                &[USING[0], VACATION],
                vec![json!(["VacationResponse/get", { "accountId": self.account_id, "ids": ["singleton"] }, "v"])],
            )
            .await?;
        r[0]["list"].get(0).cloned().context("no vacation reply settings found")
    }

    pub async fn set_vacation(&self, patch: Value) -> Result<()> {
        self.require_vacation()?;
        let r = self
            .call_using(
                &[USING[0], VACATION],
                vec![json!(["VacationResponse/set", {
                    "accountId": self.account_id,
                    "update": { "singleton": patch }
                }, "v"])],
            )
            .await?;
        if let Some(err) = r[0]["notUpdated"].get("singleton") {
            bail!("JMAP refused the change: {}", err["description"].as_str().or(err["type"].as_str()).unwrap_or("unknown"));
        }
        Ok(())
    }
}

const MASKED: &str = "https://www.fastmail.com/dev/maskedemail";

impl Jmap {
    fn require_masked(&self) -> Result<()> {
        if !self.has(MASKED) {
            bail!("masked email needs the Masked Email permission on the JMAP token");
        }
        Ok(())
    }

    pub async fn masked_list(&self) -> Result<Vec<Value>> {
        self.require_masked()?;
        let r = self
            .call_using(&[USING[0], MASKED], vec![json!(["MaskedEmail/get", { "accountId": self.account_id, "ids": null }, "m"])])
            .await?;
        Ok(r[0]["list"].as_array().cloned().unwrap_or_default())
    }

    /// Creates a masked address for a site. It stays pending until mail arrives or it's enabled.
    pub async fn masked_create(&self, for_domain: &str, description: &str, prefix: Option<&str>) -> Result<Value> {
        self.require_masked()?;
        let mut item = json!({ "forDomain": for_domain, "description": description });
        if let Some(p) = prefix {
            item["emailPrefix"] = json!(p);
        }
        let r = self
            .call_using(&[USING[0], MASKED], vec![json!(["MaskedEmail/set", {
                "accountId": self.account_id,
                "create": { "m": item }
            }, "c"])])
            .await?;
        if let Some(err) = r[0]["notCreated"].get("m") {
            bail!("JMAP refused the masked address: {}", err["description"].as_str().or(err["type"].as_str()).unwrap_or("unknown"));
        }
        Ok(r[0]["created"]["m"].clone())
    }

    /// Sets a masked address's state: enabled, disabled, or deleted.
    pub async fn masked_set_state(&self, id: &str, state: &str) -> Result<()> {
        self.require_masked()?;
        let r = self
            .call_using(&[USING[0], MASKED], vec![json!(["MaskedEmail/set", {
                "accountId": self.account_id,
                "update": { id: { "state": state } }
            }, "u"])])
            .await?;
        if let Some(err) = r[0]["notUpdated"].get(id) {
            bail!("JMAP refused the change: {}", err["description"].as_str().or(err["type"].as_str()).unwrap_or("unknown"));
        }
        Ok(())
    }
}
