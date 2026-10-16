use super::*;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

#[tokio::test]
async fn bulk_patches_escape_paths_batch_to_limits_and_preserve_partial_errors() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).respond_with(|request: &wiremock::Request| {
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        let args = &body["methodCalls"][0][1];
        let update = args["update"].as_object().unwrap();
        assert_eq!(update.len(), 1);
        let (id, patch) = update.iter().next().unwrap();
        assert_eq!(patch, &json!({"keywords/project~1x~0y": true, "keywords/$answered": null, "mailboxIds/shared~1folder": true, "mailboxIds/old": null}));
        let result = if id == "bad" { json!({"notUpdated": {"bad": {"type": "forbidden"}}}) } else { json!({"updated": {id: null}}) };
        ResponseTemplate::new(200).set_body_json(json!({"methodResponses": [["Email/set", result, "0"]]}))
    }).expect(3).mount(&server).await;
    let mut client = JmapClient::with_test_session(&server.uri());
    client.session.as_mut().unwrap().capabilities.insert(
        "urn:ietf:params:jmap:core".into(),
        json!({"maxObjectsInSet": 1}),
    );
    let keywords = HashMap::from([("project/x~y".into(), true), ("$answered".into(), false)]);
    let result = client
        .patch_emails(
            &["a".into(), "bad".into(), "c".into()],
            &keywords,
            &["shared/folder".into()],
            &["old".into()],
            None,
        )
        .await
        .unwrap();
    assert_eq!(result["updated"].as_object().unwrap().len(), 2);
    assert_eq!(result["notUpdated"]["bad"]["type"], "forbidden");
}

#[tokio::test]
async fn selected_accounts_are_checked_and_apply_to_every_capability() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(|request: &wiremock::Request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            let call = &body["methodCalls"][0];
            assert_eq!(call[1]["accountId"], "shared");
            ResponseTemplate::new(200).set_body_json(
                json!({"methodResponses": [[call[0], {"list": [], "state": "s1"}, "0"]]}),
            )
        })
        .mount(&server)
        .await;
    let mut client = JmapClient::with_test_session(&server.uri());
    assert!(client.select_account(Some("missing".into())).is_err());
    client.session.as_mut().unwrap().accounts.insert(
        "shared".into(),
        Account {
            name: "Shared".into(),
            is_personal: false,
            is_read_only: false,
            account_capabilities: HashMap::from([
                ("urn:ietf:params:jmap:mail".into(), json!({})),
                ("urn:ietf:params:jmap:contacts".into(), json!({})),
            ]),
        },
    );
    client.select_account(Some("shared".into())).unwrap();
    client.records(DataType::Email, Some(&[])).await.unwrap();
    client.records(DataType::Contact, Some(&[])).await.unwrap();
    assert!(
        client
            .records(DataType::Submission, Some(&[]))
            .await
            .is_err()
    );
    client.select_account(None).unwrap();
    assert_eq!(client.account_id().unwrap(), "acct1");
}

#[tokio::test]
async fn changes_keep_updates_and_deletions_and_cancellation_uses_undo_status() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).respond_with(|request: &wiremock::Request| {
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        let call = &body["methodCalls"][0];
        let result = if call[0] == "Email/changes" {
            assert_eq!(call[1]["sinceState"], "old");
            json!({"newState": "new", "created": ["c"], "updated": ["u"], "destroyed": ["d"], "hasMoreChanges": true})
        } else {
            assert_eq!(call[0], "EmailSubmission/set");
            assert_eq!(call[1]["update"], json!({"s1": {"undoStatus": "canceled"}}));
            json!({"updated": {"s1": null}})
        };
        ResponseTemplate::new(200).set_body_json(json!({"methodResponses": [[call[0], result, "0"]]}))
    }).mount(&server).await;
    let mut client = JmapClient::with_test_session(&server.uri());
    client
        .session
        .as_mut()
        .unwrap()
        .primary_accounts
        .insert(DataType::Submission.capability().into(), "acct1".into());
    let delta = client
        .changes(DataType::Email, "old", Some(10))
        .await
        .unwrap();
    assert_eq!(delta["updated"], json!(["u"]));
    assert_eq!(delta["destroyed"], json!(["d"]));
    client.cancel_submission(&["s1".into()]).await.unwrap();
}

