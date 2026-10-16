//! GraphQL mutation resolvers

use async_graphql::{Context, Object, Result};

use crate::models::EmailAddress;
use crate::util::parse_addresses;

use super::api::*;
use super::attachments::{self, AttachmentInput, GqlUploadedAttachment};
use super::types::*;

pub struct MutationRoot;

#[derive(async_graphql::InputObject)]
pub struct MailboxUpdateInput {
    pub name: Option<String>,
    pub parent_id: Option<String>,
    /// Move to the root instead of a parent mailbox.
    pub root: Option<bool>,
    pub sort_order: Option<u32>,
    pub is_subscribed: Option<bool>,
}

#[derive(async_graphql::InputObject)]
pub struct VacationResponseInput {
    pub is_enabled: Option<bool>,
    pub subject: Option<String>,
    pub text_body: Option<String>,
    pub html_body: Option<String>,
    pub from_date: Option<String>,
    pub to_date: Option<String>,
    pub clear_from: Option<bool>,
    pub clear_to: Option<bool>,
}

#[Object]
#[allow(clippy::too_many_arguments)]
impl MutationRoot {
    async fn empty_trash(&self, ctx: &Context<'_>) -> Result<SetResult> {
        let mut client = ctx.data::<super::SharedClient>()?.lock().await;
        Ok(client.empty_trash().await?.into())
    }
    /// Bulk keyword/membership patches. False keyword values remove the keyword.
    async fn patch_emails(
        &self,
        ctx: &Context<'_>,
        ids: Vec<String>,
        keywords: Option<async_graphql::Json<std::collections::HashMap<String, bool>>>,
        add_mailboxes: Option<Vec<String>>,
        remove_mailboxes: Option<Vec<String>>,
        move_to: Option<String>,
    ) -> Result<SetResult> {
        let client = ctx.data::<super::SharedClient>()?.lock().await;
        Ok(client
            .patch_emails(
                &ids,
                &keywords.map(|v| v.0).unwrap_or_default(),
                &add_mailboxes.unwrap_or_default(),
                &remove_mailboxes.unwrap_or_default(),
                move_to.as_deref(),
            )
            .await?
            .into())
    }

    /// Permanently destroy messages, returning per-record success/failure details.
    async fn destroy_emails(&self, ctx: &Context<'_>, ids: Vec<String>) -> Result<SetResult> {
        let client = ctx.data::<super::SharedClient>()?.lock().await;
        Ok(client.destroy_emails(&ids).await?.into())
    }

    async fn update_identity(
        &self,
        ctx: &Context<'_>,
        id: String,
        patch: async_graphql::Json<serde_json::Value>,
    ) -> Result<SetResult> {
        let client = ctx.data::<super::SharedClient>()?.lock().await;
        Ok(client.update_identity(&id, patch.0).await?.into())
    }

    async fn cancel_submissions(&self, ctx: &Context<'_>, ids: Vec<String>) -> Result<SetResult> {
        let client = ctx.data::<super::SharedClient>()?.lock().await;
        Ok(client.cancel_submission(&ids).await?.into())
    }

    async fn copy_emails(
        &self,
        ctx: &Context<'_>,
        from_account_id: String,
        emails: async_graphql::Json<serde_json::Value>,
        #[graphql(default = false)] destroy_original: bool,
    ) -> Result<SetResult> {
        let client = ctx.data::<super::SharedClient>()?.lock().await;
        Ok(client
            .copy_emails(&from_account_id, emails.0, destroy_original)
            .await?
            .into())
    }

    async fn parse_email(&self, ctx: &Context<'_>, base64: String) -> Result<ParseResult> {
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD.decode(base64)?;
        let client = ctx.data::<super::SharedClient>()?.lock().await;
        Ok(client.parse_email(bytes).await?.into())
    }

    /// Native PushSubscription/set arguments, including verificationCode updates.
    async fn set_push_subscriptions(
        &self,
        ctx: &Context<'_>,
        arguments: async_graphql::Json<serde_json::Value>,
    ) -> Result<SetResult> {
        let client = ctx.data::<super::SharedClient>()?.lock().await;
        Ok(client.set_push_subscriptions(arguments.0).await?.into())
    }

    async fn create_calendar(
        &self,
        ctx: &Context<'_>,
        href: String,
        name: String,
        description: Option<String>,
        color: Option<String>,
    ) -> Result<DavWriteResult> {
        dav(ctx, false)?
            .create_calendar(&href, &name, description.as_deref(), color.as_deref())
            .await?;
        Ok(DavWriteResult {
            href,
            etag: None,
            success: true,
        })
    }

    async fn update_calendar(
        &self,
        ctx: &Context<'_>,
        href: String,
        name: Option<String>,
        description: Option<String>,
        color: Option<String>,
    ) -> Result<DavWriteResult> {
        dav(ctx, false)?
            .update_calendar(
                &href,
                name.as_deref(),
                description.as_deref(),
                color.as_deref(),
            )
            .await?;
        Ok(DavWriteResult {
            href,
            etag: None,
            success: true,
        })
    }

    async fn delete_calendar(&self, ctx: &Context<'_>, href: String) -> Result<DavWriteResult> {
        let client = dav(ctx, false)?;
        if !client.calendars().await?.iter().any(|c| c.href == href) {
            return Err("Use a calendar href returned by calendars".into());
        }
        client.delete(&href, None).await?;
        Ok(DavWriteResult {
            href,
            etag: None,
            success: true,
        })
    }

    /// Create an event if absent, or replace it conditionally using its current ETag.
    async fn put_calendar_event(
        &self,
        ctx: &Context<'_>,
        href: String,
        calendar_data: String,
        etag: Option<String>,
    ) -> Result<DavWriteResult> {
        validate_calendar(&calendar_data)?;
        let etag = dav(ctx, false)?
            .put(
                &href,
                calendar_data.into_bytes(),
                "text/calendar; charset=utf-8",
                etag.as_deref(),
                false,
            )
            .await?;
        Ok(DavWriteResult {
            href,
            etag,
            success: true,
        })
    }

    async fn delete_calendar_event(
        &self,
        ctx: &Context<'_>,
        href: String,
        etag: Option<String>,
    ) -> Result<DavWriteResult> {
        if !href.trim_end_matches('/').ends_with(".ics") {
            return Err("Use an event .ics href".into());
        }
        dav(ctx, false)?.delete(&href, etag.as_deref()).await?;
        Ok(DavWriteResult {
            href,
            etag: None,
            success: true,
        })
    }

