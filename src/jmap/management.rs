use super::*;

impl JmapClient {
    pub async fn create_masked_email_options(&self, properties: Value) -> Result<Value> {
        self.require_capability("https://www.fastmail.com/dev/maskedemail", "Masked email")?;
        self.call(
            "MaskedEmail/set",
            json!({"accountId": self.account_id()?, "create": {"new": properties}}),
        )
        .await
    }

    pub async fn update_masked_email_options(&self, id: &str, properties: Value) -> Result<Value> {
        self.require_capability("https://www.fastmail.com/dev/maskedemail", "Masked email")?;
        self.call(
            "MaskedEmail/set",
            json!({"accountId": self.account_id()?, "update": {id: properties}}),
        )
        .await
    }
    pub async fn star_email(&self, id: &str, starred: bool) -> Result<Value> {
        let flag = if starred { json!(true) } else { Value::Null };
        self.call(
            "Email/set",
            json!({"accountId": self.account_id()?, "update": {id: {"keywords/$flagged": flag}}}),
        )
        .await
    }
    /// Whether this session advertises a capability.
    pub fn supports(&self, capability: &str) -> bool {
        self.session
            .as_ref()
            .is_some_and(|s| s.capabilities.contains_key(capability))
    }

    pub(super) fn capability_account(&self, capability: &str) -> Result<&str> {
        self.require_capability(capability, "This operation")?;
        self.account_for(capability)
    }

    pub fn account_for(&self, capability: &str) -> Result<&str> {
        if let Some(id) = &self.selected_account {
            let account = self
                .session()?
                .accounts
                .get(id)
                .ok_or_else(|| Error::Config(format!("Unknown account: {id}")))?;
            if !account.account_capabilities.contains_key(capability) {
                return Err(Error::Config(format!(
                    "Account {id} does not support {capability}"
                )));
            }
            return Ok(id);
        }
        self.session()?
            .primary_accounts
            .get(capability)
            .map(String::as_str)
            .ok_or_else(|| Error::Config(format!("No primary account for {capability}")))
    }

    pub fn select_account(&mut self, id: Option<String>) -> Result<()> {
        if let Some(id) = &id
            && !self.session()?.accounts.contains_key(id)
        {
            return Err(Error::Config(format!("Unknown account: {id}")));
        }
        self.selected_account = id;
        self.cached_mailboxes = None;
        Ok(())
    }

    /// Execute one method, preserving method-level and per-object errors.
    pub(super) async fn call(&self, method: &str, args: Value) -> Result<Value> {
        let responses = self.request(vec![json!([method, args, "0"])]).await?;
        let value: Value = Self::parse_response(responses.first().unwrap_or(&Value::Null), method)?;
        // A copy that destroys originals generates an additional /set response.
        // Report failures there too rather than implying that the source was removed.
        for (index, response) in responses.iter().enumerate() {
            let response_method = if index == 0 {
                method
            } else {
                response[0].as_str().unwrap_or(method)
            };
            let result: Value = Self::parse_response(response, response_method)?;
            for key in ["notCreated", "notUpdated", "notDestroyed"] {
                if let Some(errors) = result[key].as_object()
                    && let Some((id, error)) = errors.iter().next()
                {
                    return Err(Error::Jmap {
                        method: response_method.into(),
                        error_type: error["type"].as_str().unwrap_or("unknown").into(),
                        description: format!(
                            "{} {id}: {}",
                            if index == 0 {
                                "Operation failed for"
                            } else {
                                "Primary operation completed, but follow-up failed for"
                            },
                            error["description"].as_str().unwrap_or("Operation failed")
                        ),
                    });
                }
            }
        }
        Ok(value)
    }

    pub async fn create_mailbox(&mut self, properties: Value) -> Result<Value> {
        let result = self
            .call(
                "Mailbox/set",
                json!({"accountId": self.account_id()?, "create": {"new": properties}}),
            )
            .await?;
        self.cached_mailboxes = None;
        Ok(result["created"]["new"].clone())
    }

    pub async fn update_mailbox(&mut self, id: &str, properties: Value) -> Result<Value> {
        let result = self
            .call(
                "Mailbox/set",
                json!({"accountId": self.account_id()?, "update": {id: properties}}),
            )
            .await?;
        self.cached_mailboxes = None;
        Ok(result)
    }

    pub async fn delete_mailbox(&mut self, id: &str, remove_emails: bool) -> Result<Value> {
        let result = self.call("Mailbox/set", json!({"accountId": self.account_id()?, "destroy": [id], "onDestroyRemoveEmails": remove_emails})).await?;
        self.cached_mailboxes = None;
        Ok(result)
    }

