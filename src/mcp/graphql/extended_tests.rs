use super::*;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::sync::Mutex;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{header, method, path},
};

#[tokio::test]
async fn typed_bulk_and_management_results_expose_fields_and_partial_errors() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).respond_with(|request: &wiremock::Request| {
        let body: Value = serde_json::from_slice(&request.body).unwrap(); let call = &body["methodCalls"][0];
        let value = match call[0].as_str().unwrap() {
            "Email/set" => json!({"updated":{"e1":null},"notUpdated":{"bad":{"type":"forbidden","description":"No rights"}}}),
            "Mailbox/set" => json!({"created":{"new":{"id":"new-folder"}}}),
            "SearchSnippet/get" => json!({"list":[{"emailId":"e1","subject":"Match","preview":"A <mark>match</mark>"}],"notFound":["gone"]}),
            _ => panic!("Unexpected call {call}"),
        };
        ResponseTemplate::new(200).set_body_json(json!({"methodResponses":[[call[0],value,"0"]]}))
    }).mount(&server).await;
    let client = Arc::new(Mutex::new(crate::jmap::JmapClient::with_test_session(
        &server.uri(),
    )));
    let schema = build_schema();
    let response = schema.execute(request("mutation { patchEmails(ids:[\"e1\",\"bad\"],keywords:{project:true}) { success updated notUpdated { id kind description } } createMailbox(name:\"Work\") { id } }", client.clone(), CardDavCreds::default())).await;
    assert!(response.errors.is_empty(), "{:?}", response.errors);
    let data = response.data.into_json().unwrap();
    assert_eq!(data["patchEmails"]["success"], false);
    assert_eq!(data["patchEmails"]["notUpdated"][0]["id"], "bad");
    assert_eq!(data["createMailbox"]["id"], "new-folder");
    let response = schema.execute(request("{ searchSnippets(text:\"match\",emailIds:[\"e1\",\"gone\"]) { snippets { emailId preview } notFound } }", client, CardDavCreds::default())).await;
    assert!(response.errors.is_empty(), "{:?}", response.errors);
    assert_eq!(
        response.data.into_json().unwrap()["searchSnippets"]["notFound"],
        json!(["gone"])
    );
}

#[tokio::test]
async fn dav_graphql_uses_request_credentials_and_typed_results() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/a%20b.txt"))
        .and(header(
            "Authorization",
            "Basic cmVxdWVzdC11c2VyOnJlcXVlc3QtcGFzc3dvcmQ=",
        ))
        .and(header("If-None-Match", "*"))
        .respond_with(ResponseTemplate::new(201).insert_header("ETag", "\"v1\""))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/a%20b.txt"))
        .respond_with(ResponseTemplate::new(200).set_body_string("hello"))
        .expect(1)
        .mount(&server)
        .await;
    let client = Arc::new(Mutex::new(crate::jmap::JmapClient::with_test_session(
        &server.uri(),
    )));
    let schema = build_schema();
    let creds = CardDavCreds {
        username: Some("request-user".into()),
        app_password: Some("request-password".into()),
    };
    let req = request(
        "mutation { uploadFile(path:\"/a b.txt\",base64:\"aGVsbG8=\") { success etag href } }",
        client.clone(),
        creds.clone(),
    )
    .data(api::TestDavBase(server.uri()));
    let response = schema.execute(req).await;
    assert!(response.errors.is_empty(), "{:?}", response.errors);
    assert_eq!(
        response.data.into_json().unwrap()["uploadFile"]["etag"],
        "\"v1\""
    );
    let req = request(
        "{ fileDownload(path:\"/a b.txt\") { base64 size } }",
        client.clone(),
        creds,
    )
    .data(api::TestDavBase(server.uri()));
    let response = schema.execute(req).await;
    assert!(response.errors.is_empty(), "{:?}", response.errors);
    assert_eq!(
        response.data.into_json().unwrap()["fileDownload"]["size"],
        5
    );
    let response = schema
        .execute(request(
            "{ files { path } }",
            client,
            CardDavCreds::default(),
        ))
        .await;
    assert!(
        response
            .errors
            .iter()
            .any(|e| e.message.contains("username"))
    );
}