    async fn schedule_calendar(
        &self,
        ctx: &Context<'_>,
        outbox: String,
        originator: String,
        recipients: Vec<String>,
        calendar_data: String,
    ) -> Result<Vec<SchedulingResponse>> {
        validate_calendar(&calendar_data)?;
        if !calendar_data
            .lines()
            .any(|line| line.trim_end().starts_with("METHOD:"))
        {
            return Err("Scheduling requires METHOD".into());
        }
        Ok(scheduling_responses(
            dav(ctx, false)?
                .schedule(
                    &outbox,
                    &originator,
                    &recipients,
                    calendar_data.into_bytes(),
                )
                .await?,
        ))
    }

    async fn upload_file(
        &self,
        ctx: &Context<'_>,
        path: String,
        base64: String,
        #[graphql(default = false)] overwrite: bool,
    ) -> Result<DavWriteResult> {
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD.decode(base64)?;
        let etag = dav(ctx, true)?
            .put(
                &crate::dav::DavClient::file_path(&path)?,
                bytes,
                "application/octet-stream",
                None,
                overwrite,
            )
            .await?;
        Ok(DavWriteResult {
            href: path,
            etag,
            success: true,
        })
    }

    async fn transfer_file(
        &self,
        ctx: &Context<'_>,
        source: String,
        destination: String,
        #[graphql(default = false)] copy: bool,
        #[graphql(default = false)] overwrite: bool,
    ) -> Result<DavWriteResult> {
        dav(ctx, true)?
            .transfer(
                &crate::dav::DavClient::file_path(&source)?,
                &crate::dav::DavClient::file_path(&destination)?,
                copy,
                overwrite,
            )
            .await?;
        Ok(DavWriteResult {
            href: destination,
            etag: None,
            success: true,
        })
    }

    async fn create_file_folder(&self, ctx: &Context<'_>, path: String) -> Result<DavWriteResult> {
        dav(ctx, true)?
            .mkdir(&crate::dav::DavClient::file_path(&path)?)
            .await?;
        Ok(DavWriteResult {
            href: path,
            etag: None,
            success: true,
        })
    }

    async fn delete_file(&self, ctx: &Context<'_>, path: String) -> Result<DavWriteResult> {
        dav(ctx, true)?
            .delete(&crate::dav::DavClient::file_path(&path)?, None)
            .await?;
        Ok(DavWriteResult {
            href: path,
            etag: None,
            success: true,
        })
    }
    /// Create a full JSContact Card. Omitted addressBookId uses its memberships or a writable default.
    async fn create_contact_card(
        &self,
        ctx: &Context<'_>,
        card: async_graphql::Json<serde_json::Value>,
        address_book_id: Option<String>,
    ) -> Result<SetResult> {
        let client = ctx.data::<super::SharedClient>()?.lock().await;
        Ok(SetResult::from(
            client
                .create_contact_card(card.0, address_book_id.as_deref())
                .await?,
        ))
    }

    /// Apply a native JMAP PatchObject to any JSContact fields.
    async fn update_contact_card(
        &self,
        ctx: &Context<'_>,
        id: String,
        patch: async_graphql::Json<serde_json::Value>,
    ) -> Result<SetResult> {
        let client = ctx.data::<super::SharedClient>()?.lock().await;
        Ok(SetResult::from(
            client
                .contacts_set(false, serde_json::json!({"update": {id: patch.0}}))
                .await?,
        ))
    }

    /// Native AddressBook/set: create/update/destroy, subscriptions, sharing, default selection.
    async fn set_address_books(
        &self,
        ctx: &Context<'_>,
        arguments: async_graphql::Json<serde_json::Value>,
    ) -> Result<SetResult> {
        let client = ctx.data::<super::SharedClient>()?.lock().await;
        Ok(SetResult::from(
            client.contacts_set(true, arguments.0).await?,
        ))
    }

    /// Copy a creation-ID to card map from a different account.
    async fn copy_contact_cards(
        &self,
        ctx: &Context<'_>,
        from_account_id: String,
        cards: async_graphql::Json<serde_json::Value>,
        #[graphql(default = false)] destroy_original: bool,
    ) -> Result<SetResult> {
        let client = ctx.data::<super::SharedClient>()?.lock().await;
        Ok(SetResult::from(
            client
                .copy_contacts(&from_account_id, cards.0, destroy_original)
                .await?,
        ))
    }

    /// Upload a photo into the contacts account, retaining non-photo media.
    async fn set_contact_photo(
        &self,
        ctx: &Context<'_>,
        id: String,
        base64: String,
        content_type: String,
    ) -> Result<SetResult> {
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD.decode(base64)?;
        let client = ctx.data::<super::SharedClient>()?.lock().await;
        Ok(SetResult::from(
            client.set_contact_photo(&id, bytes, &content_type).await?,
        ))
    }

    /// Create an Apple-compatible note with text, HTML, and optional attachments.
    async fn create_note(
        &self,
        ctx: &Context<'_>,
        title: String,
        body: Option<String>,
        html_body: Option<String>,
        attachments: Option<Vec<AttachmentInput>>,
    ) -> Result<CreatedRecord> {
        if body.is_none() && html_body.is_none() {
            return Err("Provide body or htmlBody".into());
        }
        let client = ctx.data::<super::SharedClient>()?.lock().await;
        let attachments = attachments::resolve(&client, attachments.unwrap_or_default(), false)
            .await
            .map_err(async_graphql::Error::new)?;
        Ok(CreatedRecord::from(
            client
                .create_rich_note(crate::jmap::NoteEdit {
                    title: Some(title),
                    body,
                    html_body,
                    attachments: Some(attachments.into_iter().map(|a| a.data).collect()),
                })
                .await?,
        ))
    }

    /// Replace a note after saving its new content; returns a new message ID.
    /// Omit attachments to preserve them; pass [] to remove regular attachments.
    async fn update_note(
        &self,
        ctx: &Context<'_>,
        id: String,
        title: Option<String>,
        body: Option<String>,
        html_body: Option<String>,
        attachments: Option<Vec<AttachmentInput>>,
    ) -> Result<CreatedRecord> {
        let client = ctx.data::<super::SharedClient>()?.lock().await;
        let attachments = match attachments {
            Some(inputs) => Some(
                attachments::resolve(&client, inputs, false)
                    .await
                    .map_err(async_graphql::Error::new)?
                    .into_iter()
                    .map(|a| a.data)
                    .collect(),
            ),
            None => None,
        };
        Ok(CreatedRecord::from(
            client
                .update_rich_note(
                    &id,
                    crate::jmap::NoteEdit {
                        title,
                        body,
                        html_body,
                        attachments,
                    },
                )
                .await?,
        ))
    }

