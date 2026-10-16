use super::*;
use crate::contacts::ContactsClient;
use std::sync::Arc;
use tokio::sync::Mutex;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};

fn client(server: &MockServer) -> JmapClient {
    let mut client = JmapClient::with_test_session(&server.uri());
    let session = client.session.as_mut().unwrap();
    session.capabilities.insert(CONTACTS.into(), json!({}));
    session
        .primary_accounts
        .insert(CONTACTS.into(), "contacts-account".into());
    client
}

#[tokio::test]
async fn contacts_prefer_jmap_without_dav_credentials_and_follow_server_pages() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).respond_with(|req: &wiremock::Request| {
        let request: Value = serde_json::from_slice(&req.body).unwrap();
        let call = &request["methodCalls"][0];
        assert_eq!(call[1]["accountId"], "contacts-account");
        let result = if call[0] == "ContactCard/query" {
            assert_eq!(call[1]["filter"], json!({"text": "Alice"}));
            match call[1]["position"].as_u64().unwrap() {
                0 => json!({"ids": ["c1"]}),
                1 => json!({"ids": ["c2"]}),
                2 => json!({"ids": []}),
                _ => panic!("Unexpected page"),
            }
        } else {
            json!({"list": [{"id": call[1]["ids"][0], "name": {"full": "Alice"}, "emails": {"e1": {"address": "alice@example.com"}}}]})
        };
        ResponseTemplate::new(200).set_body_json(json!({"methodResponses": [[call[0], result, "0"]]}))
    }).mount(&server).await;
    let backend = ContactsClient::for_request(Arc::new(Mutex::new(client(&server))), None, None)
        .await
        .unwrap();
    assert!(matches!(backend, ContactsClient::Jmap(_)));
    let contacts = backend.search(Some("Alice")).await.unwrap();
    assert_eq!(contacts.len(), 2);
    assert_eq!(contacts[1].id, "c2");
    assert_eq!(contacts[0].emails[0].email, "alice@example.com");
}

#[tokio::test]
async fn missing_contacts_capability_retains_carddav_fallback() {
    let server = MockServer::start().await;
    let client = Arc::new(Mutex::new(JmapClient::with_test_session(&server.uri())));
    let backend =
        ContactsClient::for_request(client.clone(), Some("user".into()), Some("password".into()))
            .await
            .unwrap();
    assert!(matches!(backend, ContactsClient::Carddav(_)));
    assert!(
        ContactsClient::for_request(client, None, None)
            .await
            .is_err()
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn create_uses_a_writable_default_book_and_valid_jscontact_fields() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(|req: &wiremock::Request| {
            let request: Value = serde_json::from_slice(&req.body).unwrap();
            let call = &request["methodCalls"][0];
            let result = match call[0].as_str().unwrap() {
                "AddressBook/get" => json!({"list": [
                    {"id": "readonly", "isDefault": false, "myRights": {"mayWrite": false}},
                    {"id": "default", "isDefault": true, "myRights": {"mayWrite": true}}
                ]}),
                "ContactCard/set" => {
                    let card = &call[1]["create"]["new"];
                    assert_eq!(card["@type"], "Card");
                    assert_eq!(card["version"], "1.0");
                    assert!(uuid::Uuid::parse_str(card["uid"].as_str().unwrap()).is_ok());
                    assert_eq!(card["addressBookIds"], json!({"default": true}));
                    assert_eq!(card["name"], json!({"full": "Alice"}));
                    assert_eq!(card["emails"]["e0"]["address"], "alice@example.com");
                    assert_eq!(card["titles"]["title"]["kind"], "title");
                    json!({"created": {"new": {"id": "c1"}}})
                }
                "ContactCard/get" => json!({"list": [{"id": "c1", "name": {"full": "Alice"}}]}),
                _ => panic!("Unexpected method"),
            };
            ResponseTemplate::new(200)
                .set_body_json(json!({"methodResponses": [[call[0], result, "0"]]}))
        })
        .mount(&server)
        .await;
    let contact = client(&server)
        .create_jmap_contact(&ContactFields {
            name: Some("Alice"),
            emails: Some(&[ContactEmail {
                email: "alice@example.com".into(),
                label: None,
            }]),
            title: Some("Engineer"),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(contact.id, "c1");
}

#[tokio::test]
async fn update_preserves_structured_names_and_unmentioned_fields_and_can_clear_emails() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).respond_with(|req: &wiremock::Request| {
        let request: Value = serde_json::from_slice(&req.body).unwrap();
        let call = &request["methodCalls"][0];
        let result = if call[0] == "ContactCard/get" {
            json!({"list": [{"id": "c1", "name": {"full": "Alice", "components": [{"kind": "given", "value": "Alice"}]}}]})
        } else {
            assert_eq!(call[1]["update"]["c1"], json!({"name/full": "New name", "emails": {}}));
            json!({"updated": {"c1": null}})
        };
        ResponseTemplate::new(200).set_body_json(json!({"methodResponses": [[call[0], result, "0"]]}))
    }).mount(&server).await;
    client(&server)
        .update_jmap_contact(
            "c1",
            &ContactFields {
                name: Some("New name"),
                emails: Some(&[]),
                ..Default::default()
            },
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn jmap_contact_object_errors_do_not_trigger_dav_fallback() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"methodResponses": [["ContactCard/set", {"notDestroyed": {"c1": {"type": "forbidden"}}}, "0"]]}))).expect(1).mount(&server).await;
    let backend = ContactsClient::for_request(
        Arc::new(Mutex::new(client(&server))),
        Some("user".into()),
        Some("password".into()),
    )
    .await
    .unwrap();
    assert!(
        backend
            .delete("c1")
            .await
            .unwrap_err()
            .to_string()
            .contains("forbidden")
    );
}

