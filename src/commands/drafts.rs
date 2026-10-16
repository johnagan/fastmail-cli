use crate::{
    jmap::{DraftEdit, EmailQuery, authenticated_client},
    models::Output,
    util,
};
use anyhow::{Result, ensure};
use clap::Subcommand;
use serde_json::json;

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
pub enum DraftCommands {
    List {
        #[arg(long, default_value = "50")]
        limit: u32,
    },
    Get {
        id: String,
    },
    Update {
        id: String,
        #[arg(long)]
        to: Option<String>,
        #[arg(long)]
        cc: Option<String>,
        #[arg(long)]
        bcc: Option<String>,
        #[arg(long)]
        from: Option<String>,
        #[arg(long)]
        subject: Option<String>,
        #[arg(long)]
        body: Option<String>,
        #[arg(long, conflicts_with = "html_file")]
        html_body: Option<String>,
        #[arg(long)]
        html_file: Option<String>,
        /// Replace regular attachments with these files
        #[arg(short = 'a', long = "attachment", conflicts_with = "clear_attachments")]
        attachments: Vec<String>,
        #[arg(long = "attachment-manifest", conflicts_with = "clear_attachments")]
        attachment_manifests: Vec<String>,
        #[arg(long)]
        clear_attachments: bool,
    },
    Send {
        id: String,
    },
    /// Move a draft to Trash
    #[command(alias = "discard")]
    Delete {
        id: String,
        #[arg(short = 'y', long)]
        yes: bool,
    },
}

pub async fn drafts(command: DraftCommands) -> Result<()> {
    if matches!(&command, DraftCommands::Delete { yes: false, .. }) {
        anyhow::bail!("Deletion requires -y / --yes to confirm");
    }
    let mut client = authenticated_client().await?;
    let result = match command {
        DraftCommands::List { limit } => serde_json::to_value(
            client
                .query_emails(EmailQuery {
                    filter: json!({"hasKeyword": "$draft"}),
                    ..EmailQuery::first(limit)
                })
                .await?
                .emails,
        )?,
        DraftCommands::Get { id } => {
            let draft = client.get_email(&id).await?;
            ensure!(draft.is_draft(), "Message {id} is not a draft");
            serde_json::to_value(draft)?
        }
        DraftCommands::Update {
            id,
            to,
            cc,
            bcc,
            from,
            subject,
            body,
            html_body,
            html_file,
            attachments,
            attachment_manifests,
            clear_attachments,
        } => {
            let html_body = util::resolve_html(html_body, html_file)?;
            let replacing =
                clear_attachments || !attachments.is_empty() || !attachment_manifests.is_empty();
            ensure!(
                to.is_some()
                    || cc.is_some()
                    || bcc.is_some()
                    || from.is_some()
                    || subject.is_some()
                    || body.is_some()
                    || html_body.is_some()
                    || replacing,
                "Provide at least one draft change"
            );
            let attachments = if replacing {
                Some(util::load_attachments(&attachments, &attachment_manifests)?)
            } else {
                None
            };
            let draft = client.get_email(&id).await?;
            let new_id = client
                .replace_draft(
                    &draft,
                    DraftEdit {
                        to: to.as_deref().map(util::parse_addresses),
                        cc: cc.as_deref().map(util::parse_addresses),
                        bcc: bcc.as_deref().map(util::parse_addresses),
                        from,
                        subject,
                        body,
                        html_body,
                        attachments,
                    },
                )
                .await?;
            json!({"id": new_id, "replacedId": id})
        }
        DraftCommands::Send { id } => {
            let draft = client.get_email(&id).await?;
            client.send_draft(&draft).await?;
            json!({"id": id, "sent": true})
        }
        DraftCommands::Delete { id, .. } => {
            let draft = client.get_email(&id).await?;
            ensure!(draft.is_draft(), "Message {id} is not a draft");
            let trash = client.find_mailbox("trash").await?;
            client.move_email(&id, &trash.id).await?;
            json!({"id": id, "trashed": true})
        }
    };
    Output::success(result).print();
    Ok(())
}

pub async fn upload_attachment(file: &str, content_type: Option<String>) -> Result<()> {
    let attachment = util::load_attachment(file)?;
    let content_type = content_type.unwrap_or(attachment.content_type);
    let crate::jmap::AttachmentContent::Bytes(bytes) = attachment.content else {
        unreachable!()
    };
    let size = bytes.len();
    let client = authenticated_client().await?;
    let blob = client.upload_blob(bytes, &content_type).await?;
    Output::success(json!({"name": attachment.filename, "contentType": content_type, "blobId": blob, "size": size})).print();
    Ok(())
}
