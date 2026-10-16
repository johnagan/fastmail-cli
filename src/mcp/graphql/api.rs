use async_graphql::{Context, Json, SimpleObject};
use serde_json::Value;

#[derive(SimpleObject)]
pub struct CreatedRecord {
    pub id: String,
    /// The caller's creation key for /set results; null for fetched records.
    pub creation_id: Option<String>,
    pub blob_id: Option<String>,
    pub thread_id: Option<String>,
    pub size: Option<u64>,
    pub properties: Json<Value>,
}
impl From<Value> for CreatedRecord {
    fn from(value: Value) -> Self {
        Self {
            id: value["id"].as_str().unwrap_or_default().into(),
            creation_id: None,
            blob_id: text(&value, "blobId"),
            thread_id: text(&value, "threadId"),
            size: value["size"].as_u64(),
            properties: Json(value),
        }
    }
}

#[derive(SimpleObject)]
pub struct ObjectError {
    pub id: String,
    pub kind: String,
    pub description: Option<String>,
}

#[derive(SimpleObject)]
pub struct SetResult {
    pub success: bool,
    pub account_id: Option<String>,
    pub old_state: Option<String>,
    pub new_state: Option<String>,
    pub created: Vec<CreatedRecord>,
    pub updated: Vec<String>,
    pub destroyed: Vec<String>,
    pub not_created: Vec<ObjectError>,
    pub not_updated: Vec<ObjectError>,
    pub not_destroyed: Vec<ObjectError>,
}
fn text(value: &Value, key: &str) -> Option<String> {
    value[key].as_str().map(str::to_owned)
}
fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}
impl From<Value> for SetResult {
    fn from(value: Value) -> Self {
        let errors = |key| {
            value[key]
                .as_object()
                .into_iter()
                .flat_map(|m| m.iter())
                .map(|(id, error)| ObjectError {
                    id: id.clone(),
                    kind: error["type"].as_str().unwrap_or("unknown").into(),
                    description: text(error, "description"),
                })
                .collect::<Vec<_>>()
        };
        let not_created = errors("notCreated");
        let not_updated = errors("notUpdated");
        let not_destroyed = errors("notDestroyed");
        Self {
            success: not_created.is_empty() && not_updated.is_empty() && not_destroyed.is_empty(),
            account_id: text(&value, "accountId"),
            old_state: text(&value, "oldState"),
            new_state: text(&value, "newState"),
            created: value["created"]
                .as_object()
                .into_iter()
                .flat_map(|m| m.iter())
                .map(|(key, value)| {
                    let mut record = CreatedRecord::from(value.clone());
                    record.creation_id = Some(key.clone());
                    record
                })
                .collect(),
            updated: value["updated"]
                .as_object()
                .into_iter()
                .flat_map(|m| m.keys())
                .cloned()
                .collect(),
            destroyed: strings(&value["destroyed"]),
            not_created,
            not_updated,
            not_destroyed,
        }
    }
}

#[derive(SimpleObject)]
pub struct ChangeResult {
    pub old_state: Option<String>,
    pub new_state: String,
    pub has_more_changes: bool,
    pub created: Vec<String>,
    pub updated: Vec<String>,
    pub destroyed: Vec<String>,
    pub updated_properties: Option<Vec<String>>,
}
impl From<Value> for ChangeResult {
    fn from(value: Value) -> Self {
        Self {
            old_state: text(&value, "oldState"),
            new_state: text(&value, "newState").unwrap_or_default(),
            has_more_changes: value["hasMoreChanges"] == true,
            created: strings(&value["created"]),
            updated: strings(&value["updated"]),
            destroyed: strings(&value["destroyed"]),
            updated_properties: value["updatedProperties"]
                .as_array()
                .map(|_| strings(&value["updatedProperties"])),
        }
    }
}

#[derive(SimpleObject)]
pub struct RecordPage {
    pub account_id: Option<String>,
    pub state: Option<String>,
    pub records: Vec<CreatedRecord>,
    pub not_found: Vec<String>,
}
impl From<Value> for RecordPage {
    fn from(value: Value) -> Self {
        Self {
            account_id: text(&value, "accountId"),
            state: text(&value, "state"),
            records: value["list"]
                .as_array()
                .into_iter()
                .flatten()
                .cloned()
                .map(Into::into)
                .collect(),
            not_found: strings(&value["notFound"]),
        }
    }
}

#[derive(SimpleObject)]
pub struct VacationResponse {
    pub id: String,
    pub is_enabled: bool,
    pub from_date: Option<String>,
    pub to_date: Option<String>,
    pub subject: Option<String>,
    pub text_body: Option<String>,
    pub html_body: Option<String>,
}
impl From<Value> for VacationResponse {
    fn from(value: Value) -> Self {
        Self {
            id: text(&value, "id").unwrap_or_default(),
            is_enabled: value["isEnabled"] == true,
            from_date: text(&value, "fromDate"),
            to_date: text(&value, "toDate"),
            subject: text(&value, "subject"),
            text_body: text(&value, "textBody"),
            html_body: text(&value, "htmlBody"),
        }
    }
}

