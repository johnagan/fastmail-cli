use anyhow::{anyhow, bail, Context, Result};
use imap::Session;
use native_tls::TlsStream;
use serde_json::{json, Value};
use std::net::TcpStream;

const HOST: &str = "imap.fastmail.com";
const FOLDER: &str = "Notes";
const MAX_DIFF_LINES: usize = 200;

type Imap = Session<TlsStream<TcpStream>>;

/// Notes are stored as mail in the Notes folder, so they're read over IMAP.
fn session(username: &str, password: &str) -> Result<Imap> {
    let tls = native_tls::TlsConnector::new().context("creating TLS connector")?;
    let client = imap::connect((HOST, 993), HOST, &tls).context("connecting to IMAP")?;
    client.login(username, password).map_err(|(err, _)| {
        anyhow!("IMAP login failed: {err}. Check FASTMAIL_IMAP_PASSWORD and that it has mail access")
    })
}

/// Reads one header field from a raw header block, unfolding continuation lines.
fn header_value(raw: &str, name: &str) -> Option<String> {
    let mut lines: Vec<String> = Vec::new();
    for line in raw.lines() {
        if line.starts_with(' ') || line.starts_with('\t') {
            if let Some(last) = lines.last_mut() {
                last.push(' ');
                last.push_str(line.trim());
            }
        } else if !line.is_empty() {
            lines.push(line.to_string());
        }
    }
    lines.iter().find_map(|l| {
        let (key, value) = l.split_once(':')?;
        key.trim().eq_ignore_ascii_case(name).then(|| value.trim().to_string())
    })
}

/// Splits a raw message into its header block and body.
fn split_message(raw: &str) -> (&str, &str) {
    raw.split_once("\r\n\r\n")
        .or_else(|| raw.split_once("\n\n"))
        .unwrap_or((raw, ""))
}

/// Returns the body as HTML, decoding quoted-printable when the message uses it.
fn decode_body(headers: &str, body: &str) -> String {
    match header_value(headers, "Content-Transfer-Encoding") {
        Some(enc) if enc.eq_ignore_ascii_case("quoted-printable") => decode_quoted_printable(body),
        _ => body.to_string(),
    }
}