    async fn delete_note(&self, ctx: &Context<'_>, id: String) -> Result<GqlStatus> {
        let client = ctx.data::<super::SharedClient>()?.lock().await;
        client.delete_note(&id).await?;
        Ok(GqlStatus {
            success: true,
            message: Some("Note deleted.".into()),
            error: None,
        })
    }
    /// Create a mailbox. Parent is a mailbox ID.
    async fn create_mailbox(
        &self,
        ctx: &Context<'_>,
        name: String,
        parent_id: Option<String>,
        role: Option<String>,
    ) -> Result<CreatedRecord> {
        let mut properties = serde_json::json!({"name": name});
        if let Some(parent) = parent_id {
            properties["parentId"] = serde_json::json!(parent);
        }
        if let Some(role) = role {
            properties["role"] = serde_json::json!(role);
        }
        let mut client = ctx.data::<super::SharedClient>()?.lock().await;
        Ok(client.create_mailbox(properties).await?.into())
    }

    /// Rename, reparent, subscribe, or reorder a mailbox.
    async fn update_mailbox(
        &self,
        ctx: &Context<'_>,
        id: String,
        input: MailboxUpdateInput,
    ) -> Result<SetResult> {
        let mut p = serde_json::json!({});
        if input.root == Some(true) && input.parent_id.is_some() {
            return Err("Use root or parentId, not both".into());
        }
        if let Some(name) = input.name {
            p["name"] = serde_json::json!(name);
        }
        if let Some(parent) = input.parent_id {
            p["parentId"] = serde_json::json!(parent);
        }
        if input.root == Some(true) {
            p["parentId"] = serde_json::Value::Null;
        }
        if let Some(order) = input.sort_order {
            p["sortOrder"] = serde_json::json!(order);
        }
        if let Some(subscribed) = input.is_subscribed {
            p["isSubscribed"] = serde_json::json!(subscribed);
        }
        if p.as_object().unwrap().is_empty() {
            return Err("Provide at least one mailbox change".into());
        }
        let mut client = ctx.data::<super::SharedClient>()?.lock().await;
        Ok(client.update_mailbox(&id, p).await?.into())
    }

    /// Delete a mailbox. Nonempty mailboxes are rejected unless removeEmails is true.
    async fn delete_mailbox(
        &self,
        ctx: &Context<'_>,
        id: String,
        #[graphql(default = false)] remove_emails: bool,
    ) -> Result<SetResult> {
        let mut client = ctx.data::<super::SharedClient>()?.lock().await;
        Ok(client.delete_mailbox(&id, remove_emails).await?.into())
    }

    /// Update the singleton vacation response, preserving omitted settings.
    async fn set_vacation_response(
        &self,
        ctx: &Context<'_>,
        input: VacationResponseInput,
    ) -> Result<SetResult> {
        if (input.clear_from == Some(true) && input.from_date.is_some())
            || (input.clear_to == Some(true) && input.to_date.is_some())
        {
            return Err("Cannot set and clear the same vacation date".into());
        }
        let mut p = serde_json::json!({});
        if let Some(enabled) = input.is_enabled {
            p["isEnabled"] = serde_json::json!(enabled);
        }
        for (key, value) in [
            ("subject", input.subject),
            ("textBody", input.text_body),
            ("htmlBody", input.html_body),
            ("fromDate", input.from_date),
            ("toDate", input.to_date),
        ] {
            if let Some(value) = value {
                p[key] = serde_json::json!(value);
            }
        }
        if input.clear_from == Some(true) {
            p["fromDate"] = serde_json::Value::Null;
        }
        if input.clear_to == Some(true) {
            p["toDate"] = serde_json::Value::Null;
        }
        if p.as_object().unwrap().is_empty() {
            return Err("Provide at least one vacation response change".into());
        }
        let client = ctx.data::<super::SharedClient>()?.lock().await;
        Ok(client.set_vacation_response(p).await?.into())
    }