#[derive(SimpleObject)]
pub struct SearchSnippet {
    pub email_id: String,
    pub subject: Option<String>,
    pub preview: Option<String>,
}
#[derive(SimpleObject)]
pub struct SnippetResult {
    pub snippets: Vec<SearchSnippet>,
    pub not_found: Vec<String>,
}

#[derive(SimpleObject)]
pub struct NoteAttachment {
    pub blob_id: Option<String>,
    pub name: Option<String>,
    pub content_type: Option<String>,
    pub size: Option<u64>,
    pub cid: Option<String>,
    pub disposition: Option<String>,
}
#[derive(SimpleObject)]
pub struct Note {
    pub id: String,
    pub subject: Option<String>,
    pub uuid: Option<String>,
    pub preview: Option<String>,
    pub mailbox_ids: Vec<String>,
    pub keywords: Vec<String>,
    pub text_body: Option<String>,
    pub html_body: Option<String>,
    pub attachments: Vec<NoteAttachment>,
    pub properties: Json<Value>,
}
fn body(value: &Value, kind: &str) -> Option<String> {
    let parts: Vec<_> = value[kind]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|part| part["partId"].as_str())
        .filter_map(|id| value["bodyValues"][id]["value"].as_str())
        .collect();
    (!parts.is_empty()).then(|| parts.join("\n"))
}
impl From<Value> for Note {
    fn from(value: Value) -> Self {
        Self {
            id: text(&value, "id").unwrap_or_default(),
            subject: text(&value, "subject"),
            uuid: text(&value, "header:X-Universally-Unique-Identifier:asText"),
            preview: text(&value, "preview"),
            mailbox_ids: value["mailboxIds"]
                .as_object()
                .into_iter()
                .flat_map(|m| m.keys())
                .cloned()
                .collect(),
            keywords: value["keywords"]
                .as_object()
                .into_iter()
                .flat_map(|m| m.keys())
                .cloned()
                .collect(),
            text_body: body(&value, "textBody"),
            html_body: body(&value, "htmlBody"),
            attachments: value["attachments"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|a| NoteAttachment {
                    blob_id: text(a, "blobId"),
                    name: text(a, "name"),
                    content_type: text(a, "type"),
                    size: a["size"].as_u64(),
                    cid: text(a, "cid"),
                    disposition: text(a, "disposition"),
                })
                .collect(),
            properties: Json(value),
        }
    }
}
#[derive(SimpleObject)]
pub struct NotePage {
    pub notes: Vec<Note>,
    pub total: Option<u64>,
    pub query_state: Option<String>,
}
impl From<Value> for NotePage {
    fn from(value: Value) -> Self {
        Self {
            notes: value["notes"]
                .as_array()
                .into_iter()
                .flatten()
                .cloned()
                .map(Into::into)
                .collect(),
            total: value["total"].as_u64(),
            query_state: text(&value, "queryState"),
        }
    }
}

#[derive(SimpleObject)]
pub struct AddressBookRights {
    pub may_read: bool,
    pub may_write: bool,
    pub may_share: bool,
    pub may_delete: bool,
}
#[derive(SimpleObject)]
pub struct AddressBook {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub sort_order: u64,
    pub is_default: bool,
    pub is_subscribed: bool,
    pub my_rights: AddressBookRights,
    pub share_with: Json<Value>,
}
impl From<Value> for AddressBook {
    fn from(value: Value) -> Self {
        Self {
            id: text(&value, "id").unwrap_or_default(),
            name: text(&value, "name").unwrap_or_default(),
            description: text(&value, "description"),
            sort_order: value["sortOrder"].as_u64().unwrap_or_default(),
            is_default: value["isDefault"] == true,
            is_subscribed: value["isSubscribed"] == true,
            my_rights: AddressBookRights {
                may_read: value["myRights"]["mayRead"] == true,
                may_write: value["myRights"]["mayWrite"] == true,
                may_share: value["myRights"]["mayShare"] == true,
                may_delete: value["myRights"]["mayDelete"] == true,
            },
            share_with: Json(value["shareWith"].clone()),
        }
    }
}

#[derive(SimpleObject)]
pub struct SchedulingEndpoints {
    pub principal: Option<String>,
    pub inbox: Vec<String>,
    pub outbox: Vec<String>,
    pub addresses: Vec<String>,
    pub home: Vec<String>,
}
impl From<Value> for SchedulingEndpoints {
    fn from(value: Value) -> Self {
        Self {
            principal: text(&value, "principal"),
            inbox: strings(&value["inbox"]),
            outbox: strings(&value["outbox"]),
            addresses: strings(&value["addresses"]),
            home: strings(&value["home"]),
        }
    }
}
#[derive(SimpleObject)]
pub struct SchedulingResponse {
    pub recipient: Option<String>,
    pub request_status: Option<String>,
    pub calendar_data: Option<String>,
}
pub fn scheduling_responses(value: Value) -> Vec<SchedulingResponse> {
    value["responses"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|v| SchedulingResponse {
            recipient: text(v, "recipient"),
            request_status: text(v, "requestStatus"),
            calendar_data: text(v, "calendarData"),
        })
        .collect()
}