/// Decodes quoted-printable transfer encoding, which Apple Notes uses for note bodies.
fn decode_quoted_printable(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'=' {
            if bytes.get(i + 1..i + 3).is_some_and(|h| h.iter().all(u8::is_ascii_hexdigit)) {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("00");
                out.push(u8::from_str_radix(hex, 16).unwrap_or(b'?'));
                i += 3;
                continue;
            }
            // Soft line break: "=" at the end of a line.
            let mut j = i + 1;
            if bytes.get(j) == Some(&b'\r') {
                j += 1;
            }
            if bytes.get(j) == Some(&b'\n') {
                i = j + 1;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn list(username: &str, password: &str, limit: usize) -> Result<Vec<Value>> {
    let mut imap = session(username, password)?;
    imap.examine(FOLDER).context("opening the Notes folder")?;
    let mut uids: Vec<u32> = imap.uid_search("ALL")?.into_iter().collect();
    uids.sort_unstable_by(|a, b| b.cmp(a));
    uids.truncate(limit);
    if uids.is_empty() {
        imap.logout().ok();
        return Ok(Vec::new());
    }
    let set = uids.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
    let fetches = imap.uid_fetch(&set, "(UID INTERNALDATE BODY.PEEK[HEADER.FIELDS (SUBJECT DATE)])")?;
    let notes = fetches
        .iter()
        .map(|f| {
            let headers = f.header().map(|h| String::from_utf8_lossy(h).into_owned()).unwrap_or_default();
            json!({
                "uid": f.uid,
                "subject": header_value(&headers, "Subject"),
                "date": f.internal_date().map(|d| d.to_rfc3339()),
            })
        })
        .collect();
    imap.logout().ok();
    Ok(notes)
}

pub fn get(username: &str, password: &str, uid: u32) -> Result<Value> {
    let mut imap = session(username, password)?;
    imap.examine(FOLDER).context("opening the Notes folder")?;
    let fetches = imap.uid_fetch(uid.to_string(), "(UID INTERNALDATE BODY.PEEK[])")?;
    let fetch = fetches.iter().next().context("no note with that UID")?;
    let raw = String::from_utf8_lossy(fetch.body().context("note has no body")?).into_owned();
    let (headers, body) = split_message(&raw);
    let html = decode_body(headers, body);
    let subject = header_value(headers, "Subject");
    let date = fetch.internal_date().map(|d| d.to_rfc3339());
    imap.logout().ok();
    Ok(json!({
        "uid": uid,
        "subject": subject,
        "date": date,
        "html": html,
    }))
}

pub fn ping(username: &str, password: &str) -> Result<()> {
    let mut imap = session(username, password)?;
    imap.examine(FOLDER).context("opening the Notes folder")?;
    imap.logout().ok();
    Ok(())
}

/// Builds a message around an HTML body. The Apple header is what makes Fastmail's notes view list it.
fn render(subject: &str, html: &str, from: &str, uuid: &str, apple_header: bool) -> String {
    let date = chrono::Utc::now().to_rfc2822();
    let mut msg = String::new();
    msg.push_str("Content-Type: text/html; charset=utf-8\r\n");
    msg.push_str("Content-Transfer-Encoding: 8bit\r\n");
    msg.push_str(&format!("Date: {date}\r\n"));
    msg.push_str(&format!("From: {from}\r\n"));
    msg.push_str(&format!("Subject: {subject}\r\n"));
    msg.push_str("MIME-Version: 1.0\r\n");
    if apple_header {
        msg.push_str("X-Uniform-Type-Identifier: com.apple.mail-note\r\n");
    }
    msg.push_str(&format!("X-Universally-Unique-Identifier: {uuid}\r\n"));
    msg.push_str("\r\n");
    msg.push_str(&html.replace("\r\n", "\n").replace('\n', "\r\n"));
    msg
}

fn lf(s: &str) -> String {
    s.replace("\r\n", "\n")
}

/// Appends a new note to the Notes folder. Returns the UID of the new message.
pub fn create(username: &str, password: &str, subject: &str, html: &str) -> Result<u32> {
    let mut imap = session(username, password)?;
    let first_new = next_uid(&mut imap)?;
    let uuid = uuid_string()?;
    let msg = render(subject, html, username, &uuid, true);
    imap.append(FOLDER, msg.as_bytes()).context("appending note")?;
    let uid = find_new_uid(&mut imap, &uuid, first_new)?;
    imap.logout().ok();
    Ok(uid)
}

/// Replaces a note's HTML body with `new_html`. Without `apply`, only the diff is returned.
///
/// With `apply`, the new version is written with the note's unique ID and Apple header, read back
/// and compared, and only then is the old version removed. If the read-back differs, the new version
/// is removed and the original stays.
pub fn put(username: &str, password: &str, uid: u32, new_html: &str, apply: bool) -> Result<Value> {
    let mut imap = session(username, password)?;
    let first_new = next_uid(&mut imap)?;
    let fetches = imap.uid_fetch(uid.to_string(), "BODY.PEEK[]")?;
    let fetch = fetches.iter().next().context("no note with that UID")?;
    let raw = String::from_utf8_lossy(fetch.body().context("note has no body")?).into_owned();
    let (headers, body) = split_message(&raw);
    let old_html = decode_body(headers, body);
    let uuid = header_value(headers, "X-Universally-Unique-Identifier")
        .context("note has no unique ID; refusing to replace it")?;
    let apple = header_value(headers, "X-Uniform-Type-Identifier").is_some();
    let subject = header_value(headers, "Subject").unwrap_or_default();

    let checks = safety_checks(&old_html, new_html)?;
    let diff = line_diff(&old_html, new_html);
    let summary = json!({
        "uid": uid,
        "applied": false,
        "existing_formatting_not_in_markdown": crate::markdown::to_markdown(&old_html).warnings,
        "added_lines": diff.added,
        "removed_lines": diff.removed,
        "diff": diff.lines,
        "checks": checks,
    });
    if !apply || (diff.added == 0 && diff.removed == 0) {
        imap.logout().ok();
        return Ok(summary);
    }

    let msg = render(&subject, new_html, username, &uuid, apple);
    imap.append(FOLDER, msg.as_bytes()).context("appending updated note")?;
    let new_uid = find_new_uid(&mut imap, &uuid, first_new)?;

    let back = imap.uid_fetch(new_uid.to_string(), "BODY.PEEK[]")?;
    let back_raw = String::from_utf8_lossy(back.iter().next().and_then(|f| f.body()).unwrap_or_default()).into_owned();
    let (back_headers, back_body) = split_message(&back_raw);
    let back_html = decode_body(back_headers, back_body);
    if lf(&back_html).trim() != lf(new_html).trim() {
        imap.uid_store(new_uid.to_string(), "+FLAGS (\\Deleted)")?;
        imap.expunge()?;
        imap.logout().ok();
        let (want, got) = (lf(new_html), lf(&back_html));
        let at = want.chars().zip(got.chars()).position(|(a, b)| a != b).unwrap_or(want.len().min(got.len()));
        let snip = |s: &str| s.chars().skip(at.saturating_sub(40)).take(90).collect::<String>();
        bail!(
            "read-back did not match at character {at} (wrote {} chars, read {} chars): wrote {:?}, read {:?}. Removed the new version; original is unchanged",
            want.chars().count(),
            got.chars().count(),
            snip(&want),
            snip(&got)
        );
    }

    imap.uid_store(uid.to_string(), "+FLAGS (\\Deleted)")?;
    imap.expunge()?;
    imap.logout().ok();
    let mut done = summary;
    done["uid"] = json!(new_uid);
    done["replaced"] = json!(uid);
    done["applied"] = json!(true);
    Ok(done)
}

/// Refuses changes that would silently drop embedded images, which Apple notes can't restore.
fn safety_checks(old_html: &str, new_html: &str) -> Result<Value> {
    let count = |html: &str, needle: &str| html.matches(needle).count();
    let old_imgs = count(old_html, "<img");
    let new_imgs = count(new_html, "<img");
    let old_cids = count(old_html, "cid:");
    let new_cids = count(new_html, "cid:");
    if new_imgs < old_imgs || new_cids < old_cids {
        bail!(
            "new HTML drops embedded images (images {old_imgs} -> {new_imgs}, inline references {old_cids} -> {new_cids}); refusing"
        );
    }
    Ok(json!({ "images": { "before": old_imgs, "after": new_imgs } }))
}

struct Diff {
    added: usize,
    removed: usize,
    lines: Vec<String>,
}

/// Line diff of the two HTML bodies, split at tag boundaries so changes are easy to read.
fn line_diff(old: &str, new: &str) -> Diff {
    let split = |s: &str| -> Vec<String> { s.replace("><", ">\n<").lines().map(str::to_string).collect() };
    let a = split(old);
    let b = split(new);
    let (n, m) = (a.len(), b.len());
    if n.saturating_mul(m) > 4_000_000 {
        return Diff { added: m, removed: n, lines: vec![format!("(diff skipped: {n} vs {m} lines)")] };
    }
    let mut lcs = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i][j] = if a[i] == b[j] { lcs[i + 1][j + 1] + 1 } else { lcs[i + 1][j].max(lcs[i][j + 1]) };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut lines = Vec::new();
    let (mut added, mut removed) = (0, 0);
    while i < n || j < m {
        if i < n && j < m && a[i] == b[j] {
            i += 1;
            j += 1;
        } else if j >= m || (i < n && lcs[i + 1][j] >= lcs[i][j + 1]) {
            removed += 1;
            lines.push(format!("- {}", a[i]));
            i += 1;
        } else {
            added += 1;
            lines.push(format!("+ {}", b[j]));
            j += 1;
        }
    }
    if lines.len() > MAX_DIFF_LINES {
        let extra = lines.len() - MAX_DIFF_LINES;
        lines.truncate(MAX_DIFF_LINES);
        lines.push(format!("... {extra} more changed lines"));
    }
    Diff { added, removed, lines }
}

/// The UID the server will assign to the next message appended to the Notes folder.
fn next_uid(imap: &mut Imap) -> Result<u32> {
    let mailbox = imap.select(FOLDER).context("opening the Notes folder")?;
    mailbox.uid_next.context("server did not report the next UID")
}

/// Finds the one message appended after `first_new` that carries `uuid`. Anything else is an error,
/// so a failed check never removes a message we can't confirm we wrote.
fn find_new_uid(imap: &mut Imap, uuid: &str, first_new: u32) -> Result<u32> {
    let fetches = imap.uid_fetch(
        format!("{first_new}:*"),
        "(UID BODY.PEEK[HEADER.FIELDS (X-UNIVERSALLY-UNIQUE-IDENTIFIER)])",
    )?;
    let mut hits = Vec::new();
    for f in fetches.iter() {
        let Some(uid) = f.uid else { continue };
        if uid < first_new {
            continue;
        }
        let headers = f.header().map(|h| String::from_utf8_lossy(h).into_owned()).unwrap_or_default();
        if header_value(&headers, "X-Universally-Unique-Identifier").as_deref() == Some(uuid) {
            hits.push(uid);
        }
    }
    match hits.as_slice() {
        [one] => Ok(*one),
        _ => bail!("could not confirm the new message ({} matches); nothing was removed", hits.len()),
    }
}

fn uuid_string() -> Result<String> {
    crate::uid::new_uuid()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_quoted_printable_soft_breaks_and_utf8() {
        let input = "Quick=\r\nref =E2=86=92 links";
        assert_eq!(decode_quoted_printable(input), "Quickref \u{2192} links");
    }

    #[test]
    fn diff_reports_only_changed_lines() {
        let old = "<div>a</div><div>b</div><div>c</div>";
        let new = "<div>a</div><div>B</div><div>c</div>";
        let d = line_diff(old, new);
        assert_eq!((d.added, d.removed), (1, 1));
        assert!(d.lines.contains(&"- <div>b</div>".to_string()));
        assert!(d.lines.contains(&"+ <div>B</div>".to_string()));
    }

    #[test]
    fn refuses_to_drop_images() {
        assert!(safety_checks("<img src=x><div>a</div>", "<div>a</div>").is_err());
        assert!(safety_checks("<img src=x>", "<img src=x><div>new</div>").is_ok());
    }
}

/// Removes a note from the Notes folder. Fastmail keeps deleted notes in Trash for a while.
pub fn delete(username: &str, password: &str, uid: u32) -> Result<()> {
    let mut imap = session(username, password)?;
    imap.select(FOLDER).context("opening the Notes folder")?;
    imap.uid_store(uid.to_string(), "+FLAGS (\\Deleted)")?;
    imap.expunge()?;
    imap.logout().ok();
    Ok(())
}