    pub async fn vacation_response(&self) -> Result<Value> {
        let account = self.capability_account("urn:ietf:params:jmap:vacationresponse")?;
        let result = self
            .call(
                "VacationResponse/get",
                json!({"accountId": account, "ids": ["singleton"]}),
            )
            .await?;
        Ok(result["list"].clone())
    }

    pub async fn set_vacation_response(&self, properties: Value) -> Result<Value> {
        let account = self.capability_account("urn:ietf:params:jmap:vacationresponse")?;
        self.call(
            "VacationResponse/set",
            json!({"accountId": account, "update": {"singleton": properties}}),
        )
        .await
    }

    /// Upload and import an RFC 5322 message without submitting it for delivery.
    pub async fn import_email(
        &self,
        bytes: Vec<u8>,
        mailbox_id: &str,
        keywords: Value,
        received_at: Option<&str>,
    ) -> Result<Value> {
        let blob = self.upload_blob(bytes, "message/rfc822").await?;
        let mut email =
            json!({"blobId": blob, "mailboxIds": {mailbox_id: true}, "keywords": keywords});
        if let Some(date) = received_at {
            email["receivedAt"] = json!(date);
        }
        let result = self
            .call(
                "Email/import",
                json!({"accountId": self.account_id()?, "emails": {"new": email}}),
            )
            .await?;
        Ok(result["created"]["new"].clone())
    }