    /// Import a base64-encoded RFC 5322 message into a mailbox without sending it.
    async fn import_email(
        &self,
        ctx: &Context<'_>,
        message_base64: String,
        mailbox_id: String,
        keywords: Option<Vec<String>>,
        received_at: Option<String>,
    ) -> Result<CreatedRecord> {
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD.decode(message_base64)?;
        if bytes.is_empty() {
            return Err("Message is empty".into());
        }
        let keywords = serde_json::Value::Object(
            keywords
                .unwrap_or_default()
                .into_iter()
                .map(|k| (k, serde_json::json!(true)))
                .collect(),
        );
        let client = ctx.data::<super::SharedClient>()?.lock().await;
        Ok(CreatedRecord::from(
            client
                .import_email(bytes, &mailbox_id, keywords, received_at.as_deref())
                .await?,
        ))
    }
    /// Star or unstar an email, preserving all other keywords.
    async fn star_email(
        &self,
        ctx: &Context<'_>,
        email_id: String,
        starred: bool,
    ) -> Result<GqlStatus> {
        let client = ctx.data::<super::SharedClient>()?.lock().await;
        client.star_email(&email_id, starred).await?;
        Ok(GqlStatus {
            success: true,
            message: Some(
                if starred {
                    "Email starred."
                } else {
                    "Email unstarred."
                }
                .into(),
            ),
            error: None,
        })
    }
    /// Compose and send a new email. ALWAYS use action=PREVIEW first, show the user, then CONFIRM or DRAFT with the confirmation_token from the preview.
    async fn send_email(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "PREVIEW first, then CONFIRM to send or DRAFT to save")]
        action: SendAction,
        #[graphql(desc = "Recipient email address(es), comma-separated")] to: String,
        #[graphql(desc = "Email subject line")] subject: String,
        #[graphql(desc = "Email body text")] body: String,
        #[graphql(desc = "CC recipients, comma-separated")] cc: Option<String>,
        #[graphql(desc = "BCC recipients (hidden), comma-separated")] bcc: Option<String>,
        #[graphql(desc = "Send from a specific identity/email address")] from: Option<String>,
        #[graphql(desc = "HTML body content (alternative to plain text)")] html_body: Option<
            String,
        >,
        #[graphql(desc = "Files to attach, as base64 or by blobId")] attachments: Option<
            Vec<AttachmentInput>,
        >,
        #[graphql(desc = "Token from PREVIEW response — required for CONFIRM/DRAFT")]
        confirmation_token: Option<String>,
    ) -> Result<GqlComposeResult> {
        let to_addrs = parse_addresses(&to);
        let cc_addrs = cc.as_deref().map(parse_addresses).unwrap_or_default();
        let bcc_addrs = bcc.as_deref().map(parse_addresses).unwrap_or_default();
        let nonce_store = ctx.data::<super::types::NonceStore>()?;
        let client = ctx.data::<crate::mcp::graphql::SharedClient>()?;
        let mut client = client.lock().await;
        let attachments = match attachments::resolve(
            &client,
            attachments.unwrap_or_default(),
            matches!(action, SendAction::Preview),
        )
        .await
        {
            Ok(a) => a,
            Err(msg) => return Ok(failed(msg)),
        };
        let attachment_keys = attachments::fingerprint(&attachments);
        let mut params = vec![
            to.as_str(),
            subject.as_str(),
            body.as_str(),
            cc.as_deref().unwrap_or(""),
            bcc.as_deref().unwrap_or(""),
            from.as_deref().unwrap_or(""),
            html_body.as_deref().unwrap_or(""),
        ];
        params.extend(attachment_keys.iter().map(String::as_str));

        if matches!(action, SendAction::Preview) {
            let nonce = super::types::issue_nonce(nonce_store, &params).await;
            let mut body_preview = body.clone();
            if html_body.is_some() {
                body_preview.push_str("\n[HTML body included]");
            }
            body_preview.push_str(&attachments::preview(&attachments));
            let preview =
                format_send_preview(&to_addrs, &cc_addrs, &bcc_addrs, &subject, &body_preview);
            return Ok(GqlComposeResult {
                success: true,
                email_id: None,
                preview: Some(preview),
                confirmation_token: Some(nonce),
                error: None,
            });
        }

        if let Err(msg) =
            super::types::consume_nonce(nonce_store, confirmation_token.as_deref(), &params).await
        {
            return Ok(GqlComposeResult {
                success: false,
                email_id: None,
                preview: None,
                confirmation_token: None,
                error: Some(msg.to_string()),
            });
        }

        let draft = matches!(action, SendAction::Draft);
        match client
            .send_email(
                to_addrs,
                &subject,
                &body,
                None,
                crate::jmap::ComposeParams {
                    cc: cc_addrs,
                    bcc: bcc_addrs,
                    from: from.as_deref(),
                    draft,
                    html_body,
                    attachments: attachments.into_iter().map(|a| a.data).collect(),
                },
            )
            .await
        {
            Ok(email_id) => Ok(GqlComposeResult {
                success: true,
                email_id: Some(email_id),
                preview: None,
                confirmation_token: None,
                error: None,
            }),
            Err(e) => Ok(GqlComposeResult {
                success: false,
                email_id: None,
                preview: None,
                confirmation_token: None,
                error: Some(e.to_string()),
            }),
        }
    }

    /// Reply to an existing email thread. ALWAYS use action=PREVIEW first.
    async fn reply_to_email(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "PREVIEW first, then CONFIRM to send or DRAFT to save")]
        action: SendAction,
        #[graphql(desc = "The email ID to reply to")] email_id: String,
        #[graphql(desc = "Reply body text (your response, without quoting original)")] body: String,
        #[graphql(desc = "Reply to all recipients")] all: Option<bool>,
        #[graphql(desc = "CC recipients, comma-separated")] cc: Option<String>,
        #[graphql(desc = "BCC recipients, comma-separated")] bcc: Option<String>,
        #[graphql(desc = "Send from a specific identity/email address")] from: Option<String>,
        #[graphql(desc = "HTML body content (alternative to plain text)")] html_body: Option<
            String,
        >,
        #[graphql(desc = "Files to attach, as base64 or by blobId")] attachments: Option<
            Vec<AttachmentInput>,
        >,
        #[graphql(desc = "Token from PREVIEW response — required for CONFIRM/DRAFT")]
        confirmation_token: Option<String>,
    ) -> Result<GqlComposeResult> {
        let nonce_store = ctx.data::<super::types::NonceStore>()?;
        let reply_all = all.unwrap_or(false);
        let client = ctx.data::<crate::mcp::graphql::SharedClient>()?;
        let mut client = client.lock().await;
        let attachments = match attachments::resolve(
            &client,
            attachments.unwrap_or_default(),
            matches!(action, SendAction::Preview),
        )
        .await
        {
            Ok(a) => a,
            Err(msg) => return Ok(failed(msg)),
        };
        let attachment_keys = attachments::fingerprint(&attachments);
        let mut params = vec![
            email_id.as_str(),
            body.as_str(),
            if reply_all { "all" } else { "" },
            cc.as_deref().unwrap_or(""),
            bcc.as_deref().unwrap_or(""),
            from.as_deref().unwrap_or(""),
            html_body.as_deref().unwrap_or(""),
        ];
        params.extend(attachment_keys.iter().map(String::as_str));

        let original = client.get_email(&email_id).await?;
        let extra_cc = cc.as_deref().map(parse_addresses).unwrap_or_default();
        let bcc_addrs = bcc.as_deref().map(parse_addresses).unwrap_or_default();

        let subject = if original
            .subject
            .as_ref()
            .is_some_and(|s| s.to_lowercase().starts_with("re:"))
        {
            original.subject.clone().unwrap_or_default()
        } else {
            format!("Re: {}", original.subject.as_deref().unwrap_or(""))
        };

        // Compute the final recipient lists once, up front. Both PREVIEW
        // (for display) and CONFIRM/DRAFT (for the actual send) use these
        // exact values — preview and send can't diverge because they share
        // the same variables, not the same code path.
        let my_email = client.resolve_my_email(from.as_deref()).await;
        let (to_addrs, cc_addrs) = crate::jmap::expand_reply_recipients(
            &original,
            reply_all,
            my_email.as_deref(),
            extra_cc,
        );

        if matches!(action, SendAction::Preview) {
            let nonce = super::types::issue_nonce(nonce_store, &params).await;
            let in_reply_to = original
                .message_id
                .as_ref()
                .and_then(|v| v.first())
                .cloned()
                .unwrap_or_else(|| "(none)".to_string());
            let mut preview = format!(
                "REPLY PREVIEW:\nTo: {}\nCC: {}\nBCC: {}\nSubject: {}\nIn-Reply-To: {}\n\n--- Your Reply ---\n{}",
                format_addrs(&to_addrs),
                if cc_addrs.is_empty() {
                    "(none)".to_string()
                } else {
                    format_addrs(&cc_addrs)
                },
                if bcc_addrs.is_empty() {
                    "(none)".to_string()
                } else {
                    format_addrs(&bcc_addrs)
                },
                subject,
                in_reply_to,
                body
            );
            if html_body.is_some() {
                preview.push_str("\n[HTML body included]");
            }
            preview.push_str(&attachments::preview(&attachments));
            return Ok(GqlComposeResult {
                success: true,
                email_id: None,
                preview: Some(preview),
                confirmation_token: Some(nonce),
                error: None,
            });
        }

        if let Err(msg) =
            super::types::consume_nonce(nonce_store, confirmation_token.as_deref(), &params).await
        {
            return Ok(GqlComposeResult {
                success: false,
                email_id: None,
                preview: None,
                confirmation_token: None,
                error: Some(msg.to_string()),
            });
        }

        let draft = matches!(action, SendAction::Draft);
        match client
            .reply_email(
                &original,
                &body,
                to_addrs,
                crate::jmap::ComposeParams {
                    cc: cc_addrs,
                    bcc: bcc_addrs,
                    from: from.as_deref(),
                    draft,
                    html_body,
                    attachments: attachments.into_iter().map(|a| a.data).collect(),
                },
            )
            .await
        {
            Ok(eid) => Ok(GqlComposeResult {
                success: true,
                email_id: Some(eid),
                preview: None,
                confirmation_token: None,
                error: None,
            }),
            Err(e) => Ok(GqlComposeResult {
                success: false,
                email_id: None,
                preview: None,
                confirmation_token: None,
                error: Some(e.to_string()),
            }),
        }
    }

    /// Forward an email to new recipients. ALWAYS use action=PREVIEW first.
    async fn forward_email(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "PREVIEW first, then CONFIRM to send or DRAFT to save")]
        action: SendAction,
        #[graphql(desc = "The email ID to forward")] email_id: String,
        #[graphql(desc = "Recipient email address(es), comma-separated")] to: String,
        #[graphql(desc = "Your message to include above forwarded content")] body: Option<String>,
        #[graphql(desc = "CC recipients, comma-separated")] cc: Option<String>,
        #[graphql(desc = "BCC recipients, comma-separated")] bcc: Option<String>,
        #[graphql(desc = "Send from a specific identity/email address")] from: Option<String>,
        #[graphql(desc = "HTML body content (alternative to plain text)")] html_body: Option<
            String,
        >,
        #[graphql(desc = "Files to attach, as base64 or by blobId")] attachments: Option<
            Vec<AttachmentInput>,
        >,
        #[graphql(desc = "Token from PREVIEW response — required for CONFIRM/DRAFT")]
        confirmation_token: Option<String>,
    ) -> Result<GqlComposeResult> {
        let body_str = body.as_deref().unwrap_or("");
        let nonce_store = ctx.data::<super::types::NonceStore>()?;
        let client = ctx.data::<crate::mcp::graphql::SharedClient>()?;
        let mut client = client.lock().await;
        let attachments = match attachments::resolve(
            &client,
            attachments.unwrap_or_default(),
            matches!(action, SendAction::Preview),
        )
        .await
        {
            Ok(a) => a,
            Err(msg) => return Ok(failed(msg)),
        };
        let attachment_keys = attachments::fingerprint(&attachments);
        let mut params = vec![
            email_id.as_str(),
            to.as_str(),
            body_str,
            cc.as_deref().unwrap_or(""),
            bcc.as_deref().unwrap_or(""),
            from.as_deref().unwrap_or(""),
            html_body.as_deref().unwrap_or(""),
        ];
        params.extend(attachment_keys.iter().map(String::as_str));

        let original = client.get_email(&email_id).await?;
        let to_addrs = parse_addresses(&to);
        let cc_addrs = cc.as_deref().map(parse_addresses).unwrap_or_default();
        let bcc_addrs = bcc.as_deref().map(parse_addresses).unwrap_or_default();

        let subject = if original
            .subject
            .as_ref()
            .is_some_and(|s| s.to_lowercase().starts_with("fwd:"))
        {
            original.subject.clone().unwrap_or_default()
        } else {
            format!("Fwd: {}", original.subject.as_deref().unwrap_or(""))
        };

        if matches!(action, SendAction::Preview) {
            let nonce = super::types::issue_nonce(nonce_store, &params).await;
            let original_body = original.text_content().unwrap_or_default();
            let sender = format_addrs(&original.from.clone().unwrap_or_default());

            let mut preview = format!(
                "FORWARD PREVIEW:\nTo: {}\nCC: {}\nBCC: {}\nSubject: {}\nForwarding from: {}\n\n--- Your Message ---\n{}\n\n--- Forwarded ---\nFrom: {}\nDate: {}\nSubject: {}\n\n{}",
                format_addrs(&to_addrs),
                if cc_addrs.is_empty() {
                    "(none)".to_string()
                } else {
                    format_addrs(&cc_addrs)
                },
                if bcc_addrs.is_empty() {
                    "(none)".to_string()
                } else {
                    format_addrs(&bcc_addrs)
                },
                subject,
                sender,
                body_str,
                sender,
                original.received_at.as_deref().unwrap_or("unknown"),
                original.subject.as_deref().unwrap_or(""),
                original_body,
            );
            if html_body.is_some() {
                preview.push_str("\n[HTML body included]");
            }
            preview.push_str(&attachments::preview(&attachments));
            return Ok(GqlComposeResult {
                success: true,
                email_id: None,
                preview: Some(preview),
                confirmation_token: Some(nonce),
                error: None,
            });
        }

        if let Err(msg) =
            super::types::consume_nonce(nonce_store, confirmation_token.as_deref(), &params).await
        {
            return Ok(GqlComposeResult {
                success: false,
                email_id: None,
                preview: None,
                confirmation_token: None,
                error: Some(msg.to_string()),
            });
        }

        let draft = matches!(action, SendAction::Draft);
        match client
            .forward_email(
                &original,
                to_addrs,
                body_str,
                crate::jmap::ComposeParams {
                    cc: cc_addrs,
                    bcc: bcc_addrs,
                    from: from.as_deref(),
                    draft,
                    html_body,
                    attachments: attachments.into_iter().map(|a| a.data).collect(),
                },
            )
            .await
        {
            Ok(eid) => Ok(GqlComposeResult {
                success: true,
                email_id: Some(eid),
                preview: None,
                confirmation_token: None,
                error: None,
            }),
            Err(e) => Ok(GqlComposeResult {
                success: false,
                email_id: None,
                preview: None,
                confirmation_token: None,
                error: Some(e.to_string()),
            }),
        }
    }

    /// Put a file on the server without sending anything, and return its
    /// `blobId` to attach by. Saves repeating the base64 in both PREVIEW and
    /// CONFIRM, and keeps each request small.
    async fn upload_attachment(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "File name the recipient sees")] name: String,
        #[graphql(desc = "MIME type; inferred from the name when omitted")] content_type: Option<
            String,
        >,
        #[graphql(desc = "The file's bytes, base64-encoded")] base64: String,
    ) -> Result<GqlUploadedAttachment> {
        let (name, content_type) = attachments::name_and_type(name, content_type)?;
        let bytes = attachments::decode(&name, &base64, attachments::MAX_ATTACHMENT_BYTES)?;
        let size = bytes.len() as u64;
        let client = ctx.data::<crate::mcp::graphql::SharedClient>()?;
        let client = client.lock().await;
        let blob_id = client.upload_blob(bytes, &content_type).await?;
        Ok(GqlUploadedAttachment {
            blob_id,
            name,
            content_type,
            size,
        })
    }

    /// Edit a saved draft. Only the fields given change; an empty string clears
    /// to/cc/bcc. A new `body` replaces any HTML part unless `htmlBody` comes
    /// with it. JMAP emails are immutable, so this saves an edited copy and
    /// removes the original: the returned `emailId` is the draft's new ID, and
    /// the old one stops resolving.
    async fn update_draft(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "The draft's email ID")] email_id: String,
        #[graphql(desc = "Recipients, comma-separated")] to: Option<String>,
        #[graphql(desc = "CC recipients, comma-separated")] cc: Option<String>,
        #[graphql(desc = "BCC recipients, comma-separated")] bcc: Option<String>,
        #[graphql(desc = "Identity email address to send as")] from: Option<String>,
        #[graphql(desc = "Subject line")] subject: Option<String>,
        #[graphql(desc = "Plain-text body")] body: Option<String>,
        #[graphql(desc = "HTML body")] html_body: Option<String>,
        #[graphql(
            desc = "Replaces the draft's attachments; pass an existing one's blobId to keep it"
        )]
        attachments: Option<Vec<AttachmentInput>>,
    ) -> Result<GqlComposeResult> {
        let client = ctx.data::<crate::mcp::graphql::SharedClient>()?;
        let client = client.lock().await;
        let draft = client.get_email(&email_id).await?;
        let attachments = match attachments {
            Some(inputs) => match attachments::resolve(&client, inputs, true).await {
                Ok(a) => Some(a.into_iter().map(|a| a.data).collect()),
                Err(msg) => return Ok(failed(msg)),
            },
            None => None,
        };

        let edit = crate::jmap::DraftEdit {
            to: to.as_deref().map(parse_addresses),
            cc: cc.as_deref().map(parse_addresses),
            bcc: bcc.as_deref().map(parse_addresses),
            from,
            subject,
            body,
            html_body,
            attachments,
        };
        Ok(match client.replace_draft(&draft, edit).await {
            Ok(id) => GqlComposeResult {
                success: true,
                email_id: Some(id),
                preview: None,
                confirmation_token: None,
                error: None,
            },
            Err(e) => GqlComposeResult {
                success: false,
                email_id: None,
                preview: None,
                confirmation_token: None,
                error: Some(e.to_string()),
            },
        })
    }

    /// Send a saved draft as it stands. ALWAYS use action=PREVIEW first, show the user, then CONFIRM with the confirmation_token.
    async fn send_draft(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "PREVIEW first, then CONFIRM to send")] action: SendDraftAction,
        #[graphql(desc = "The draft's email ID")] email_id: String,
        #[graphql(desc = "Token from PREVIEW response — required for CONFIRM")]
        confirmation_token: Option<String>,
    ) -> Result<GqlComposeResult> {
        let nonce_store = ctx.data::<super::types::NonceStore>()?;
        // An email's content can't change under its ID, so the ID alone pins
        // exactly what was previewed.
        let params = ["send_draft", email_id.as_str()];
        let client = ctx.data::<crate::mcp::graphql::SharedClient>()?;
        let mut client = client.lock().await;
        let draft = client.get_email(&email_id).await?;

        let failure = |msg: String| GqlComposeResult {
            success: false,
            email_id: None,
            preview: None,
            confirmation_token: None,
            error: Some(msg),
        };
        if !draft.is_draft() {
            return Ok(failure(format!("Email {email_id} is not a draft")));
        }

        if matches!(action, SendDraftAction::Preview) {
            let nonce = super::types::issue_nonce(nonce_store, &params).await;
            let mut preview = format!(
                "DRAFT PREVIEW:\nFrom: {}\nTo: {}\nCC: {}\nBCC: {}\nSubject: {}\n\n--- Body ---\n{}",
                format_addrs(draft.from.as_deref().unwrap_or_default()),
                format_addrs(draft.to.as_deref().unwrap_or_default()),
                format_addrs(draft.cc.as_deref().unwrap_or_default()),
                format_addrs(draft.bcc.as_deref().unwrap_or_default()),
                draft.subject.as_deref().unwrap_or(""),
                draft.text_content().unwrap_or_default(),
            );
            let attachments: Vec<String> = draft
                .attachments
                .iter()
                .flatten()
                .map(|a| {
                    format!(
                        "{} ({})",
                        a.name.as_deref().unwrap_or("(unnamed)"),
                        attachments::human_size(a.size as usize)
                    )
                })
                .collect();
            if !attachments.is_empty() {
                preview.push_str(&format!("\n\nAttachments: {}", attachments.join(", ")));
            }
            preview.push_str("\n\nTo send: use action=CONFIRM.");
            return Ok(GqlComposeResult {
                success: true,
                email_id: None,
                preview: Some(preview),
                confirmation_token: Some(nonce),
                error: None,
            });
        }

        if let Err(msg) =
            super::types::consume_nonce(nonce_store, confirmation_token.as_deref(), &params).await
        {
            return Ok(failure(msg.to_string()));
        }

        Ok(match client.send_draft(&draft).await {
            Ok(()) => GqlComposeResult {
                success: true,
                email_id: Some(email_id),
                preview: None,
                confirmation_token: None,
                error: None,
            },
            Err(e) => failure(e.to_string()),
        })
    }

    /// Discard a draft by moving it to Trash, where it can still be recovered.
    async fn delete_draft(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "The draft's email ID")] email_id: String,
    ) -> Result<GqlStatus> {
        let client = ctx.data::<crate::mcp::graphql::SharedClient>()?;
        let mut client = client.lock().await;
        let draft = client.get_email(&email_id).await?;
        if !draft.is_draft() {
            return Ok(GqlStatus {
                success: false,
                message: None,
                error: Some(format!("Email {email_id} is not a draft")),
            });
        }
        let trash = client.find_mailbox("trash").await?;
        Ok(match client.move_email(&email_id, &trash.id).await {
            Ok(()) => GqlStatus {
                success: true,
                message: Some(format!(
                    "Moved draft \"{}\" to {}",
                    draft.subject.as_deref().unwrap_or("(no subject)"),
                    trash.name
                )),
                error: None,
            },
            Err(e) => GqlStatus {
                success: false,
                message: None,
                error: Some(e.to_string()),
            },
        })
    }

    /// Move an email to a different mailbox/folder.
    async fn move_email(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "The email ID to move")] email_id: String,
        #[graphql(desc = "Target mailbox name (e.g., 'Archive', 'Trash') or role")]
        target_mailbox: String,
    ) -> Result<GqlStatus> {
        let client = ctx.data::<crate::mcp::graphql::SharedClient>()?;
        let mut client = client.lock().await;

        let email = client.get_email(&email_id).await?;
        let target = client.find_mailbox(&target_mailbox).await?;

        match client.move_email(&email_id, &target.id).await {
            Ok(()) => Ok(GqlStatus {
                success: true,
                message: Some(format!(
                    "Moved \"{}\" to {}",
                    email.subject.as_deref().unwrap_or("(no subject)"),
                    target.name
                )),
                error: None,
            }),
            Err(e) => Ok(GqlStatus {
                success: false,
                message: None,
                error: Some(e.to_string()),
            }),
        }
    }

    /// Mark an email as read or unread.
    async fn mark_as_read(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "The email ID")] email_id: String,
        #[graphql(desc = "true to mark read, false to mark unread (default: true)")] read: Option<
            bool,
        >,
    ) -> Result<GqlStatus> {
        let client = ctx.data::<crate::mcp::graphql::SharedClient>()?;
        let client = client.lock().await;
        let read = read.unwrap_or(true);

        let email = client.get_email(&email_id).await?;
        let mut keywords = email.keywords.clone();
        if read {
            keywords.insert("$seen".to_string(), true);
        } else {
            keywords.remove("$seen");
        }

        match client.set_keywords(&email_id, keywords).await {
            Ok(()) => {
                let status = if read { "read" } else { "unread" };
                Ok(GqlStatus {
                    success: true,
                    message: Some(format!(
                        "Marked \"{}\" as {status}",
                        email.subject.as_deref().unwrap_or("(no subject)")
                    )),
                    error: None,
                })
            }
            Err(e) => Ok(GqlStatus {
                success: false,
                message: None,
                error: Some(e.to_string()),
            }),
        }
    }

    /// Mark an email as spam. Moves to Junk AND trains the spam filter. MUST use action=PREVIEW first.
    async fn mark_as_spam(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "The email ID")] email_id: String,
        #[graphql(desc = "PREVIEW first, then CONFIRM")] action: SpamAction,
    ) -> Result<GqlStatus> {
        let client = ctx.data::<crate::mcp::graphql::SharedClient>()?;
        let mut client = client.lock().await;

        let email = client.get_email(&email_id).await?;

        if matches!(action, SpamAction::Preview) {
            let sender = email
                .from
                .as_ref()
                .and_then(|f| f.first())
                .map(|a| a.to_string())
                .unwrap_or_else(|| "(unknown)".to_string());
            return Ok(GqlStatus {
                success: true,
                message: Some(format!(
                    "SPAM PREVIEW — This will:\n1. Move to Junk folder\n2. Train spam filter\n\nEmail: \"{}\"\nFrom: {}\n\nUse action=CONFIRM to proceed.",
                    email.subject.as_deref().unwrap_or("(no subject)"),
                    sender
                )),
                error: None,
            });
        }

        match client.mark_spam(&email_id).await {
            Ok(()) => Ok(GqlStatus {
                success: true,
                message: Some(format!(
                    "Marked as spam: \"{}\"",
                    email.subject.as_deref().unwrap_or("(no subject)")
                )),
                error: None,
            }),
            Err(e) => Ok(GqlStatus {
                success: false,
                message: None,
                error: Some(e.to_string()),
            }),
        }
    }

    /// Create a new masked email address.
    async fn create_masked_email(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "The website/domain this masked email is for")] for_domain: Option<String>,
        #[graphql(desc = "A note to remember what this is for")] description: Option<String>,
        #[graphql(desc = "Custom prefix for the email address")] prefix: Option<String>,
        #[graphql(desc = "Deep link to the related record")] url: Option<String>,
        #[graphql(default = false)] pending: bool,
    ) -> Result<GqlMaskedEmail> {
        let client = ctx.data::<crate::mcp::graphql::SharedClient>()?;
        let client = client.lock().await;
        let mut properties =
            serde_json::json!({"state": if pending { "pending" } else { "enabled" }});
        for (key, value) in [
            ("forDomain", for_domain),
            ("description", description),
            ("emailPrefix", prefix),
            ("url", url),
        ] {
            if let Some(value) = value {
                properties[key] = serde_json::json!(value);
            }
        }
        let response = client.create_masked_email_options(properties).await?;
        let masked: crate::models::MaskedEmail =
            serde_json::from_value(response["created"]["new"].clone())?;
        Ok(GqlMaskedEmail::from(masked))
    }

    /// Edit Masked Email metadata; omit fields to preserve them.
    async fn update_masked_email(
        &self,
        ctx: &Context<'_>,
        id: String,
        for_domain: Option<String>,
        description: Option<String>,
        url: Option<String>,
        #[graphql(default = false)] clear_url: bool,
    ) -> Result<SetResult> {
        if clear_url && url.is_some() {
            return Err("Use url or clearUrl, not both".into());
        }
        let mut properties = serde_json::json!({});
        for (key, value) in [
            ("forDomain", for_domain),
            ("description", description),
            ("url", url),
        ] {
            if let Some(value) = value {
                properties[key] = serde_json::json!(value);
            }
        }
        if clear_url {
            properties["url"] = serde_json::Value::Null;
        }
        if properties.as_object().unwrap().is_empty() {
            return Err("Provide at least one metadata change".into());
        }
        let client = ctx.data::<super::SharedClient>()?.lock().await;
        Ok(SetResult::from(
            client.update_masked_email_options(&id, properties).await?,
        ))
    }

    /// Enable a disabled masked email address.
    async fn enable_masked_email(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "The masked email ID")] id: String,
    ) -> Result<GqlStatus> {
        let client = ctx.data::<crate::mcp::graphql::SharedClient>()?;
        let client = client.lock().await;
        match client
            .update_masked_email(&id, Some("enabled"), None, None)
            .await
        {
            Ok(()) => Ok(GqlStatus {
                success: true,
                message: Some(format!("Masked email {id} enabled.")),
                error: None,
            }),
            Err(e) => Ok(GqlStatus {
                success: false,
                message: None,
                error: Some(e.to_string()),
            }),
        }
    }

    /// Disable a masked email address. Emails sent to it will be rejected.
    async fn disable_masked_email(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "The masked email ID")] id: String,
    ) -> Result<GqlStatus> {
        let client = ctx.data::<crate::mcp::graphql::SharedClient>()?;
        let client = client.lock().await;
        match client
            .update_masked_email(&id, Some("disabled"), None, None)
            .await
        {
            Ok(()) => Ok(GqlStatus {
                success: true,
                message: Some(format!("Masked email {id} disabled.")),
                error: None,
            }),
            Err(e) => Ok(GqlStatus {
                success: false,
                message: None,
                error: Some(e.to_string()),
            }),
        }
    }

    /// Permanently delete a masked email address. Cannot be undone!
    async fn delete_masked_email(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "The masked email ID")] id: String,
    ) -> Result<GqlStatus> {
        let client = ctx.data::<crate::mcp::graphql::SharedClient>()?;
        let client = client.lock().await;
        match client
            .update_masked_email(&id, Some("deleted"), None, None)
            .await
        {
            Ok(()) => Ok(GqlStatus {
                success: true,
                message: Some(format!("Masked email {id} deleted.")),
                error: None,
            }),
            Err(e) => Ok(GqlStatus {
                success: false,
                message: None,
                error: Some(e.to_string()),
            }),
        }
    }

    /// Create a contact using JMAP when available, otherwise CardDAV.
    async fn create_contact(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Full name")] name: String,
        #[graphql(desc = "Email address")] email: Option<String>,
        #[graphql(desc = "Phone number")] phone: Option<String>,
        #[graphql(desc = "Organization/company")] organization: Option<String>,
        #[graphql(desc = "Job title")] title: Option<String>,
        #[graphql(desc = "Notes")] notes: Option<String>,
        #[graphql(desc = "JMAP address-book ID; omitted uses a writable default")] address_book_id: Option<String>,
    ) -> Result<GqlContact> {
        let (client, emails, phones) = build_contact_context(ctx, email, phone).await?;

        let contact = client
            .create_in(
                &crate::carddav::ContactFields {
                    name: Some(&name),
                    emails: Some(&emails),
                    phones: Some(&phones),
                    organization: organization.as_deref(),
                    title: title.as_deref(),
                    notes: notes.as_deref(),
                },
                address_book_id.as_deref(),
            )
            .await?;
        Ok(GqlContact::from(contact))
    }

    /// Update a contact. Only provided fields are changed; others are preserved.
    async fn update_contact(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Contact ID returned by contacts")] id: String,
        #[graphql(desc = "New full name")] name: Option<String>,
        #[graphql(desc = "New email address (replaces existing)")] email: Option<String>,
        #[graphql(desc = "New phone number (replaces existing)")] phone: Option<String>,
        #[graphql(desc = "New organization/company")] organization: Option<String>,
        #[graphql(desc = "New job title")] title: Option<String>,
        #[graphql(desc = "New notes")] notes: Option<String>,
    ) -> Result<GqlContact> {
        let has_email = email.is_some();
        let has_phone = phone.is_some();
        let (client, emails, phones) = build_contact_context(ctx, email, phone).await?;

        let emails_ref = has_email.then_some(emails.as_slice());
        let phones_ref = has_phone.then_some(phones.as_slice());

        let contact = client
            .update(
                &id,
                &crate::carddav::ContactFields {
                    name: name.as_deref(),
                    emails: emails_ref,
                    phones: phones_ref,
                    organization: organization.as_deref(),
                    title: title.as_deref(),
                    notes: notes.as_deref(),
                },
            )
            .await?;
        Ok(GqlContact::from(contact))
    }

    /// Delete a contact using the preferred backend. Cannot be undone!
    async fn delete_contact(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Contact ID returned by contacts")] id: String,
    ) -> Result<GqlStatus> {
        let (client, _, _) = build_contact_context(ctx, None, None).await?;
        match client.delete(&id).await {
            Ok(()) => Ok(GqlStatus {
                success: true,
                message: Some(format!("Contact {id} deleted.")),
                error: None,
            }),
            Err(e) => Ok(GqlStatus {
                success: false,
                message: None,
                error: Some(e.to_string()),
            }),
        }
    }
}