#[derive(SimpleObject)]
pub struct ParsedEmail {
    pub blob_id: String,
    pub subject: Option<String>,
    pub text_body: Option<String>,
    pub html_body: Option<String>,
    pub properties: Json<Value>,
}
#[derive(SimpleObject)]
pub struct ParseResult {
    pub parsed: Vec<ParsedEmail>,
    pub not_found: Vec<String>,
    pub not_parsable: Vec<String>,
}
impl From<Value> for ParseResult {
    fn from(value: Value) -> Self {
        Self {
            parsed: value["parsed"]
                .as_object()
                .into_iter()
                .flat_map(|m| m.iter())
                .map(|(id, v)| ParsedEmail {
                    blob_id: id.clone(),
                    subject: text(v, "subject"),
                    text_body: body(v, "textBody"),
                    html_body: body(v, "htmlBody"),
                    properties: Json(v.clone()),
                })
                .collect(),
            not_found: strings(&value["notFound"]),
            not_parsable: strings(&value["notParsable"]),
        }
    }
}
impl From<Value> for SnippetResult {
    fn from(value: Value) -> Self {
        Self {
            snippets: value["list"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|v| SearchSnippet {
                    email_id: text(v, "emailId").unwrap_or_default(),
                    subject: text(v, "subject"),
                    preview: text(v, "preview"),
                })
                .collect(),
            not_found: strings(&value["notFound"]),
        }
    }
}

#[derive(SimpleObject)]
pub struct BlobDownload {
    pub base64: String,
    pub size: u64,
    pub etag: Option<String>,
}

#[derive(SimpleObject)]
pub struct QueryAddition {
    pub id: String,
    pub index: u64,
}
#[derive(SimpleObject)]
pub struct QueryResult {
    pub ids: Vec<String>,
    pub position: Option<u64>,
    pub total: Option<u64>,
    pub query_state: Option<String>,
    pub can_calculate_changes: Option<bool>,
    pub old_query_state: Option<String>,
    pub new_query_state: Option<String>,
    pub removed: Vec<String>,
    pub added: Vec<QueryAddition>,
}
impl From<Value> for QueryResult {
    fn from(value: Value) -> Self {
        Self {
            ids: strings(&value["ids"]),
            position: value["position"].as_u64(),
            total: value["total"].as_u64(),
            query_state: text(&value, "queryState"),
            can_calculate_changes: value["canCalculateChanges"].as_bool(),
            old_query_state: text(&value, "oldQueryState"),
            new_query_state: text(&value, "newQueryState"),
            removed: strings(&value["removed"]),
            added: value["added"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|v| QueryAddition {
                    id: text(v, "id").unwrap_or_default(),
                    index: v["index"].as_u64().unwrap_or_default(),
                })
                .collect(),
        }
    }
}
#[derive(SimpleObject)]
pub struct CalendarData {
    pub href: String,
    pub etag: Option<String>,
    pub calendar_data: String,
}
#[derive(SimpleObject)]
pub struct DavWriteResult {
    pub href: String,
    pub etag: Option<String>,
    pub success: bool,
}
#[derive(SimpleObject)]
pub struct FileEntry {
    pub path: String,
    pub href: String,
    pub name: String,
    pub collection: bool,
    pub content_type: Option<String>,
    pub size: Option<u64>,
    pub etag: Option<String>,
}

#[cfg(test)]
pub(super) struct TestDavBase(pub String);

pub fn dav(ctx: &Context<'_>, files: bool) -> async_graphql::Result<crate::dav::DavClient> {
    let creds = &ctx.data::<super::DavCreds>()?.0;
    let username = creds
        .username
        .clone()
        .ok_or("DAV username is not configured")?;
    let password = creds
        .app_password
        .clone()
        .ok_or("DAV app password is not configured")?;
    let base = if files {
        "https://myfiles.fastmail.com/"
    } else {
        "https://caldav.fastmail.com/"
    };
    #[cfg(test)]
    let base = ctx
        .data_opt::<TestDavBase>()
        .map(|b| b.0.as_str())
        .unwrap_or(base);
    Ok(crate::dav::DavClient::new(base, username, password)?)
}

pub fn validate_calendar(data: &str) -> async_graphql::Result<()> {
    if !data.lines().any(|l| l.trim_end() == "BEGIN:VCALENDAR")
        || !data.lines().any(|l| l.trim_end() == "BEGIN:VEVENT")
        || !data.lines().any(|l| l.trim_end() == "END:VCALENDAR")
    {
        return Err("Expected a complete VCALENDAR containing a VEVENT".into());
    }
    Ok(())
}