#[tokio::test]
async fn graphql_contacts_and_star_work_with_only_jmap_credentials() {
    use crate::mcp::graphql::{CardDavCreds, build_schema, request};
    let server = MockServer::start().await;
    Mock::given(method("POST")).respond_with(|req: &wiremock::Request| {
        let request: Value = serde_json::from_slice(&req.body).unwrap();
        let call = &request["methodCalls"][0];
        let result = match call[0].as_str().unwrap() {
            "ContactCard/query" if call[1]["position"] == 0 => json!({"ids": ["c1"]}),
            "ContactCard/query" => json!({"ids": []}),
            "ContactCard/get" => json!({"list": [{"id": "c1", "name": {"full": "Alice"}}]}),
            "AddressBook/get" => json!({"list": [{"id": "book", "isDefault": true, "myRights": {"mayWrite": true}}]}),
            "ContactCard/set" if call[1]["create"].is_object() => json!({"created": {"new": {"id": "c1"}}}),
            "ContactCard/set" => json!({"updated": {"c1": null}, "destroyed": ["c1"]}),
            "Email/set" => {
                assert_eq!(call[1]["update"]["e1"], json!({"keywords/$flagged": true}));
                json!({"updated": {"e1": null}})
            }
            _ => panic!("Unexpected method"),
        };
        ResponseTemplate::new(200).set_body_json(json!({"methodResponses": [[call[0], result, "0"]]}))
    }).mount(&server).await;
    let client = Arc::new(Mutex::new(client(&server)));
    let schema = build_schema();
    for query in [
        "{ contacts(query: \"Alice\") { nodes { id name } } }",
        "mutation { createContact(name: \"Alice\") { id name } }",
        "mutation { updateContact(id: \"c1\", name: \"New name\") { id } }",
        "mutation { deleteContact(id: \"c1\") { success } }",
        "mutation { starEmail(emailId: \"e1\", starred: true) { success } }",
    ] {
        let response = schema
            .execute(request(query, client.clone(), CardDavCreds::default()))
            .await;
        assert!(response.errors.is_empty(), "{query}: {:?}", response.errors);
    }
}