// ============ Contacts helpers ============

async fn build_contact_context(
    ctx: &Context<'_>,
    email: Option<String>,
    phone: Option<String>,
) -> async_graphql::Result<(
    crate::contacts::ContactsClient,
    Vec<crate::carddav::ContactEmail>,
    Vec<crate::carddav::ContactPhone>,
)> {
    let creds = ctx.data::<super::CardDavCreds>()?;
    let client = crate::contacts::ContactsClient::for_request(
        ctx.data::<super::SharedClient>()?.clone(),
        creds.username.clone(),
        creds.app_password.clone(),
    )
    .await?;

    let emails: Vec<crate::carddav::ContactEmail> = email
        .map(|e| {
            e.split(',')
                .filter(|addr| !addr.trim().is_empty())
                .map(|addr| crate::carddav::ContactEmail {
                    email: addr.trim().to_string(),
                    label: None,
                })
                .collect()
        })
        .unwrap_or_default();

    let phones: Vec<crate::carddav::ContactPhone> = phone
        .map(|p| {
            p.split(',')
                .filter(|num| !num.trim().is_empty())
                .map(|num| crate::carddav::ContactPhone {
                    number: num.trim().to_string(),
                    label: None,
                })
                .collect()
        })
        .unwrap_or_default();

    Ok((client, emails, phones))
}

// ============ Formatting helpers (preview only) ============

fn format_addrs(addrs: &[EmailAddress]) -> String {
    if addrs.is_empty() {
        "(none)".to_string()
    } else {
        addrs
            .iter()
            .map(|a| a.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn format_send_preview(
    to: &[EmailAddress],
    cc: &[EmailAddress],
    bcc: &[EmailAddress],
    subject: &str,
    body: &str,
) -> String {
    format!(
        "EMAIL PREVIEW:\nTo: {}\nCC: {}\nBCC: {}\nSubject: {}\n\n--- Body ---\n{}\n\nTo send: use action=CONFIRM. To save draft: use action=DRAFT.",
        format_addrs(to),
        if cc.is_empty() {
            "(none)".to_string()
        } else {
            format_addrs(cc)
        },
        if bcc.is_empty() {
            "(none)".to_string()
        } else {
            format_addrs(bcc)
        },
        subject,
        body
    )
}

fn failed(msg: String) -> GqlComposeResult {
    GqlComposeResult {
        success: false,
        email_id: None,
        preview: None,
        confirmation_token: None,
        error: Some(msg),
    }
}
