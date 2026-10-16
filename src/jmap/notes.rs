//! Apple-compatible notes are messages, never EmailSubmission objects.
use super::JmapClient;
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};

const NOTE_TYPE: &str = "com.apple.mail-note";
const NOTE_HEADER: &str = "header:X-Uniform-Type-Identifier:asText";
const UUID_HEADER: &str = "header:X-Universally-Unique-Identifier:asText";

impl JmapClient {
    async fn note_call(&self, method: &str, mut args: Value) -> Result<Value> {
        args["accountId"] = json!(self.account_id()?);
        let responses = self.request(vec![json!([method, args, "notes"])]).await?;
        let response = responses.first().context("Missing JMAP notes response")?;
        ensure!(
            response[0] == method || response[0] == "error",
            "Unexpected JMAP notes response"
        );
        Ok(Self::parse_response(response, method)?)
    }

    async fn notes_mailbox(&self) -> Result<String> {
        let mailboxes = self.fetch_mailboxes().await?;
        let mut matches: Vec<_> = mailboxes
            .iter()
            .filter(|m| m.role.as_deref() == Some("memos"))
            .collect();
        if matches.is_empty() {
            matches = mailboxes.iter().filter(|m| m.name == "Notes").collect();
        }
        ensure!(
            matches.len() == 1,
            "Expected one notes mailbox (role memos, or name Notes); found {}",
            matches.len()
        );
        Ok(matches[0].id.clone())
    }

    pub async fn list_notes(&self, text: Option<&str>, limit: u32) -> Result<Value> {
        ensure!(limit > 0, "Limit must be positive");
        let mailbox = self.notes_mailbox().await?;
        let mut filter =
            json!({"inMailbox": mailbox, "header": ["X-Uniform-Type-Identifier", NOTE_TYPE]});
        if let Some(text) = text {
            filter["text"] = json!(text);
        }
        let query = self.note_call("Email/query", json!({"filter": filter, "sort": [{"property": "receivedAt", "isAscending": false}], "limit": limit, "calculateTotal": true})).await?;
        let ids = query["ids"].as_array().context("Missing note IDs")?;
        let notes = if ids.is_empty() {
            json!([])
        } else {
            self.note_call("Email/get", json!({"ids": ids, "properties": ["id", "subject", "preview", "receivedAt", "keywords"]})).await?["list"].clone()
        };
        Ok(json!({"notes": notes, "total": query["total"], "queryState": query["queryState"]}))
    }

    pub async fn get_note(&self, id: &str) -> Result<Value> {
        let mailbox = self.notes_mailbox().await?;
        let response = self.note_call("Email/get", json!({"ids": [id], "properties": ["id", "subject", "mailboxIds", "keywords", "textBody", "htmlBody", "attachments", "bodyValues", NOTE_HEADER, UUID_HEADER], "fetchAllBodyValues": true})).await?;
        let note = response["list"]
            .as_array()
            .and_then(|l| l.first())
            .context("Note not found")?
            .clone();
        ensure!(
            note["mailboxIds"][&mailbox] == true && note[NOTE_HEADER] == NOTE_TYPE,
            "Message is not a Fastmail note"
        );
        Ok(note)
    }

    pub async fn create_note(&self, title: &str, body: &str) -> Result<Value> {
        let mailbox = self.notes_mailbox().await?;
        let uid = uuid::Uuid::new_v4().to_string().to_uppercase();
        self.write_note(&mailbox, title, body, &uid, json!({}))
            .await
    }

    async fn write_note(
        &self,
        mailbox: &str,
        title: &str,
        body: &str,
        uid: &str,
        keywords: Value,
    ) -> Result<Value> {
        ensure!(
            !title.contains(['\r', '\n']),
            "Note title cannot contain newlines"
        );
        let response = self
            .note_call(
                "Email/set",
                json!({"create": {"note": {
                    "mailboxIds": {mailbox: true}, "keywords": keywords, "subject": title,
                    "header:X-Uniform-Type-Identifier:asText": NOTE_TYPE,
                    "header:X-Universally-Unique-Identifier:asText": uid,
                    "textBody": [{"partId": "body", "type": "text/plain"}],
                    "bodyValues": {"body": {"value": body, "charset": "utf-8"}}
                }}}),
            )
            .await?;
        let created = &response["created"]["note"];
        ensure!(
            created["id"].is_string(),
            "Could not create note: {}",
            response["notCreated"]
        );
        Ok(created.clone())
    }