#[tokio::test]
async fn native_contacts_support_addressbooks_groups_copy_and_sync() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).respond_with(|req: &wiremock::Request| {
        let request: Value = serde_json::from_slice(&req.body).unwrap();
        let call = &request["methodCalls"][0];
        assert_eq!(call[1]["accountId"], "contacts-account");
        let result = match call[0].as_str().unwrap() {
            "AddressBook/get" => { assert_eq!(call[1]["ids"], json!([])); json!({"state": "books-state", "list": []}) }
            "AddressBook/set" => {
                assert_eq!(call[1]["onSuccessSetIsDefault"], "#new");
                assert_eq!(call[1]["create"]["new"]["name"], "Team");
                json!({"created": {"new": {"id": "book"}}})
            }
            "ContactCard/set" => {
                if call[1]["create"].is_object() {
                    let card = &call[1]["create"]["new"];
                    assert_eq!(card["kind"], "group");
                    assert_eq!(card["members"], json!({"person-uid": true}));
                    assert_eq!(card["addressBookIds"], json!({"book": true}));
                } else { assert_eq!(call[1]["update"]["c1"], json!({"anniversaries/birthday": {"kind": "birth", "date": {"@type": "PartialDate", "year": 1990, "month": 3, "day": 15}}})); }
                json!({"created": {"new": {"id": "group"}}, "updated": {"c1": null}})
            }
            "ContactCard/copy" => {
                assert_eq!(call[1]["fromAccountId"], "shared");
                assert_eq!(call[1]["onSuccessDestroyOriginal"], false);
                assert_eq!(call[1]["create"]["copy"]["id"], "c1");
                json!({"created": {"copy": {"id": "c2"}}})
            }
            "ContactCard/changes" | "AddressBook/changes" => {
                assert_eq!(call[1]["sinceState"], "old");
                json!({"newState": "next", "hasMoreChanges": true, "created": ["c1"], "updated": [], "destroyed": []})
            }
            "ContactCard/queryChanges" => { assert_eq!(call[1]["sinceQueryState"], "query-old"); json!({"newQueryState": "query-next", "added": [], "removed": []}) }
            _ => panic!("Unexpected method: {}", call[0]),
        };
        ResponseTemplate::new(200).set_body_json(json!({"methodResponses": [[call[0], result, "0"]]}))
    }).mount(&server).await;
    let client = client(&server);
    assert_eq!(
        client.contacts_get(true, Some(vec![])).await.unwrap()["state"],
        "books-state"
    );
    client.contacts_set(true, json!({"accountId": "must-not-override", "create": {"new": {"name": "Team"}}, "onSuccessSetIsDefault": "#new"})).await.unwrap();
    client
        .create_contact_card(
            json!({"kind": "group", "members": {"person-uid": true}}),
            Some("book"),
        )
        .await
        .unwrap();
    client.contacts_set(false, json!({"update": {"c1": {"anniversaries/birthday": {"kind": "birth", "date": {"@type": "PartialDate", "year": 1990, "month": 3, "day": 15}}}}})).await.unwrap();
    client
        .copy_contacts(
            "shared",
            json!({"copy": {"id": "c1", "addressBookIds": {"book": true}}}),
            false,
        )
        .await
        .unwrap();
    for books in [true, false] {
        assert_eq!(
            client
                .contacts_changes(books, "old", Some(1))
                .await
                .unwrap()["hasMoreChanges"],
            true
        );
    }
    client
        .contacts_query_changes(
            json!({"sinceQueryState": "query-old", "filter": {"kind": "group"}}),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn photos_upload_to_contacts_account_and_preserve_other_media() {
    use wiremock::matchers::{header, path};
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/upload/contacts-account"))
        .and(header("Content-Type", "image/png"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"blobId": "photo-blob"})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST")).and(path("/")).respond_with(|req: &wiremock::Request| {
        let request: Value = serde_json::from_slice(&req.body).unwrap();
        let call = &request["methodCalls"][0];
        let result = if call[0] == "ContactCard/get" {
            json!({"list": [{"id": "c1", "media": {"old": {"kind": "photo", "blobId": "old"}, "sound": {"kind": "sound", "uri": "https://example.com/name.mp3"}}}]})
        } else {
            let media = call[1]["update"]["c1"]["media"].as_object().unwrap();
            assert_eq!(media.len(), 2);
            assert!(media.contains_key("sound"));
            assert!(!media.contains_key("old"));
            assert!(media.values().any(|v| v["kind"] == "photo" && v["blobId"] == "photo-blob"));
            json!({"updated": {"c1": null}})
        };
        ResponseTemplate::new(200).set_body_json(json!({"methodResponses": [[call[0], result, "0"]]}))
    }).mount(&server).await;
    let mut client = client(&server);
    client.session.as_mut().unwrap().upload_url = format!("{}/upload/{{accountId}}", server.uri());
    client
        .set_contact_photo("c1", vec![1, 2, 3], "image/png")
        .await
        .unwrap();
}

#[tokio::test]
async fn copy_reports_source_deletion_failure_after_a_successful_copy() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"methodResponses": [
                ["ContactCard/copy", {"created": {"new": {"id": "copied"}}}, "0"],
                ["ContactCard/set", {"notDestroyed": {"original": {"type": "forbidden"}}}, "0"]
            ]})),
        )
        .mount(&server)
        .await;
    let error = client(&server)
        .copy_contacts("source", json!({"new": {"id": "original"}}), true)
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("Primary operation completed"));
    assert!(error.contains("forbidden"));
}