#[tokio::test]
async fn parsing_uploads_bytes_and_push_methods_are_not_account_scoped() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/upload"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"blobId": "raw"})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/"))
        .respond_with(|request: &wiremock::Request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            let call = &body["methodCalls"][0];
            let result = if call[0] == "Email/parse" {
                assert_eq!(call[1]["blobIds"], json!(["raw"]));
                json!({"parsed": {"raw": {"subject": "Parsed"}}})
            } else {
                assert!(call[1]["accountId"].is_null());
                json!({"list": []})
            };
            ResponseTemplate::new(200)
                .set_body_json(json!({"methodResponses": [[call[0], result, "0"]]}))
        })
        .mount(&server)
        .await;
    let client = JmapClient::with_test_session(&server.uri());
    assert_eq!(
        client
            .parse_email(b"Subject: Parsed\r\n\r\nbody".to_vec())
            .await
            .unwrap()["parsed"]["raw"]["subject"],
        "Parsed"
    );
    client.push_subscriptions().await.unwrap();
    client
        .set_push_subscriptions(json!({"update": {"p1": {"verificationCode": "code"}}}))
        .await
        .unwrap();
}

#[tokio::test]
async fn emptying_trash_preserves_other_memberships_and_checks_the_state() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).respond_with(|request: &wiremock::Request| {
        let body: Value = serde_json::from_slice(&request.body).unwrap(); let call = &body["methodCalls"][0];
        let result = match call[0].as_str().unwrap() {
            "Mailbox/get" => json!({"list":[{"id":"trash", "name":"Trash", "role":"trash"}]}),
            "Email/query" if call[1]["position"] == 0 => json!({"ids":["shared","only-trash"]}),
            "Email/query" => json!({"ids":[]}),
            "Email/get" => json!({"state":"before", "list":[{"id":"shared","mailboxIds":{"trash":true,"archive":true}},{"id":"only-trash","mailboxIds":{"trash":true}}]}),
            "Email/set" => {
                assert_eq!(call[1]["ifInState"], "before");
                assert_eq!(call[1]["update"], json!({"shared":{"mailboxIds/trash":null}}));
                assert_eq!(call[1]["destroy"], json!(["only-trash"]));
                json!({"updated":{"shared":null},"destroyed":["only-trash"],"newState":"after"})
            }
            _ => panic!("Unexpected call {call}"),
        };
        ResponseTemplate::new(200).set_body_json(json!({"methodResponses":[[call[0],result,"0"]]}))
    }).mount(&server).await;
    let result = JmapClient::with_test_session(&server.uri())
        .empty_trash()
        .await
        .unwrap();
    assert_eq!(result["destroyed"], json!(["only-trash"]));
    assert!(result["updated"].get("shared").is_some());
}

#[tokio::test]
async fn invalid_api_token_is_reported_without_retrying() {
    use wiremock::matchers::header;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(header("Authorization", "Bearer test-token"))
        .respond_with(ResponseTemplate::new(401))
        .expect(1)
        .mount(&server)
        .await;
    let error = JmapClient::with_test_session(&server.uri())
        .records(DataType::Email, Some(&[]))
        .await
        .unwrap_err();
    assert!(matches!(error, Error::InvalidToken(_)));
}

#[tokio::test]
async fn snapshot_pagination_anchors_survive_changes_before_the_page() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(|request: &wiremock::Request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            let call = &body["methodCalls"][0];
            let result = if call[1]["anchor"].is_null() {
                json!({"ids":["a","b"],"queryState":"old"})
            } else if call[1]["anchor"] == "b" {
                assert_eq!(call[1]["anchorOffset"], 1);
                json!({"ids":["c"],"queryState":"changed"})
            } else {
                assert_eq!(call[1]["anchor"], "c");
                json!({"ids":[],"queryState":"changed"})
            };
            ResponseTemplate::new(200)
                .set_body_json(json!({"methodResponses":[[call[0],result,"0"]]}))
        })
        .expect(3)
        .mount(&server)
        .await;
    let ids = JmapClient::with_test_session(&server.uri())
        .all_query_ids(DataType::Email, json!({}))
        .await
        .unwrap();
    assert_eq!(ids, vec!["a", "b", "c"]);
}
