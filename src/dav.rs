//! CalDAV and WebDAV access using Fastmail app-password credentials.
use anyhow::{Context, Result, bail, ensure};
use reqwest::{Client, Method, Url};
use serde::Serialize;
use std::time::Duration;

const DAV: &str = "DAV:";
const CAL: &str = "urn:ietf:params:xml:ns:caldav";

pub struct DavClient {
    client: Client,
    base: Url,
    username: String,
    password: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DavResource {
    pub href: String,
    pub name: String,
    pub collection: bool,
    pub calendar: bool,
    pub etag: Option<String>,
    pub content_type: Option<String>,
    pub size: Option<u64>,
    pub calendar_data: Option<String>,
}

impl DavClient {
    pub fn new(base: &str, username: String, password: String) -> Result<Self> {
        Ok(Self {
            client: Client::builder()
                .timeout(Duration::from_secs(60))
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
            base: Url::parse(base)?,
            username,
            password,
        })
    }

    // Returned hrefs may be absolute, but credentials must stay on this origin.
    fn url(&self, href: &str) -> Result<Url> {
        let url = self.base.join(href)?;
        ensure!(
            url.origin() == self.base.origin(),
            "DAV path must use the service's own origin"
        );
        ensure!(
            url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none(),
            "Invalid DAV resource URL"
        );
        Ok(url)
    }

    /// Encode a user-supplied file path; unlike hrefs, it is not already URL encoded.
    pub fn file_path(path: &str) -> Result<String> {
        ensure!(
            !path.split('/').any(|p| p == "." || p == ".."),
            "File paths cannot contain . or .. segments"
        );
        let mut url = Url::parse("https://myfiles.fastmail.com/")?;
        url.path_segments_mut()
            .unwrap()
            .clear()
            .extend(path.trim_start_matches('/').split('/'));
        Ok(url.path().to_string())
    }

    async fn request(
        &self,
        method: &str,
        href: &str,
        body: Vec<u8>,
        content_type: &str,
        depth: Option<&str>,
        condition: Option<(&str, &str)>,
    ) -> Result<reqwest::Response> {
        let mut request = self
            .client
            .request(Method::from_bytes(method.as_bytes())?, self.url(href)?)
            .basic_auth(&self.username, Some(&self.password))
            .header("Content-Type", content_type)
            .body(body);
        if let Some(depth) = depth {
            request = request.header("Depth", depth);
        }
        if let Some((header, value)) = condition {
            request = request.header(header, value);
        }
        let response = request.send().await?;
        ensure!(
            response.status().is_success(),
            "DAV {method} failed: {}",
            response.status()
        );
        Ok(response)
    }

    pub async fn list(&self, href: &str) -> Result<Vec<DavResource>> {
        let body = br#"<d:propfind xmlns:d="DAV:"><d:prop><d:displayname/><d:resourcetype/><d:getetag/><d:getcontenttype/><d:getcontentlength/></d:prop></d:propfind>"#;
        let text = self
            .request(
                "PROPFIND",
                href,
                body.to_vec(),
                "application/xml; charset=utf-8",
                Some("1"),
                None,
            )
            .await?
            .text()
            .await?;
        parse_resources(&text)
    }

    pub async fn calendars(&self) -> Result<Vec<DavResource>> {
        let mut url = self.base.clone();
        url.path_segments_mut()
            .unwrap()
            .extend(["dav", "calendars", "user", &self.username, ""]);
        Ok(self
            .list(url.path())
            .await?
            .into_iter()
            .filter(|r| r.calendar)
            .collect())
    }

    pub async fn events(
        &self,
        calendar: &str,
        after: Option<&str>,
        before: Option<&str>,
    ) -> Result<Vec<DavResource>> {
        let mut range = String::new();
        for (key, value) in [("start", after), ("end", before)] {
            if let Some(value) = value {
                ensure!(
                    valid_timestamp(value),
                    "Use UTC timestamps like 20261016T090000Z"
                );
                range.push_str(&format!(" {key}=\"{value}\""));
            }
        }
        if let (Some(after), Some(before)) = (after, before) {
            ensure!(after < before, "--after must precede --before");
        }
        let range = if range.is_empty() {
            String::new()
        } else {
            format!("<c:time-range{range}/>")
        };
        let body = format!(
            r#"<c:calendar-query xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:prop><d:getetag/><c:calendar-data/></d:prop><c:filter><c:comp-filter name="VCALENDAR"><c:comp-filter name="VEVENT">{range}</c:comp-filter></c:comp-filter></c:filter></c:calendar-query>"#
        );
        let text = self
            .request(
                "REPORT",
                calendar,
                body.into_bytes(),
                "application/xml; charset=utf-8",
                Some("1"),
                None,
            )
            .await?
            .text()
            .await?;
        parse_resources(&text)
    }

    pub async fn get(&self, href: &str) -> Result<(Vec<u8>, Option<String>)> {
        let response = self
            .request(
                "GET",
                href,
                Vec::new(),
                "application/octet-stream",
                None,
                None,
            )
            .await?;
        let etag = response
            .headers()
            .get("etag")
            .and_then(|h| h.to_str().ok())
            .map(str::to_owned);
        Ok((response.bytes().await?.to_vec(), etag))
    }

