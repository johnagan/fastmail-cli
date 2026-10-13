//! Attachments on outgoing mail.
//!
//! The server usually runs remotely, so a client cannot name a file on its own
//! disk. Content arrives either inline as base64, or as the ID of a blob already
//! on the server: one put there by `uploadAttachment`, or an attachment of an
//! existing email.

use async_graphql::{InputObject, SimpleObject};
use base64::Engine;

use crate::jmap::{AttachmentContent, AttachmentData, JmapClient};

/// Total size allowed across one message's attachments, and for one upload.
/// Fastmail accepts 50 MB per message, but every byte of an inline attachment
/// passes through a tool argument as base64, a third larger again.
pub const MAX_ATTACHMENT_BYTES: usize = 25 * 1024 * 1024;

/// A file to attach. Give exactly one of `base64` and `blobId`.
#[derive(InputObject)]
#[graphql(name = "AttachmentInput")]
pub struct AttachmentInput {
    /// File name the recipient sees.
    pub name: String,
    /// MIME type. Inferred from the name's extension when omitted.
    pub content_type: Option<String>,
    /// The file's bytes, base64-encoded.
    pub base64: Option<String>,
    /// A blob already on the server: from `uploadAttachment`, or an existing
    /// email's `Attachment.blobId`. Not re-uploaded.
    pub blob_id: Option<String>,
}

/// A blob put on the server by `uploadAttachment`, ready to attach by its ID.
#[derive(SimpleObject)]
#[graphql(name = "UploadedAttachment")]
pub struct GqlUploadedAttachment {
    pub blob_id: String,
    pub name: String,
    pub content_type: String,
    pub size: u64,
}

/// An attachment with its content resolved and measured.
pub struct Attachment {
    pub data: AttachmentData,
    pub size: usize,
}

/// Decode, measure and check a message's attachments against the size cap.
///
/// A blob is downloaded to learn its size: Fastmail does not offer JMAP's
/// `Blob/get`, and a preview that cannot say how large an attachment is cannot
/// be checked by the person approving it.
pub async fn resolve(
    client: &JmapClient,
    inputs: Vec<AttachmentInput>,
) -> Result<Vec<Attachment>, String> {
    let mut total = 0;
    let mut out = Vec::with_capacity(inputs.len());
    for input in inputs {
        let name = input.name.trim().to_string();
        if name.is_empty() {
            return Err("Every attachment needs a name".into());
        }
        let content_type = input
            .content_type
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| crate::util::mime_from_filename(&name));
        let (content, size) = match (input.base64, input.blob_id) {
            (Some(encoded), None) => {
                let bytes = decode(&name, &encoded, MAX_ATTACHMENT_BYTES - total)?;
                let size = bytes.len();
                (AttachmentContent::Bytes(bytes), size)
            }
            (None, Some(blob_id)) => {
                let size = client
                    .download_blob(&blob_id)
                    .await
                    .map_err(|e| format!("Attachment '{name}': {e}"))?
                    .len();
                (AttachmentContent::Blob(blob_id), size)
            }
            _ => {
                return Err(format!(
                    "Attachment '{name}': give exactly one of base64 and blobId"
                ));
            }
        };
        total += size;
        if total > MAX_ATTACHMENT_BYTES {
            return Err(over_cap(total));
        }
        out.push(Attachment {
            data: AttachmentData {
                filename: name,
                content_type,
                content,
            },
            size,
        });
    }
    Ok(out)
}

/// Decode base64, refusing before decoding anything that cannot fit in `room`.
pub fn decode(name: &str, encoded: &str, room: usize) -> Result<Vec<u8>, String> {
    let compact: String = encoded
        .chars()
        .filter(|c| !c.is_ascii_whitespace())
        .collect();
    let decoded_len = compact.len() / 4 * 3;
    if decoded_len.saturating_sub(2) > room {
        return Err(over_cap(MAX_ATTACHMENT_BYTES - room + decoded_len));
    }
    base64::engine::general_purpose::STANDARD
        .decode(compact)
        .map_err(|e| format!("Attachment '{name}' is not valid base64: {e}"))
}

fn over_cap(total: usize) -> String {
    format!(
        "Attachments come to {}, over the {} limit for one message. \
         Send the larger files as links instead.",
        human_size(total),
        human_size(MAX_ATTACHMENT_BYTES)
    )
}

/// What a confirmation token vouches for: each attachment's name, type and
/// exact content. A blob's ID pins its content, since JMAP blobs are immutable.
pub fn fingerprint(attachments: &[Attachment]) -> Vec<String> {
    use std::hash::{Hash, Hasher};
    attachments
        .iter()
        .map(|a| {
            let content = match &a.data.content {
                AttachmentContent::Bytes(bytes) => {
                    let mut hasher = std::collections::hash_map::DefaultHasher::new();
                    bytes.hash(&mut hasher);
                    format!("bytes:{:016x}", hasher.finish())
                }
                AttachmentContent::Blob(id) => format!("blob:{id}"),
            };
            format!(
                "attachment\n{}\n{}\n{content}",
                a.data.filename, a.data.content_type
            )
        })
        .collect()
}

/// The attachment lines of a send preview, or nothing when there are none.
pub fn preview(attachments: &[Attachment]) -> String {
    if attachments.is_empty() {
        return String::new();
    }
    let mut out = String::from("\n\nAttachments:");
    for a in attachments {
        out.push_str(&format!(
            "\n  {} ({}, {})",
            a.data.filename,
            a.data.content_type,
            human_size(a.size)
        ));
    }
    out
}

pub fn human_size(bytes: usize) -> String {
    const KB: f64 = 1024.0;
    let b = bytes as f64;
    if b < KB {
        format!("{bytes} B")
    } else if b < KB * KB {
        format!("{:.1} KB", b / KB)
    } else {
        format!("{:.1} MB", b / (KB * KB))
    }
}
