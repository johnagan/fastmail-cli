//! Apple-compatible notes are messages, never EmailSubmission objects.
use super::JmapClient;
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};

const NOTE_TYPE: &str = "com.apple.mail-note";
const NOTE_HEADER: &str = "header:X-Uniform-Type-Identifier:asText";
const UUID_HEADER: &str = "header:X-Universally-Unique-Identifier:asText";

#[derive(Default)]
pub struct NoteEdit {
    pub title: Option<String>,
    pub body: Option<String>,
    pub html_body: Option<String>,
    pub attachments: Option<Vec<super::AttachmentData>>,
}

impl JmapClient {
    pub async fn create_rich_note(&self, edit: NoteEdit) -> Result<Value> {
        let mailbox = self.notes_mailbox().await?;
        let mut note = json!({"mailboxIds": {mailbox: true}, "keywords": {}, "subject": edit.title.as_deref().unwrap_or(""), NOTE_HEADER: NOTE_TYPE, UUID_HEADER: uuid::Uuid::new_v4().to_string().to_uppercase()});
        self.apply_note_edit(&mut note, None, edit).await?;
        self.save_note(note).await
    }

    /// Preserve the note UUID, mailbox memberships, flags, rich bodies, and
    /// attachment blobs, saving the replacement before deleting the original.
    pub async fn update_rich_note(&self, id: &str, edit: NoteEdit) -> Result<Value> {
        ensure!(
            edit.title.is_some()
                || edit.body.is_some()
                || edit.html_body.is_some()
                || edit.attachments.is_some(),
            "Provide at least one note change"
        );
        let old = self.get_note(id).await?;
        ensure!(
            old[UUID_HEADER].is_string(),
            "Note is missing its unique identifier"
        );
        let mut note = json!({"mailboxIds": old["mailboxIds"], "keywords": old["keywords"], "subject": old["subject"], NOTE_HEADER: NOTE_TYPE, UUID_HEADER: old[UUID_HEADER]});
        self.apply_note_edit(&mut note, Some(&old), edit).await?;
        let created = self.save_note(note).await?;
        self.delete_note(id).await.with_context(|| {
            format!(
                "Replacement note {} saved, but old note {id} could not be deleted",
                created["id"]
            )
        })?;
        Ok(created)
    }

    async fn apply_note_edit(
        &self,
        note: &mut Value,
        old: Option<&Value>,
        edit: NoteEdit,
    ) -> Result<()> {
        if let Some(title) = edit.title {
            note["subject"] = json!(title);
        }
        ensure!(
            !note["subject"]
                .as_str()
                .unwrap_or("")
                .contains(['\r', '\n']),
            "Note title cannot contain newlines"
        );
        let old_text = note_body(old, "textBody")?;
        let old_html = note_body(old, "htmlBody")?;
        let replacing_text = edit.body.is_some();
        let text = edit.body.unwrap_or_else(|| old_text.unwrap_or_default());
        let html = edit
            .html_body
            .or(if replacing_text { None } else { old_html });
        let mut attachments = Vec::new();
        if let Some(old) = old {
            for part in old["attachments"].as_array().into_iter().flatten() {
                let cid = part["cid"]
                    .as_str()
                    .filter(|_| part["disposition"] == "inline");
                let referenced_inline = cid.is_some_and(|cid| {
                    html.as_deref()
                        .is_some_and(|h| h.contains(&format!("cid:{cid}")))
                });
                if edit.attachments.is_none() || referenced_inline {
                    let blob = part["blobId"]
                        .as_str()
                        .context("Note attachment is missing its blobId")?;
                    attachments.push(super::UploadedAttachment {
                        blob_id: blob.into(),
                        filename: part["name"].as_str().unwrap_or("attachment").into(),
                        content_type: part["type"]
                            .as_str()
                            .unwrap_or("application/octet-stream")
                            .into(),
                        inline_cid: cid.map(str::to_owned),
                    });
                }
            }
        }
        if let Some(new) = edit.attachments {
            attachments.extend(self.upload_attachments(new).await?);
        }
        let mut fields = std::collections::HashMap::new();
        super::apply_body_structure(&mut fields, &text, html.as_deref(), &attachments);
        note.as_object_mut()
            .context("Expected note object")?
            .extend(fields);
        Ok(())
    }