    pub async fn search_snippets(&self, filter: Value, email_ids: &[String]) -> Result<Value> {
        self.call(
            "SearchSnippet/get",
            json!({"accountId": self.account_id()?, "filter": filter, "emailIds": email_ids}),
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn masked_metadata_and_pending_creation_preserve_omitted_properties() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(|request: &wiremock::Request| {
                let body: Value = serde_json::from_slice(&request.body).unwrap();
                let call = &body["methodCalls"][0];
                assert_eq!(call[0], "MaskedEmail/set");
                let result = if call[1]["create"].is_object() {
                    assert_eq!(
                        call[1]["create"]["new"],
                        json!({"state": "pending", "url": "app://record"})
                    );
                    json!({"created": {"new": {"id": "m1"}}})
                } else {
                    assert_eq!(
                        call[1]["update"]["m1"],
                        json!({"description": "New", "url": null})
                    );
                    json!({"updated": {"m1": null}})
                };
                wiremock::ResponseTemplate::new(200)
                    .set_body_json(json!({"methodResponses": [[call[0], result, call[2]]]}))
            })
            .mount(&server)
            .await;
        let client = JmapClient::with_test_session(&server.uri());
        client
            .create_masked_email_options(json!({"state": "pending", "url": "app://record"}))
            .await
            .unwrap();
        client
            .update_masked_email_options("m1", json!({"description": "New", "url": null}))
            .await
            .unwrap();
    }
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    #[tokio::test]
    async fn mailbox_writes_invalidate_the_cache_and_report_object_errors() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).respond_with(|req: &wiremock::Request| {
            let request: Value = serde_json::from_slice(&req.body).unwrap();
            let call = &request["methodCalls"][0];
            let result = match call[0].as_str().unwrap() {
                "Mailbox/get" => json!({"list": []}),
                "Mailbox/set" if call[1]["create"].is_object() => json!({"created": {"new": {"id": "folder"}}}),
                "Mailbox/set" if call[1]["update"].is_object() => json!({"updated": {"folder": null}}),
                _ => json!({"notDestroyed": {"folder": {"type": "mailboxHasEmail", "description": "Nonempty mailbox"}}}),
            };
            ResponseTemplate::new(200).set_body_json(json!({"methodResponses": [[call[0], result, "0"]]}))
        }).mount(&server).await;
        let mut client = JmapClient::with_test_session(&server.uri());
        client.list_mailboxes().await.unwrap();
        assert!(client.cached_mailboxes.is_some());
        assert_eq!(
            client
                .create_mailbox(json!({"name": "Projects", "parentId": "parent"}))
                .await
                .unwrap()["id"],
            "folder"
        );
        assert!(client.cached_mailboxes.is_none());
        client.list_mailboxes().await.unwrap();
        client
            .update_mailbox("folder", json!({"parentId": null, "isSubscribed": false}))
            .await
            .unwrap();
        assert!(client.cached_mailboxes.is_none());
        assert!(
            client
                .delete_mailbox("folder", false)
                .await
                .unwrap_err()
                .to_string()
                .contains("mailboxHasEmail")
        );
        let requests = server.received_requests().await.unwrap();
        let delete: Value = serde_json::from_slice(&requests.last().unwrap().body).unwrap();
        assert_eq!(delete["methodCalls"][0][1]["onDestroyRemoveEmails"], false);
    }

    #[tokio::test]
    async fn vacation_uses_its_capability_account_and_partial_updates() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(|req: &wiremock::Request| {
                let request: Value = serde_json::from_slice(&req.body).unwrap();
                let call = &request["methodCalls"][0];
                assert_eq!(call[1]["accountId"], "vacation-account");
                if call[0] == "VacationResponse/set" {
                    assert_eq!(
                        call[1]["update"],
                        json!({"singleton": {"isEnabled": false}})
                    );
                } else {
                    assert_eq!(call[1]["ids"], json!(["singleton"]));
                }
                ResponseTemplate::new(200).set_body_json(
                    json!({"methodResponses": [[call[0], {"list": [{"id": "singleton"}]}, "0"]]}),
                )
            })
            .mount(&server)
            .await;
        let mut client = JmapClient::with_test_session(&server.uri());
        assert!(client.vacation_response().await.is_err());
        let session = client.session.as_mut().unwrap();
        session
            .capabilities
            .insert("urn:ietf:params:jmap:vacationresponse".into(), json!({}));
        session.primary_accounts.insert(
            "urn:ietf:params:jmap:vacationresponse".into(),
            "vacation-account".into(),
        );
        client.vacation_response().await.unwrap();
        client
            .set_vacation_response(json!({"isEnabled": false}))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn import_uploads_original_bytes_and_does_not_submit() {
        let server = MockServer::start().await;
        let message = b"From: alice@example.com\r\nSubject: Original\r\n\r\nBody\r\n";
        Mock::given(method("POST"))
            .and(path("/upload"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"blobId": "raw-blob"})))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST")).and(path("/")).respond_with(|req: &wiremock::Request| {
            let request: Value = serde_json::from_slice(&req.body).unwrap();
            assert_eq!(request["methodCalls"].as_array().unwrap().len(), 1);
            assert_eq!(request["methodCalls"][0][0], "Email/import");
            assert_eq!(request["methodCalls"][0][1]["emails"]["new"], json!({"blobId": "raw-blob", "mailboxIds": {"inbox": true}, "keywords": {"$seen": true}, "receivedAt": "2026-10-16T00:00:00Z"}));
            ResponseTemplate::new(200).set_body_json(json!({"methodResponses": [["Email/import", {"created": {"new": {"id": "imported"}}}, "0"]]}))
        }).expect(1).mount(&server).await;
        let client = JmapClient::with_test_session(&server.uri());
        assert_eq!(
            client
                .import_email(
                    message.to_vec(),
                    "inbox",
                    json!({"$seen": true}),
                    Some("2026-10-16T00:00:00Z")
                )
                .await
                .unwrap()["id"],
            "imported"
        );
        let requests = server.received_requests().await.unwrap();
        let upload = requests.iter().find(|r| r.url.path() == "/upload").unwrap();
        assert_eq!(upload.body, message);
        assert_eq!(upload.headers["content-type"], "message/rfc822");
    }

    #[tokio::test]
    async fn snippets_preserve_the_filter_and_not_found_ids() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).respond_with(|req: &wiremock::Request| {
            let request: Value = serde_json::from_slice(&req.body).unwrap();
            let call = &request["methodCalls"][0];
            assert_eq!(call[0], "SearchSnippet/get");
            assert_eq!(call[1]["filter"], json!({"text": "project", "inMailbox": "inbox"}));
            assert_eq!(call[1]["emailIds"], json!(["e1", "missing"]));
            ResponseTemplate::new(200).set_body_json(json!({"methodResponses": [["SearchSnippet/get", {"list": [{"emailId": "e1", "subject": null, "preview": "<mark>project</mark>"}], "notFound": ["missing"]}, "0"]]}))
        }).mount(&server).await;
        let client = JmapClient::with_test_session(&server.uri());
        let result = client
            .search_snippets(
                json!({"text": "project", "inMailbox": "inbox"}),
                &["e1".into(), "missing".into()],
            )
            .await
            .unwrap();
        assert_eq!(result["notFound"], json!(["missing"]));
    }

    #[tokio::test]
    async fn star_and_unstar_patch_only_the_flagged_keyword() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"methodResponses": [["Email/set", {"updated": {"e1": null}}, "0"]]}),
            ))
            .mount(&server)
            .await;
        let client = JmapClient::with_test_session(&server.uri());
        client.star_email("e1", true).await.unwrap();
        client.star_email("e1", false).await.unwrap();
        for (req, value) in server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .zip([json!(true), Value::Null])
        {
            let request: Value = serde_json::from_slice(&req.body).unwrap();
            assert_eq!(
                request["methodCalls"][0][1]["update"],
                json!({"e1": {"keywords/$flagged": value}})
            );
        }
    }
}