    /// Notes are immutable messages: replace only after the new message is saved.
    /// Returns the new message ID. Existing rich notes must be edited in Fastmail.
    pub async fn update_note(
        &self,
        id: &str,
        title: Option<&str>,
        body: Option<&str>,
    ) -> Result<Value> {
        ensure!(
            title.is_some() || body.is_some(),
            "Provide --title or --body"
        );
        let note = self.get_note(id).await?;
        ensure!(
            note["htmlBody"].as_array().is_none_or(|a| a.is_empty())
                && note["attachments"].as_array().is_none_or(|a| a.is_empty()),
            "This note contains rich text; edit it in Fastmail to preserve formatting and attachments"
        );
        let parts = note["textBody"]
            .as_array()
            .context("Note has no editable text body")?;
        ensure!(
            parts.len() == 1,
            "Only single-part plain-text notes can be updated"
        );
        let part = parts[0]["partId"]
            .as_str()
            .context("Missing note body part")?;
        let value = &note["bodyValues"][part];
        ensure!(
            value["isTruncated"] != true && value["isEncodingProblem"] != true,
            "Cannot safely replace an incomplete note body"
        );
        let old_body = value["value"].as_str().context("Missing note body value")?;
        let mailbox = self.notes_mailbox().await?;
        let uid = note[UUID_HEADER]
            .as_str()
            .context("Note is missing its unique identifier")?;
        let created = self
            .write_note(
                &mailbox,
                title.unwrap_or(note["subject"].as_str().unwrap_or("")),
                body.unwrap_or(old_body),
                uid,
                note["keywords"].clone(),
            )
            .await?;
        self.delete_note(id).await.with_context(|| {
            format!(
                "Replacement note {} saved, but old note {id} could not be deleted",
                created["id"]
            )
        })?;
        Ok(created)
    }

    pub async fn delete_note(&self, id: &str) -> Result<()> {
        self.get_note(id).await?;
        let response = self
            .note_call("Email/set", json!({"destroy": [id]}))
            .await?;
        ensure!(
            response["destroyed"]
                .as_array()
                .is_some_and(|ids| ids.iter().any(|v| v == id)),
            "Could not delete note: {}",
            response["notDestroyed"]
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};

    #[tokio::test]
    async fn notes_mailbox_uses_role_before_name() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "methodResponses": [["Mailbox/get", {"list": [
                    {"id": "ordinary-folder", "name": "Notes"},
                    {"id": "notes-folder", "name": "Memos", "role": "memos"}
                ]}, "m0"]]
            })))
            .mount(&server)
            .await;
        let client = JmapClient::with_test_session(&server.uri());
        assert_eq!(client.notes_mailbox().await.unwrap(), "notes-folder");
    }

    async fn mock_notes_server(fail_create: bool) -> (MockServer, JmapClient) {
        let server = MockServer::start().await;
        Mock::given(method("POST")).respond_with(move |request: &wiremock::Request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            let call = &body["methodCalls"][0];
            let name = call[0].as_str().unwrap();
            let args = &call[1];
            let result = match name {
                "Mailbox/get" => json!({"list": [{"id": "notes-folder", "name": "Notes"}]}),
                "Email/get" => json!({"list": [{"id": "n1", "subject": "Old", "mailboxIds": {"notes-folder": true}, "keywords": {"$flagged": true}, NOTE_HEADER: NOTE_TYPE, UUID_HEADER: "existing-uid", "textBody": [{"partId": "text"}], "htmlBody": [], "attachments": [], "bodyValues": {"text": {"value": "Keep this"}}}]}),
                "Email/set" if args["create"].is_object() => {
                    let note = &args["create"]["note"];
                    assert_eq!(note["mailboxIds"]["notes-folder"], true);
                    assert_eq!(note[NOTE_HEADER], NOTE_TYPE);
                    assert_eq!(note[UUID_HEADER], "existing-uid");
                    assert_eq!(note["bodyValues"]["body"]["value"], "Keep this");
                    assert_eq!(note["keywords"]["$flagged"], true);
                    if fail_create { json!({"notCreated": {"note": {"type": "forbidden"}}}) }
                    else { json!({"created": {"note": {"id": "n2"}}}) }
                }
                "Email/set" => {
                    assert!(!fail_create, "Old note must survive a failed replacement");
                    assert_eq!(args["destroy"], json!(["n1"]));
                    json!({"destroyed": ["n1"]})
                }
                _ => panic!("Unexpected method {name}"),
            };
            ResponseTemplate::new(200).set_body_json(json!({"methodResponses": [[name, result, call[2]]]}))
        }).mount(&server).await;
        let client = JmapClient::with_test_session(&server.uri());
        (server, client)
    }

    #[tokio::test]
    async fn replacement_preserves_uid_body_and_keywords() {
        let (_server, client) = mock_notes_server(false).await;
        let created = client.update_note("n1", Some("New"), None).await.unwrap();
        assert_eq!(created["id"], "n2");
    }

    #[tokio::test]
    async fn failed_replacement_never_deletes_original() {
        let (server, client) = mock_notes_server(true).await;
        assert!(client.update_note("n1", Some("New"), None).await.is_err());
        let requests = server.received_requests().await.unwrap();
        assert!(!requests.iter().any(|r| {
            let body: Value = serde_json::from_slice(&r.body).unwrap();
            body["methodCalls"][0][1]["destroy"].is_array()
        }));
    }
}