    async fn save_note(&self, note: Value) -> Result<Value> {
        let response = self
            .note_call("Email/set", json!({"create": {"note": note}}))
            .await?;
        let created = &response["created"]["note"];
        ensure!(
            created["id"].is_string(),
            "Could not create note: {}",
            response["notCreated"]
        );
        Ok(created.clone())
    }
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
    /// Returns the new message ID, preserving omitted rich content and attachments.
    pub async fn update_note(
        &self,
        id: &str,
        title: Option<&str>,
        body: Option<&str>,
    ) -> Result<Value> {
        self.update_rich_note(
            id,
            NoteEdit {
                title: title.map(str::to_owned),
                body: body.map(str::to_owned),
                ..Default::default()
            },
        )
        .await
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

fn note_body(note: Option<&Value>, kind: &str) -> Result<Option<String>> {
    let Some(note) = note else {
        return Ok(None);
    };
    let mut bodies = Vec::new();
    for part in note[kind].as_array().into_iter().flatten() {
        let id = part["partId"]
            .as_str()
            .context("Note body is missing its partId")?;
        let value = &note["bodyValues"][id];
        ensure!(
            value["isTruncated"] != true && value["isEncodingProblem"] != true,
            "Cannot replace an incomplete note body"
        );
        bodies.push(value["value"].as_str().context("Missing note body value")?);
    }
    Ok((!bodies.is_empty()).then(|| bodies.join("\n")))
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
                    assert!(note["bodyValues"].as_object().unwrap().values().any(|value| value["value"] == "Keep this"));
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

    #[tokio::test]
    async fn rich_note_edits_preserve_bodies_memberships_flags_and_inline_blobs() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).respond_with(|request: &wiremock::Request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            let call = &body["methodCalls"][0];
            let result = match call[0].as_str().unwrap() {
                "Mailbox/get" => json!({"list": [{"id": "notes", "name": "Notes"}]}),
                "Email/get" => json!({"list": [{"id": "n1", "subject": "Old", "mailboxIds": {"notes": true, "other": true}, "keywords": {"$flagged": true}, NOTE_HEADER: NOTE_TYPE, UUID_HEADER: "uid", "textBody": [{"partId": "text"}], "htmlBody": [{"partId": "html"}], "bodyValues": {"text": {"value": "Keep text"}, "html": {"value": "<img src=\"cid:image\">Keep HTML"}}, "attachments": [{"blobId": "image-blob", "name": "image.png", "type": "image/png", "cid": "image", "disposition": "inline"}, {"blobId": "file-blob", "name": "file.txt", "type": "text/plain", "disposition": "attachment"}]}]}),
                "Email/set" if call[1]["create"].is_object() => {
                    let note = &call[1]["create"]["note"];
                    assert_eq!(note[UUID_HEADER], "uid");
                    assert_eq!(note["subject"], "New");
                    assert_eq!(note["mailboxIds"], json!({"notes": true, "other": true}));
                    assert_eq!(note["keywords"], json!({"$flagged": true}));
                    assert_eq!(note["bodyValues"]["textBody"]["value"], "Keep text");
                    assert_eq!(note["bodyValues"]["htmlBody"]["value"], "<img src=\"cid:image\">Keep HTML");
                    let parts = note["bodyStructure"]["subParts"].as_array().unwrap();
                    assert_eq!(parts.len(), 2, "Clearing regular attachments must retain referenced inline images");
                    assert_eq!(parts[1]["blobId"], "image-blob");
                    assert_eq!(parts[1]["disposition"], "inline");
                    assert_eq!(parts[1]["cid"], "image");
                    json!({"created": {"note": {"id": "n2"}}})
                }
                "Email/set" => json!({"destroyed": ["n1"]}),
                _ => panic!("Unexpected method"),
            };
            ResponseTemplate::new(200).set_body_json(json!({"methodResponses": [[call[0], result, call[2]]]}))
        }).mount(&server).await;
        let client = JmapClient::with_test_session(&server.uri());
        let result = client
            .update_rich_note(
                "n1",
                NoteEdit {
                    title: Some("New".into()),
                    attachments: Some(vec![]),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(result["id"], "n2");
        let methods: Vec<_> = server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .map(|r| serde_json::from_slice::<Value>(&r.body).unwrap()["methodCalls"][0][0].clone())
            .collect();
        assert!(!methods.iter().any(|m| m == "EmailSubmission/set"));
    }

    #[tokio::test]
    async fn rich_replacement_failure_keeps_original() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).respond_with(|request: &wiremock::Request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            let call = &body["methodCalls"][0];
            let result = match call[0].as_str().unwrap() {
                "Mailbox/get" => json!({"list": [{"id": "notes", "name": "Notes"}]}),
                "Email/get" => json!({"list": [{"id": "n1", "subject": "Old", "mailboxIds": {"notes": true}, "keywords": {}, NOTE_HEADER: NOTE_TYPE, UUID_HEADER: "uid", "textBody": [], "htmlBody": [], "attachments": []}]}),
                "Email/set" => { assert!(call[1]["destroy"].is_null()); json!({"notCreated": {"note": {"type": "forbidden"}}}) },
                _ => panic!("Unexpected method"),
            };
            ResponseTemplate::new(200).set_body_json(json!({"methodResponses": [[call[0], result, call[2]]]}))
        }).mount(&server).await;
        let client = JmapClient::with_test_session(&server.uri());
        assert!(
            client
                .update_rich_note(
                    "n1",
                    NoteEdit {
                        html_body: Some("<b>New</b>".into()),
                        ..Default::default()
                    }
                )
                .await
                .is_err()
        );
    }
}
