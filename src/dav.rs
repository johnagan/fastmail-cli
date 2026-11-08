use crate::http::{encode_path, send};
use anyhow::{bail, Context, Result};
use quick_xml::events::Event;
use quick_xml::Reader;
use reqwest::{Client, Method};
use serde_json::{json, Value};
use std::collections::BTreeMap;

const CALDAV: &str = "https://caldav.fastmail.com";
const CARDDAV: &str = "https://carddav.fastmail.com";
const WEBDAV: &str = "https://myfiles.fastmail.com";

/// One `<response>` from a WebDAV multistatus reply.
#[derive(Debug, Default)]
pub struct Entry {
    pub href: String,
    pub props: BTreeMap<String, String>,
    pub types: Vec<String>,
}

fn name_of(local: &str) -> String {
    local.to_string()
}

fn general_ref(name: &str) -> Option<&'static str> {
    Some(match name {
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" => "'",
        _ => return None,
    })
}

/// Parses a multistatus reply into entries. Only the parts this CLI reads are kept:
/// the href, leaf text properties, home-set hrefs, and resource types.
pub fn parse(xml: &str) -> Result<Vec<Entry>> {
    let mut reader = Reader::from_str(xml);
    let mut stack: Vec<String> = Vec::new();
    let mut entries = Vec::new();
    let mut current: Option<Entry> = None;
    let mut text = String::new();

    loop {
        match reader.read_event().context("reading XML")? {
            Event::Start(e) => {
                let name = name_of(e.local_name().as_ref());
                if name == "response" {
                    current = Some(Entry::default());
                }
                text.clear();
                stack.push(name);
            }
            Event::Empty(e) => {
                let name = name_of(e.local_name().as_ref());
                if stack.last().map(String::as_str) == Some("resourcetype") {
                    if let Some(entry) = current.as_mut() {
                        entry.types.push(name);
                    }
                }
            }
            Event::Text(t) => {
                text.push_str(&quick_xml::escape::unescape(&t).context("unescaping XML text")?);
            }
            Event::CData(c) => {
                text.push_str(&c);
            }
            Event::GeneralRef(r) => {
                let name = name_of(&r);
                if let Some(s) = general_ref(&name) {
                    text.push_str(s);
                } else if let Some(c) = r.resolve_char_ref().ok().flatten() {
                    text.push(c);
                }
            }
            Event::End(_) => {
                let Some(name) = stack.pop() else { continue };
                let parent = stack.last().cloned().unwrap_or_default();
                let grandparent = stack.iter().rev().nth(1).cloned().unwrap_or_default();
                if let Some(entry) = current.as_mut() {
                    if name == "href" {
                        if parent == "calendar-home-set" || parent == "addressbook-home-set" {
                            entry.props.insert(parent.clone(), text.clone());
                        } else if parent == "response" {
                            entry.href = text.clone();
                        }
                    } else if name != "response" && !text.trim().is_empty() && grandparent != "resourcetype" {
                        entry.props.insert(name.clone(), text.clone());
                    }
                }
                if name == "response" {
                    if let Some(entry) = current.take() {
                        entries.push(entry);
                    }
                }
                text.clear();
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(entries)
}

/// Splits an iCalendar or vCard body into records, unfolding continuation lines.
/// Each record is a list of (property name, value) pairs in file order.
fn records(body: &str, begin: &str, end: &str) -> Vec<Vec<(String, String)>> {
    let mut lines: Vec<String> = Vec::new();
    for raw in body.lines() {
        if raw.starts_with(' ') || raw.starts_with('\t') {
            if let Some(last) = lines.last_mut() {
                last.push_str(&raw[1..]);
            }
        } else {
            lines.push(raw.to_string());
        }
    }
    let mut out = Vec::new();
    let mut current: Option<Vec<(String, String)>> = None;
    for line in lines {
        if line.eq_ignore_ascii_case(begin) {
            current = Some(Vec::new());
        } else if line.eq_ignore_ascii_case(end) {
            if let Some(rec) = current.take() {
                out.push(rec);
            }
        } else if let Some(rec) = current.as_mut() {
            if let Some((key, value)) = line.split_once(':') {
                let name = key.split(';').next().unwrap_or(key).to_ascii_uppercase();
                rec.push((name, unescape_value(value)));
            }
        }
    }
    out
}

fn unescape_value(v: &str) -> String {
    v.replace("\\n", "\n").replace("\\N", "\n").replace("\\,", ",").replace("\\;", ";").replace("\\\\", "\\")
}

fn first(rec: &[(String, String)], key: &str) -> Option<String> {
    rec.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
}

pub struct Dav {
    http: Client,
    username: String,
    password: String,
}

impl Dav {
    pub fn new(http: Client, username: &str, password: &str) -> Self {
        Self { http, username: username.to_string(), password: password.to_string() }
    }

    async fn request(&self, method: &str, url: &str, depth: &str, body: &str) -> Result<(u16, String)> {
        let method = Method::from_bytes(method.as_bytes())?;
        let req = self
            .http
            .request(method, url)
            .basic_auth(&self.username, Some(&self.password))
            .header("Depth", depth)
            .header("Content-Type", "application/xml; charset=utf-8")
            .body(body.to_string());
        let resp = send(req).await?;
        let status = resp.status().as_u16();
        Ok((status, resp.text().await.context("reading DAV response")?))
    }

    async fn multistatus(&self, method: &str, url: &str, depth: &str, body: &str) -> Result<Vec<Entry>> {
        let (status, text) = self.request(method, url, depth, body).await?;
        match status {
            207 => parse(&text),
            401 | 403 => bail!("{method} {url}: HTTP {status}; check the DAV app password and that it covers this service"),
            _ => bail!("{method} {url}: HTTP {status}"),
        }
    }

    fn principal(&self, host: &str) -> String {
        format!("{host}/dav/principals/user/{}/", self.username)
    }

    /// Finds a home-set URL (calendar or addressbook) from the principal.
    async fn home(&self, host: &str, home_set: &str, ns: &str) -> Result<String> {
        let body = format!(
            r#"<?xml version="1.0"?><d:propfind xmlns:d="DAV:" xmlns:c="{ns}"><d:prop><c:{home_set}/></d:prop></d:propfind>"#
        );
        let entries = self.multistatus("PROPFIND", &self.principal(host), "0", &body).await?;
        let href = entries
            .iter()
            .find_map(|e| e.props.get(home_set).cloned())
            .with_context(|| format!("principal has no {home_set}"))?;
        Ok(absolute(host, &href))
    }

    pub async fn calendar_home(&self) -> Result<String> {
        self.home(CALDAV, "calendar-home-set", "urn:ietf:params:xml:ns:caldav").await
    }

    pub async fn addressbook_home(&self) -> Result<String> {
        self.home(CARDDAV, "addressbook-home-set", "urn:ietf:params:xml:ns:carddav").await
    }

    pub async fn calendars(&self) -> Result<Vec<Value>> {
        let home = self.calendar_home().await?;
        let body = r#"<?xml version="1.0"?><d:propfind xmlns:d="DAV:"><d:prop><d:displayname/><d:resourcetype/></d:prop></d:propfind>"#;
        let entries = self.multistatus("PROPFIND", &home, "1", body).await?;
        Ok(entries
            .iter()
            .filter(|e| e.types.iter().any(|t| t == "calendar"))
            .map(|e| json!({
                "name": e.props.get("displayname").cloned().unwrap_or_default(),
                "href": absolute(CALDAV, &e.href),
            }))
            .collect())
    }

    pub async fn events(&self, calendar_href: &str, from: &str, to: &str) -> Result<Vec<Value>> {
        let body = format!(
            r#"<?xml version="1.0"?><c:calendar-query xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:prop><c:calendar-data/></d:prop><c:filter><c:comp-filter name="VCALENDAR"><c:comp-filter name="VEVENT"><c:time-range start="{from}" end="{to}"/></c:comp-filter></c:comp-filter></c:filter></c:calendar-query>"#
        );
        let entries = self.multistatus("REPORT", calendar_href, "1", &body).await?;
        let mut out = Vec::new();
        for entry in entries {
            let Some(ical) = entry.props.get("calendar-data") else { continue };
            for rec in records(ical, "BEGIN:VEVENT", "END:VEVENT") {
                out.push(json!({
                    "href": absolute(CALDAV, &entry.href),
                    "uid": first(&rec, "UID"),
                    "summary": first(&rec, "SUMMARY"),
                    "start": first(&rec, "DTSTART"),
                    "end": first(&rec, "DTEND"),
                    "location": first(&rec, "LOCATION"),
                }));
            }
        }
        Ok(out)
    }

    pub async fn contacts(&self, query: Option<&str>) -> Result<Vec<Value>> {
        let home = self.addressbook_home().await?;
        let body = r#"<?xml version="1.0"?><c:addressbook-query xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:carddav"><d:prop><c:address-data/></d:prop></c:addressbook-query>"#;
        let entries = self.multistatus("REPORT", &home, "1", body).await?;
        let needle = query.map(str::to_lowercase);
        let mut out = Vec::new();
        for entry in entries {
            let Some(card) = entry.props.get("address-data") else { continue };
            for rec in records(card, "BEGIN:VCARD", "END:VCARD") {
                let emails: Vec<String> = rec.iter().filter(|(k, _)| k == "EMAIL").map(|(_, v)| v.clone()).collect();
                let phones: Vec<String> = rec.iter().filter(|(k, _)| k == "TEL").map(|(_, v)| v.clone()).collect();
                let contact = json!({
                    "href": absolute(CARDDAV, &entry.href),
                    "name": first(&rec, "FN"),
                    "emails": emails,
                    "phones": phones,
                    "organization": first(&rec, "ORG"),
                });
                if let Some(n) = &needle {
                    let hay = contact.to_string().to_lowercase();
                    if !hay.contains(n) {
                        continue;
                    }
                }
                out.push(contact);
            }
        }
        Ok(out)
    }

    pub async fn files_ls(&self, path: &str) -> Result<Vec<Value>> {
        let url = webdav_url(path, true);
        let body = r#"<?xml version="1.0"?><d:propfind xmlns:d="DAV:"><d:prop><d:displayname/><d:resourcetype/><d:getcontentlength/><d:getlastmodified/></d:prop></d:propfind>"#;
        let entries = self.multistatus("PROPFIND", &url, "1", body).await?;
        let own = url.trim_end_matches('/').to_string();
        Ok(entries
            .iter()
            .filter(|e| absolute(WEBDAV, &e.href).trim_end_matches('/') != own)
            .map(|e| json!({
                "name": e.props.get("displayname").cloned().unwrap_or_default(),
                "href": absolute(WEBDAV, &e.href),
                "directory": e.types.iter().any(|t| t == "collection"),
                "size": e.props.get("getcontentlength").and_then(|s| s.parse::<u64>().ok()),
                "modified": e.props.get("getlastmodified"),
            }))
            .collect())
    }

    pub async fn files_get(&self, path: &str) -> Result<Vec<u8>> {
        let url = webdav_url(path, false);
        let resp = send(self.http.get(&url).basic_auth(&self.username, Some(&self.password))).await?;
        if !resp.status().is_success() {
            bail!("GET {url}: HTTP {}", resp.status());
        }
        Ok(resp.bytes().await.context("reading file")?.to_vec())
    }

    /// Uploads a file with PUT. Refuses to overwrite an existing file unless `overwrite` is set.
    pub async fn files_put(&self, path: &str, bytes: Vec<u8>, overwrite: bool) -> Result<String> {
        let url = webdav_url(path, false);
        if !overwrite {
            let (status, _) = self.request("PROPFIND", &url, "0", r#"<?xml version="1.0"?><d:propfind xmlns:d="DAV:"><d:prop><d:displayname/></d:prop></d:propfind>"#).await?;
            if status == 207 {
                bail!("{path} already exists; pass --overwrite to replace it");
            }
        }
        let resp = send(
            self.http
                .put(&url)
                .basic_auth(&self.username, Some(&self.password))
                .header("Content-Type", "application/octet-stream")
                .body(bytes),
        )
        .await?;
        let status = resp.status();
        if !status.is_success() {
            bail!("PUT {url}: HTTP {status}");
        }
        Ok(url)
    }

    /// Reachability check used by `doctor`. Returns the HTTP status.
    pub async fn ping(&self, host: &str) -> Result<u16> {
        let body = r#"<?xml version="1.0"?><d:propfind xmlns:d="DAV:"><d:prop><d:displayname/></d:prop></d:propfind>"#;
        let url = if host == WEBDAV { format!("{WEBDAV}/") } else { self.principal(host) };
        Ok(self.request("PROPFIND", &url, "0", body).await?.0)
    }
}

/// Splits a text body into logical lines, joining folded continuation lines.
fn unfold_lines(body: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for raw in body.replace("\r\n", "\n").split('\n') {
        if raw.starts_with(' ') || raw.starts_with('\t') {
            if let Some(last) = lines.last_mut() {
                last.push_str(&raw[1..]);
            }
        } else if !raw.is_empty() {
            lines.push(raw.to_string());
        }
    }
    lines
}

fn property_name(line: &str) -> String {
    line.split([':', ';']).next().unwrap_or("").to_ascii_uppercase()
}

fn escape_value(v: &str) -> String {
    v.replace('\\', "\\\\").replace(';', "\\;").replace(',', "\\,").replace('\n', "\\n")
}

fn new_uid() -> Result<String> {
    crate::uid::new_uuid()
}

/// Builds a vCard from scratch. Used for new contacts.
fn vcard_new(uid: &str, name: &str, emails: &[String], phones: &[String], org: Option<&str>) -> String {
    let mut lines = vec!["BEGIN:VCARD".to_string(), "VERSION:3.0".to_string(), format!("UID:{uid}")];
    lines.push(format!("FN:{}", escape_value(name)));
    lines.push(format!("N:{};;;;", escape_value(name)));
    lines.extend(emails.iter().map(|e| format!("EMAIL:{}", escape_value(e))));
    lines.extend(phones.iter().map(|p| format!("TEL:{}", escape_value(p))));
    if let Some(o) = org {
        lines.push(format!("ORG:{}", escape_value(o)));
    }
    lines.push("END:VCARD".to_string());
    lines.join("\r\n") + "\r\n"
}

/// Applies changes to an existing vCard. Properties not being replaced are kept as they were.
fn vcard_update(existing: &str, name: Option<&str>, emails: Option<&[String]>, phones: Option<&[String]>, org: Option<&str>) -> String {
    let replaced = |key: &str| match key {
        "FN" | "N" => name.is_some(),
        "EMAIL" => emails.is_some(),
        "TEL" => phones.is_some(),
        "ORG" => org.is_some(),
        _ => false,
    };
    let mut out: Vec<String> = Vec::new();
    for line in unfold_lines(existing) {
        if line.eq_ignore_ascii_case("END:VCARD") {
            if let Some(n) = name {
                out.push(format!("FN:{}", escape_value(n)));
                out.push(format!("N:{};;;;", escape_value(n)));
            }
            out.extend(emails.unwrap_or(&[]).iter().map(|e| format!("EMAIL:{}", escape_value(e))));
            out.extend(phones.unwrap_or(&[]).iter().map(|p| format!("TEL:{}", escape_value(p))));
            if let Some(o) = org {
                out.push(format!("ORG:{}", escape_value(o)));
            }
            out.push(line);
        } else if !replaced(&property_name(&line)) {
            out.push(line);
        }
    }
    out.join("\r\n") + "\r\n"
}

fn ics_time(input: &str) -> Result<String> {
    let t = chrono::DateTime::parse_from_rfc3339(input)
        .with_context(|| format!("'{input}' is not an RFC 3339 time such as 2026-11-20T15:00:00Z"))?;
    Ok(t.with_timezone(&chrono::Utc).format("%Y%m%dT%H%M%SZ").to_string())
}

/// Builds an event from scratch. Used for new events.
fn ics_new(uid: &str, title: &str, start: &str, end: &str, location: Option<&str>, description: Option<&str>) -> Result<String> {
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let mut lines = vec![
        "BEGIN:VCALENDAR".to_string(),
        "VERSION:2.0".to_string(),
        "PRODID:-//fastmail-cli//EN".to_string(),
        "BEGIN:VEVENT".to_string(),
        format!("UID:{uid}"),
        format!("DTSTAMP:{stamp}"),
        format!("DTSTART:{}", ics_time(start)?),
        format!("DTEND:{}", ics_time(end)?),
        format!("SUMMARY:{}", escape_value(title)),
    ];
    if let Some(l) = location {
        lines.push(format!("LOCATION:{}", escape_value(l)));
    }
    if let Some(d) = description {
        lines.push(format!("DESCRIPTION:{}", escape_value(d)));
    }
    lines.push("END:VEVENT".to_string());
    lines.push("END:VCALENDAR".to_string());
    Ok(lines.join("\r\n") + "\r\n")
}

/// Applies changes to an existing event. Other properties are kept as they were.
fn ics_update(existing: &str, title: Option<&str>, start: Option<&str>, end: Option<&str>, location: Option<&str>, description: Option<&str>) -> Result<String> {
    let replaced = |key: &str| match key {
        "SUMMARY" => title.is_some(),
        "DTSTART" => start.is_some(),
        "DTEND" => end.is_some(),
        "LOCATION" => location.is_some(),
        "DESCRIPTION" => description.is_some(),
        _ => false,
    };
    let mut out: Vec<String> = Vec::new();
    let mut in_event = false;
    for line in unfold_lines(existing) {
        if line.eq_ignore_ascii_case("BEGIN:VEVENT") {
            in_event = true;
        }
        if line.eq_ignore_ascii_case("END:VEVENT") && in_event {
            in_event = false;
            if let Some(t) = title {
                out.push(format!("SUMMARY:{}", escape_value(t)));
            }
            if let Some(s) = start {
                out.push(format!("DTSTART:{}", ics_time(s)?));
            }
            if let Some(e) = end {
                out.push(format!("DTEND:{}", ics_time(e)?));
            }
            if let Some(l) = location {
                out.push(format!("LOCATION:{}", escape_value(l)));
            }
            if let Some(d) = description {
                out.push(format!("DESCRIPTION:{}", escape_value(d)));
            }
        }
        if !(in_event && replaced(&property_name(&line))) {
            out.push(line);
        }
    }
    Ok(out.join("\r\n") + "\r\n")
}

fn absolute(host: &str, href: &str) -> String {
    if href.starts_with("http") { href.to_string() } else { format!("{host}{href}") }
}

fn webdav_url(path: &str, directory: bool) -> String {
    let trimmed = path.trim_matches('/');
    if trimmed.is_empty() {
        format!("{WEBDAV}/")
    } else if directory {
        format!("{WEBDAV}/{}/", encode_path(trimmed))
    } else {
        format!("{WEBDAV}/{}", encode_path(trimmed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_cdata_text() {
        let xml = r#"<d:multistatus xmlns:d="DAV:"><d:response><d:href>/x/</d:href><d:propstat><d:prop><d:displayname><![CDATA[Acme & Co]]></d:displayname></d:prop></d:propstat></d:response></d:multistatus>"#;
        let entries = parse(xml).unwrap();
        assert_eq!(entries[0].props.get("displayname").map(String::as_str), Some("Acme & Co"));
    }

    #[test]
    fn reads_displayname_and_resource_types() {
        let xml = r#"<?xml version="1.0"?>
<d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">
 <d:response>
  <d:href>/dav/calendars/user/a@b/cal1/</d:href>
  <d:propstat>
   <d:prop>
    <d:displayname>Personal</d:displayname>
    <d:resourcetype><d:collection/><c:calendar/></d:resourcetype>
   </d:prop>
  </d:propstat>
 </d:response>
</d:multistatus>"#;
        let entries = parse(xml).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].props.get("displayname").map(String::as_str), Some("Personal"));
        assert!(entries[0].types.iter().any(|t| t == "calendar"));
    }

    #[test]
    fn vcard_update_keeps_untouched_properties() {
        let card = "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:u1\r\nFN:Old Name\r\nEMAIL:a@x.com\r\nNOTE:keep me\r\nEND:VCARD\r\n";
        let out = vcard_update(card, Some("New Name"), None, None, None);
        assert!(out.contains("FN:New Name"));
        assert!(!out.contains("FN:Old Name"));
        assert!(out.contains("EMAIL:a@x.com"));
        assert!(out.contains("NOTE:keep me"));
        assert!(out.contains("UID:u1"));
    }

    #[test]
    fn ics_update_replaces_only_given_fields() {
        let ev = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:e1\r\nSUMMARY:Old\r\nLOCATION:Home\r\nDTSTART:20261120T150000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let out = ics_update(ev, Some("New"), None, None, None, None).unwrap();
        assert!(out.contains("SUMMARY:New"));
        assert!(!out.contains("SUMMARY:Old"));
        assert!(out.contains("LOCATION:Home"));
        assert!(out.contains("DTSTART:20261120T150000Z"));
    }

    #[test]
    fn escapes_commas_and_semicolons() {
        assert_eq!(escape_value("a, b; c"), "a\\, b\\; c");
    }
}

impl Dav {
    async fn get_text(&self, url: &str) -> Result<String> {
        let resp = send(self.http.get(url).basic_auth(&self.username, Some(&self.password))).await?;
        if !resp.status().is_success() {
            bail!("GET {url}: HTTP {}", resp.status());
        }
        resp.text().await.context("reading item")
    }

    /// PUT a new item. `If-None-Match: *` makes the server refuse to replace an existing item.
    async fn put_new(&self, url: &str, content_type: &str, body: String) -> Result<()> {
        let resp = send(
            self.http
                .put(url)
                .basic_auth(&self.username, Some(&self.password))
                .header("Content-Type", content_type)
                .header("If-None-Match", "*")
                .body(body),
        )
        .await?;
        if !resp.status().is_success() {
            bail!("PUT {url}: HTTP {}", resp.status());
        }
        Ok(())
    }

    async fn put_replace(&self, url: &str, content_type: &str, body: String) -> Result<()> {
        let resp = send(
            self.http
                .put(url)
                .basic_auth(&self.username, Some(&self.password))
                .header("Content-Type", content_type)
                .body(body),
        )
        .await?;
        if !resp.status().is_success() {
            bail!("PUT {url}: HTTP {}", resp.status());
        }
        Ok(())
    }

    async fn delete_item(&self, url: &str) -> Result<()> {
        let resp = send(self.http.delete(url).basic_auth(&self.username, Some(&self.password))).await?;
        if !resp.status().is_success() {
            bail!("DELETE {url}: HTTP {}", resp.status());
        }
        Ok(())
    }

    /// The address book new contacts go into: Default, or the first book that isn't shared.
    async fn writable_addressbook(&self) -> Result<String> {
        let home = self.addressbook_home().await?;
        let body = r#"<?xml version="1.0"?><d:propfind xmlns:d="DAV:"><d:prop><d:displayname/><d:resourcetype/></d:prop></d:propfind>"#;
        let entries = self.multistatus("PROPFIND", &home, "1", body).await?;
        let books: Vec<String> = entries
            .iter()
            .filter(|e| e.types.iter().any(|t| t == "addressbook") && !e.href.contains("Shared"))
            .map(|e| absolute(CARDDAV, &e.href))
            .collect();
        books
            .iter()
            .find(|b| b.ends_with("/Default/"))
            .or(books.first())
            .cloned()
            .context("no writable address book")
    }

    pub async fn contact_create(&self, name: &str, emails: &[String], phones: &[String], org: Option<&str>) -> Result<String> {
        let book = self.writable_addressbook().await?;
        let uid = new_uid()?;
        let url = format!("{}{uid}.vcf", if book.ends_with('/') { book.clone() } else { format!("{book}/") });
        self.put_new(&url, "text/vcard; charset=utf-8", vcard_new(&uid, name, emails, phones, org)).await?;
        Ok(url)
    }

    pub async fn contact_update(&self, href: &str, name: Option<&str>, emails: Option<&[String]>, phones: Option<&[String]>, org: Option<&str>) -> Result<()> {
        let existing = self.get_text(href).await?;
        let updated = vcard_update(&existing, name, emails, phones, org);
        self.put_replace(href, "text/vcard; charset=utf-8", updated).await
    }

    pub async fn contact_delete(&self, href: &str) -> Result<()> {
        self.delete_item(href).await
    }

    pub async fn event_create(&self, calendar_href: &str, title: &str, start: &str, end: &str, location: Option<&str>, description: Option<&str>) -> Result<String> {
        let uid = new_uid()?;
        let base = if calendar_href.ends_with('/') { calendar_href.to_string() } else { format!("{calendar_href}/") };
        let url = format!("{base}{uid}.ics");
        let body = ics_new(&uid, title, start, end, location, description)?;
        self.put_new(&url, "text/calendar; charset=utf-8", body).await?;
        Ok(url)
    }

    pub async fn event_update(&self, href: &str, title: Option<&str>, start: Option<&str>, end: Option<&str>, location: Option<&str>, description: Option<&str>) -> Result<()> {
        let existing = self.get_text(href).await?;
        let updated = ics_update(&existing, title, start, end, location, description)?;
        self.put_replace(href, "text/calendar; charset=utf-8", updated).await
    }

    pub async fn event_delete(&self, href: &str) -> Result<()> {
        self.delete_item(href).await
    }

    pub async fn file_delete(&self, path: &str) -> Result<()> {
        self.delete_item(&webdav_url(path, false)).await
    }

    pub async fn file_move(&self, from: &str, to: &str, overwrite: bool) -> Result<String> {
        let src = webdav_url(from, false);
        let dst = webdav_url(to, false);
        let resp = send(
            self.http
                .request(Method::from_bytes(b"MOVE")?, &src)
                .basic_auth(&self.username, Some(&self.password))
                .header("Destination", &dst)
                .header("Overwrite", if overwrite { "T" } else { "F" }),
        )
        .await?;
        if !resp.status().is_success() {
            bail!("MOVE {src} -> {dst}: HTTP {}", resp.status());
        }
        Ok(dst)
    }
}