    pub async fn put(
        &self,
        href: &str,
        data: Vec<u8>,
        content_type: &str,
        etag: Option<&str>,
        overwrite: bool,
    ) -> Result<Option<String>> {
        let condition = etag
            .map(|e| ("If-Match", e))
            .or_else(|| (!overwrite).then_some(("If-None-Match", "*")));
        let response = self
            .request("PUT", href, data, content_type, None, condition)
            .await?;
        Ok(response
            .headers()
            .get("etag")
            .and_then(|h| h.to_str().ok())
            .map(str::to_owned))
    }

    pub async fn mkdir(&self, href: &str) -> Result<()> {
        self.request(
            "MKCOL",
            href,
            Vec::new(),
            "application/octet-stream",
            None,
            None,
        )
        .await?;
        Ok(())
    }

    pub async fn delete(&self, href: &str, etag: Option<&str>) -> Result<()> {
        ensure!(
            self.url(href)?.path() != "/",
            "Cannot delete the service root"
        );
        let response = self
            .request(
                "DELETE",
                href,
                Vec::new(),
                "application/octet-stream",
                None,
                etag.map(|e| ("If-Match", e)),
            )
            .await?;
        // A collection DELETE can succeed for some children and fail for others.
        // HTTP 207 alone is not confirmation that the folder was removed.
        if response.status().as_u16() == 207 {
            let text = response.text().await?;
            let doc = roxmltree::Document::parse(&text).context("Invalid DAV deletion response")?;
            let statuses: Vec<_> = doc
                .descendants()
                .filter(|n| n.has_tag_name((DAV, "status")))
                .filter_map(|n| n.text())
                .collect();
            ensure!(
                !statuses.is_empty(),
                "DAV deletion returned no resource statuses"
            );
            for status in statuses {
                let code = status
                    .split_whitespace()
                    .nth(1)
                    .and_then(|c| c.parse::<u16>().ok());
                ensure!(
                    code.is_some_and(|c| (200..300).contains(&c)),
                    "DAV deletion was incomplete: {status}"
                );
            }
        }
        Ok(())
    }
}

fn valid_timestamp(value: &str) -> bool {
    value.len() == 16
        && value.as_bytes()[8] == b'T'
        && value.ends_with('Z')
        && value
            .bytes()
            .enumerate()
            .all(|(i, c)| i == 8 || i == 15 || c.is_ascii_digit())
}

fn parse_resources(xml: &str) -> Result<Vec<DavResource>> {
    let doc = roxmltree::Document::parse(xml).context("Invalid DAV XML response")?;
    ensure!(
        doc.root_element().has_tag_name((DAV, "multistatus")),
        "Expected a DAV multistatus response"
    );
    let mut resources = Vec::new();
    for response in doc
        .root_element()
        .children()
        .filter(|n| n.has_tag_name((DAV, "response")))
    {
        let href = response
            .children()
            .find(|n| n.has_tag_name((DAV, "href")))
            .and_then(|n| n.text())
            .context("DAV response is missing href")?;
        let mut resource = DavResource {
            href: href.to_owned(),
            name: String::new(),
            collection: false,
            calendar: false,
            etag: None,
            content_type: None,
            size: None,
            calendar_data: None,
        };
        let mut found = false;
        for propstat in response
            .children()
            .filter(|n| n.has_tag_name((DAV, "propstat")))
        {
            let status = propstat
                .children()
                .find(|n| n.has_tag_name((DAV, "status")))
                .and_then(|n| n.text())
                .unwrap_or("");
            if status.split_whitespace().nth(1) != Some("200") {
                continue;
            }
            found = true;
            let Some(prop) = propstat.children().find(|n| n.has_tag_name((DAV, "prop"))) else {
                continue;
            };
            for node in prop.children().filter(|n| n.is_element()) {
                let text = node.text().map(str::to_owned);
                match (node.tag_name().namespace(), node.tag_name().name()) {
                    (Some(DAV), "displayname") => resource.name = text.unwrap_or_default(),
                    (Some(DAV), "getetag") => resource.etag = text,
                    (Some(DAV), "getcontenttype") => resource.content_type = text,
                    (Some(DAV), "getcontentlength") => {
                        resource.size = text.and_then(|t| t.parse().ok())
                    }
                    (Some(DAV), "resourcetype") => {
                        resource.collection =
                            node.children().any(|n| n.has_tag_name((DAV, "collection")));
                        resource.calendar =
                            node.children().any(|n| n.has_tag_name((CAL, "calendar")));
                    }
                    (Some(CAL), "calendar-data") => resource.calendar_data = text,
                    _ => {}
                }
            }
        }
        if found {
            resources.push(resource);
        } else if response.children().any(|n| n.has_tag_name((DAV, "status"))) {
            bail!("DAV resource {href} could not be retrieved");
        }
    }
    Ok(resources)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_string_contains, header, method, path},
    };

    #[test]
    fn paths_encode_literal_names_and_keep_credentials_on_origin() {
        assert_eq!(
            DavClient::file_path("/work/a #?%.txt").unwrap(),
            "/work/a%20%23%3F%25.txt"
        );
        assert!(DavClient::file_path("/work/../private").is_err());
        let client =
            DavClient::new("https://myfiles.fastmail.com/", "u".into(), "p".into()).unwrap();
        assert!(client.url("https://example.org/a").is_err());
        assert!(client.url("//example.org/a").is_err());
        assert!(client.url("/a?delete=1").is_err());
        assert!(client.url("/a%20b").is_ok());
    }

    #[test]
    fn parses_only_successful_properties_and_respects_namespaces() {
        let xml = r#"<multistatus xmlns="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><response><href>/cal/</href><propstat><prop><displayname>Work &amp; Home</displayname><resourcetype><collection/><c:calendar/></resourcetype><getetag>"v1"</getetag><c:calendar-data>BEGIN:VCALENDAR</c:calendar-data></prop><status>HTTP/1.1 200 OK</status></propstat><propstat><prop><getcontentlength>999</getcontentlength><getetag>bad</getetag></prop><status>HTTP/1.1 404 Not Found</status></propstat></response></multistatus>"#;
        let resources = parse_resources(xml).unwrap();
        assert_eq!(resources.len(), 1);
        assert_eq!(resources[0].name, "Work & Home");
        assert_eq!(resources[0].etag.as_deref(), Some("\"v1\""));
        assert_eq!(resources[0].size, None);
        assert!(resources[0].calendar && resources[0].collection);
        assert_eq!(
            resources[0].calendar_data.as_deref(),
            Some("BEGIN:VCALENDAR")
        );
        assert!(parse_resources("<html/>").is_err());
    }

    #[tokio::test]
    async fn calendar_discovery_and_range_report_use_dav_protocol() {
        let server = MockServer::start().await;
        let client = DavClient::new(
            &format!("{}/", server.uri()),
            "me@example.com".into(),
            "secret".into(),
        )
        .unwrap();
        Mock::given(method("PROPFIND"))
            .and(path("/dav/calendars/user/me@example.com/"))
            .and(header("Depth", "1"))
            .and(header("Authorization", "Basic bWVAZXhhbXBsZS5jb206c2VjcmV0"))
            .respond_with(ResponseTemplate::new(207).set_body_string(r#"<d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:response><d:href>/cal/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/><c:calendar/></d:resourcetype></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>"#)).expect(1).mount(&server).await;
        assert_eq!(client.calendars().await.unwrap()[0].href, "/cal/");
        Mock::given(method("REPORT"))
            .and(path("/cal/"))
            .and(body_string_contains(
                "<c:time-range start=\"20261016T000000Z\" end=\"20261017T000000Z\"/>",
            ))
            .respond_with(
                ResponseTemplate::new(207).set_body_string("<multistatus xmlns=\"DAV:\"/>"),
            )
            .expect(1)
            .mount(&server)
            .await;
        assert!(
            client
                .events("/cal/", Some("20261016T000000Z"), Some("20261017T000000Z"))
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            client
                .events("/cal/", Some("\"/><injection/>"), None)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn writes_are_conditional_and_errors_are_not_successes() {
        let server = MockServer::start().await;
        let client = DavClient::new(&server.uri(), "u".into(), "p".into()).unwrap();
        Mock::given(method("PUT"))
            .and(path("/new"))
            .and(header("If-None-Match", "*"))
            .respond_with(ResponseTemplate::new(201).insert_header("ETag", "\"new\""))
            .expect(1)
            .mount(&server)
            .await;
        assert_eq!(
            client
                .put("/new", vec![1], "application/octet-stream", None, false)
                .await
                .unwrap()
                .as_deref(),
            Some("\"new\"")
        );
        Mock::given(method("PUT"))
            .and(path("/old"))
            .and(header("If-Match", "\"stale\""))
            .respond_with(ResponseTemplate::new(412))
            .expect(1)
            .mount(&server)
            .await;
        assert!(
            client
                .put("/old", vec![], "text/calendar", Some("\"stale\""), false)
                .await
                .unwrap_err()
                .to_string()
                .contains("412")
        );
        Mock::given(method("GET"))
            .and(path("/redirect"))
            .respond_with(ResponseTemplate::new(302).insert_header("Location", "/new"))
            .expect(1)
            .mount(&server)
            .await;
        assert!(client.get("/redirect").await.is_err());
        assert!(client.delete("/", None).await.is_err());
        Mock::given(method("DELETE")).and(path("/folder/"))
            .respond_with(ResponseTemplate::new(207).set_body_string(r#"<multistatus xmlns="DAV:"><response><href>/folder/locked</href><status>HTTP/1.1 423 Locked</status></response></multistatus>"#))
            .expect(1).mount(&server).await;
        assert!(
            client
                .delete("/folder/", None)
                .await
                .unwrap_err()
                .to_string()
                .contains("incomplete")
        );
    }
}
