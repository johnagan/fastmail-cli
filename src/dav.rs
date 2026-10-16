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

#[derive(Debug, Serialize, async_graphql::SimpleObject)]
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
        self.events_expanded(calendar, after, before, false).await
    }

    pub async fn events_expanded(
        &self,
        calendar: &str,
        after: Option<&str>,
        before: Option<&str>,
        expand: bool,
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
        let data = if expand {
            let (Some(start), Some(end)) = (after, before) else {
                bail!("Expansion requires --after and --before");
            };
            format!(r#"<c:calendar-data><c:expand start="{start}" end="{end}"/></c:calendar-data>"#)
        } else {
            "<c:calendar-data/>".into()
        };
        let body = format!(
            r#"<c:calendar-query xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:prop><d:getetag/>{data}</d:prop><c:filter><c:comp-filter name="VCALENDAR"><c:comp-filter name="VEVENT">{range}</c:comp-filter></c:comp-filter></c:filter></c:calendar-query>"#
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

    /// MOVE also handles renames; both resource URLs stay on this DAV origin.
    pub async fn transfer(
        &self,
        source: &str,
        destination: &str,
        copy: bool,
        overwrite: bool,
    ) -> Result<()> {
        let source = self.url(source)?;
        ensure!(source.path() != "/", "Cannot move or copy the service root");
        let destination = self.url(destination)?;
        ensure!(
            destination.path() != "/" && source != destination,
            "Choose a distinct non-root destination"
        );
        let method = if copy { "COPY" } else { "MOVE" };
        let response = self
            .client
            .request(Method::from_bytes(method.as_bytes())?, source)
            .basic_auth(&self.username, Some(&self.password))
            .header("Destination", destination.as_str())
            .header("Overwrite", if overwrite { "T" } else { "F" })
            .header("Depth", "infinity")
            .send()
            .await?;
        ensure!(
            response.status().is_success(),
            "DAV {method} failed: {}",
            response.status()
        );
        if response.status().as_u16() == 207 {
            check_multistatus(&response.text().await?)?;
        }
        Ok(())
    }

    pub fn calendar_home(&self) -> String {
        let mut url = self.base.clone();
        url.path_segments_mut()
            .unwrap()
            .extend(["dav", "calendars", "user", &self.username, ""]);
        url.path().into()
    }

    pub async fn create_calendar(
        &self,
        href: &str,
        name: &str,
        description: Option<&str>,
        color: Option<&str>,
    ) -> Result<()> {
        ensure!(
            href.ends_with('/'),
            "Calendar collection href must end with /"
        );
        let properties = calendar_properties(Some(name), description, color);
        let body = format!(
            r#"<c:mkcalendar xmlns:c="urn:ietf:params:xml:ns:caldav" xmlns:d="DAV:" xmlns:a="http://apple.com/ns/ical/"><d:set><d:prop>{properties}</d:prop></d:set></c:mkcalendar>"#
        );
        let response = self
            .request(
                "MKCALENDAR",
                href,
                body.into_bytes(),
                "application/xml; charset=utf-8",
                None,
                None,
            )
            .await?;
        if response.status().as_u16() == 207 {
            check_multistatus(&response.text().await?)?;
        }
        Ok(())
    }

    pub async fn update_calendar(
        &self,
        href: &str,
        name: Option<&str>,
        description: Option<&str>,
        color: Option<&str>,
    ) -> Result<()> {
        let properties = calendar_properties(name, description, color);
        ensure!(
            !properties.is_empty(),
            "Provide a calendar property to change"
        );
        let body = format!(
            r#"<d:propertyupdate xmlns:c="urn:ietf:params:xml:ns:caldav" xmlns:d="DAV:" xmlns:a="http://apple.com/ns/ical/"><d:set><d:prop>{properties}</d:prop></d:set></d:propertyupdate>"#
        );
        let response = self
            .request(
                "PROPPATCH",
                href,
                body.into_bytes(),
                "application/xml; charset=utf-8",
                None,
                None,
            )
            .await?;
        if response.status().as_u16() == 207 {
            check_multistatus(&response.text().await?)?;
        }
        Ok(())
    }

    pub async fn free_busy(&self, href: &str, start: &str, end: &str) -> Result<String> {
        ensure!(
            valid_timestamp(start) && valid_timestamp(end) && start < end,
            "Use an increasing UTC iCalendar time range"
        );
        let body = format!(
            r#"<c:free-busy-query xmlns:c="urn:ietf:params:xml:ns:caldav"><c:time-range start="{start}" end="{end}"/></c:free-busy-query>"#
        );
        self.request(
            "REPORT",
            href,
            body.into_bytes(),
            "application/xml; charset=utf-8",
            Some("1"),
            None,
        )
        .await?
        .text()
        .await
        .map_err(Into::into)
    }

    /// Discover scheduling endpoints from the current user's principal.
    pub async fn scheduling(&self) -> Result<serde_json::Value> {
        let body = br#"<d:propfind xmlns:d="DAV:"><d:prop><d:current-user-principal/></d:prop></d:propfind>"#;
        let text = self
            .request(
                "PROPFIND",
                "/",
                body.to_vec(),
                "application/xml",
                Some("0"),
                None,
            )
            .await?
            .text()
            .await?;
        let doc = roxmltree::Document::parse(&text)?;
        let principal = property_hrefs(&doc, DAV, "current-user-principal")
            .into_iter()
            .next()
            .context("Server returned no current-user-principal")?;
        let body = br#"<d:propfind xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:prop><c:schedule-inbox-URL/><c:schedule-outbox-URL/><c:calendar-user-address-set/><c:calendar-home-set/></d:prop></d:propfind>"#;
        let text = self
            .request(
                "PROPFIND",
                &principal,
                body.to_vec(),
                "application/xml",
                Some("0"),
                None,
            )
            .await?
            .text()
            .await?;
        let doc = roxmltree::Document::parse(&text)?;
        Ok(
            serde_json::json!({"principal": principal, "inbox": property_hrefs(&doc, CAL, "schedule-inbox-URL"), "outbox": property_hrefs(&doc, CAL, "schedule-outbox-URL"), "addresses": property_hrefs(&doc, CAL, "calendar-user-address-set"), "home": property_hrefs(&doc, CAL, "calendar-home-set")}),
        )
    }

    /// Send an iTIP REQUEST/REPLY/CANCEL through a CalDAV scheduling outbox.
    pub async fn schedule(
        &self,
        outbox: &str,
        originator: &str,
        recipients: &[String],
        data: Vec<u8>,
    ) -> Result<serde_json::Value> {
        ensure!(!recipients.is_empty(), "Provide at least one recipient");
        let mut request = self
            .client
            .post(self.url(outbox)?)
            .basic_auth(&self.username, Some(&self.password))
            .header("Content-Type", "text/calendar; charset=utf-8")
            .header("Originator", originator)
            .body(data);
        for recipient in recipients {
            request = request.header("Recipient", recipient);
        }
        let response = request.send().await?;
        ensure!(
            response.status().is_success(),
            "CalDAV scheduling failed: {}",
            response.status()
        );
        let text = response.text().await?;
        let doc = roxmltree::Document::parse(&text).context("Invalid scheduling response")?;
        ensure!(
            doc.root_element().has_tag_name((CAL, "schedule-response")),
            "Expected a CalDAV schedule-response"
        );
        let mut results = Vec::new();
        for response in doc
            .root_element()
            .children()
            .filter(|n| n.has_tag_name((CAL, "response")))
        {
            let field = |name| {
                response
                    .children()
                    .find(|n| n.has_tag_name((CAL, name)))
                    .and_then(|n| n.text())
                    .map(str::to_owned)
            };
            let recipient = response
                .descendants()
                .find(|n| n.has_tag_name((DAV, "href")))
                .and_then(|n| n.text());
            results.push(serde_json::json!({"recipient": recipient, "requestStatus": field("request-status"), "calendarData": field("calendar-data")}));
        }
        ensure!(
            !results.is_empty(),
            "Scheduling returned no recipient statuses"
        );
        Ok(serde_json::json!({"responses": results}))
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

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn calendar_properties(
    name: Option<&str>,
    description: Option<&str>,
    color: Option<&str>,
) -> String {
    [
        ("d:displayname", name),
        ("c:calendar-description", description),
        ("a:calendar-color", color),
    ]
    .into_iter()
    .filter_map(|(key, value)| value.map(|value| format!("<{key}>{}</{key}>", xml_escape(value))))
    .collect()
}

fn check_multistatus(xml: &str) -> Result<()> {
    let doc = roxmltree::Document::parse(xml)?;
    ensure!(
        doc.root_element().has_tag_name((DAV, "multistatus")),
        "Expected a DAV multistatus"
    );
    let statuses: Vec<_> = doc
        .descendants()
        .filter(|n| n.has_tag_name((DAV, "status")))
        .filter_map(|n| n.text())
        .collect();
    ensure!(
        !statuses.is_empty(),
        "DAV operation returned no resource statuses"
    );
    for status in statuses {
        ensure!(
            status
                .split_whitespace()
                .nth(1)
                .and_then(|s| s.parse::<u16>().ok())
                .is_some_and(|code| (200..300).contains(&code)),
            "DAV operation was incomplete: {status}"
        );
    }
    Ok(())
}

fn property_hrefs(doc: &roxmltree::Document<'_>, ns: &str, property: &str) -> Vec<String> {
    doc.descendants()
        .filter(|n| n.has_tag_name((ns, property)))
        .filter(|n| {
            n.ancestors()
                .find(|a| a.has_tag_name((DAV, "propstat")))
                .is_none_or(|propstat| {
                    propstat
                        .children()
                        .find(|n| n.has_tag_name((DAV, "status")))
                        .and_then(|n| n.text())
                        .is_some_and(|s| s.split_whitespace().nth(1) == Some("200"))
                })
        })
        .flat_map(|n| {
            n.children()
                .filter(|n| n.has_tag_name((DAV, "href")))
                .filter_map(|n| n.text().map(str::to_owned))
        })
        .collect()
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

    #[tokio::test]
    async fn transfers_are_server_side_and_partial_failures_are_errors() {
        let server = MockServer::start().await;
        let client = DavClient::new(&server.uri(), "u".into(), "p".into()).unwrap();
        Mock::given(method("MOVE"))
            .and(path("/old%20name"))
            .and(header(
                "Destination",
                format!("{}/new%20name", server.uri()).as_str(),
            ))
            .and(header("Overwrite", "F"))
            .respond_with(ResponseTemplate::new(201))
            .expect(1)
            .mount(&server)
            .await;
        client
            .transfer("/old%20name", "/new%20name", false, false)
            .await
            .unwrap();
        Mock::given(method("COPY")).and(header("Overwrite", "T"))
            .respond_with(ResponseTemplate::new(207).set_body_string(r#"<multistatus xmlns="DAV:"><response><href>/folder/locked</href><status>HTTP/1.1 423 Locked</status></response></multistatus>"#)).expect(1).mount(&server).await;
        assert!(
            client
                .transfer("/folder/", "/copy/", true, true)
                .await
                .unwrap_err()
                .to_string()
                .contains("incomplete")
        );
        assert!(
            client
                .transfer("/folder/", "https://elsewhere.example/copy/", true, false)
                .await
                .is_err()
        );
        assert!(client.transfer("/", "/copy/", true, false).await.is_err());
    }

    #[tokio::test]
    async fn calendar_collection_edits_escape_values_and_inspect_property_statuses() {
        let server = MockServer::start().await;
        let client = DavClient::new(&server.uri(), "u".into(), "p".into()).unwrap();
        Mock::given(method("MKCALENDAR"))
            .and(body_string_contains(
                "<d:displayname>Work &amp; Home &lt;3</d:displayname>",
            ))
            .and(body_string_contains(
                "<a:calendar-color>#112233</a:calendar-color>",
            ))
            .respond_with(ResponseTemplate::new(201))
            .expect(1)
            .mount(&server)
            .await;
        client
            .create_calendar("/new/", "Work & Home <3", None, Some("#112233"))
            .await
            .unwrap();
        Mock::given(method("PROPPATCH")).respond_with(ResponseTemplate::new(207).set_body_string(r#"<multistatus xmlns="DAV:"><response><href>/new/</href><propstat><prop><displayname/></prop><status>HTTP/1.1 403 Forbidden</status></propstat></response></multistatus>"#)).expect(1).mount(&server).await;
        assert!(
            client
                .update_calendar("/new/", Some("Changed"), None, None)
                .await
                .unwrap_err()
                .to_string()
                .contains("403")
        );
    }

    #[tokio::test]
    async fn recurring_expansion_and_freebusy_use_bounded_reports() {
        let server = MockServer::start().await;
        let client = DavClient::new(&server.uri(), "u".into(), "p".into()).unwrap();
        Mock::given(method("REPORT"))
            .and(body_string_contains(
                "<c:expand start=\"20261016T000000Z\" end=\"20261017T000000Z\"/>",
            ))
            .respond_with(
                ResponseTemplate::new(207).set_body_string("<multistatus xmlns=\"DAV:\"/>"),
            )
            .expect(1)
            .mount(&server)
            .await;
        client
            .events_expanded(
                "/cal/",
                Some("20261016T000000Z"),
                Some("20261017T000000Z"),
                true,
            )
            .await
            .unwrap();
        assert!(
            client
                .events_expanded("/cal/", None, None, true)
                .await
                .is_err()
        );
        Mock::given(method("REPORT"))
            .and(body_string_contains("free-busy-query"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                "BEGIN:VCALENDAR\r\nBEGIN:VFREEBUSY\r\nEND:VFREEBUSY\r\nEND:VCALENDAR\r\n",
            ))
            .expect(1)
            .mount(&server)
            .await;
        assert!(
            client
                .free_busy("/cal/", "20261016T000000Z", "20261017T000000Z")
                .await
                .unwrap()
                .contains("VFREEBUSY")
        );
    }

    #[tokio::test]
    async fn scheduling_posts_itip_and_returns_recipient_statuses() {
        let server = MockServer::start().await;
        let client = DavClient::new(&server.uri(), "u".into(), "p".into()).unwrap();
        Mock::given(method("POST")).and(path("/outbox/"))
            .and(header("Originator", "mailto:me@example.com")).and(header("Recipient", "mailto:you@example.com"))
            .and(body_string_contains("METHOD:REPLY"))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"<c:schedule-response xmlns:c="urn:ietf:params:xml:ns:caldav" xmlns:d="DAV:"><c:response><c:recipient><d:href>mailto:you@example.com</d:href></c:recipient><c:request-status>2.0;Success</c:request-status></c:response></c:schedule-response>"#)).expect(1).mount(&server).await;
        let result = client
            .schedule(
                "/outbox/",
                "mailto:me@example.com",
                &["mailto:you@example.com".into()],
                b"BEGIN:VCALENDAR\r\nMETHOD:REPLY\r\nEND:VCALENDAR\r\n".to_vec(),
            )
            .await
            .unwrap();
        assert_eq!(
            result["responses"][0]["recipient"],
            "mailto:you@example.com"
        );
        assert_eq!(result["responses"][0]["requestStatus"], "2.0;Success");
    }
}
