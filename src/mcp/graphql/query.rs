//! GraphQL query resolvers

use async_graphql::{Context, Object, Result};

use super::api::*;
use super::connection::{
    EmailConnection, ListConnection, PageArgs, emails_connection, page_complexity, paginate,
};
use super::filter::{EmailFilter, EmailSort};
use super::loaders::{Emails, Identities, MaskedEmails, to_gql_error};
use super::types::*;
use super::{CardDavCreds, SharedClient};

pub struct QueryRoot;

#[Object]
#[allow(clippy::too_many_arguments)]
impl QueryRoot {
    async fn query_records(
        &self,
        ctx: &Context<'_>,
        kind: crate::jmap::DataType,
        arguments: async_graphql::Json<serde_json::Value>,
    ) -> Result<QueryResult> {
        let client = ctx.data::<SharedClient>()?.lock().await;
        Ok(client.query_records(kind, arguments.0, false).await?.into())
    }
    async fn active_account(
        &self,
        ctx: &Context<'_>,
        kind: crate::jmap::DataType,
    ) -> Result<String> {
        Ok(ctx
            .data::<SharedClient>()?
            .lock()
            .await
            .account_for(kind.capability())?
            .into())
    }
    /// Native records, including submission delivery status, or synchronization state with ids: [].
    async fn records(
        &self,
        ctx: &Context<'_>,
        kind: crate::jmap::DataType,
        ids: Option<Vec<String>>,
    ) -> Result<RecordPage> {
        let client = ctx.data::<SharedClient>()?.lock().await;
        Ok(client.records(kind, ids.as_deref()).await?.into())
    }

    /// Full created/updated/destroyed changes for any supported data type.
    async fn changes(
        &self,
        ctx: &Context<'_>,
        kind: crate::jmap::DataType,
        since_state: String,
        max_changes: Option<u32>,
    ) -> Result<ChangeResult> {
        let client = ctx.data::<SharedClient>()?.lock().await;
        Ok(client
            .changes(kind, &since_state, max_changes)
            .await?
            .into())
    }

    async fn query_changes(
        &self,
        ctx: &Context<'_>,
        kind: crate::jmap::DataType,
        arguments: async_graphql::Json<serde_json::Value>,
    ) -> Result<QueryResult> {
        let client = ctx.data::<SharedClient>()?.lock().await;
        Ok(client.query_records(kind, arguments.0, true).await?.into())
    }

    async fn submissions(
        &self,
        ctx: &Context<'_>,
        arguments: Option<async_graphql::Json<serde_json::Value>>,
    ) -> Result<QueryResult> {
        let client = ctx.data::<SharedClient>()?.lock().await;
        Ok(client
            .query_records(
                crate::jmap::DataType::Submission,
                arguments
                    .map(|v| v.0)
                    .unwrap_or(serde_json::json!({"limit": 50})),
                false,
            )
            .await?
            .into())
    }

    async fn push_subscriptions(&self, ctx: &Context<'_>) -> Result<RecordPage> {
        let client = ctx.data::<SharedClient>()?.lock().await;
        Ok(client.push_subscriptions().await?.into())
    }

    /// Download a complete RFC 5322 message as base64.
    async fn raw_email(&self, ctx: &Context<'_>, id: String) -> Result<BlobDownload> {
        use base64::Engine;
        let client = ctx.data::<SharedClient>()?.lock().await;
        let bytes = client.export_email(&id).await?;
        Ok(BlobDownload {
            size: bytes.len() as u64,
            base64: base64::engine::general_purpose::STANDARD.encode(bytes),
            etag: None,
        })
    }

    async fn calendars(&self, ctx: &Context<'_>) -> Result<Vec<crate::dav::DavResource>> {
        Ok(dav(ctx, false)?.calendars().await?)
    }

    async fn calendar_events(
        &self,
        ctx: &Context<'_>,
        calendar: String,
        after: Option<String>,
        before: Option<String>,
        #[graphql(default = false)] expand: bool,
    ) -> Result<Vec<crate::dav::DavResource>> {
        Ok(dav(ctx, false)?
            .events_expanded(&calendar, after.as_deref(), before.as_deref(), expand)
            .await?)
    }

    async fn calendar_event(&self, ctx: &Context<'_>, href: String) -> Result<CalendarData> {
        let (bytes, etag) = dav(ctx, false)?.get(&href).await?;
        Ok(CalendarData {
            href,
            etag,
            calendar_data: String::from_utf8(bytes)?,
        })
    }

    async fn calendar_free_busy(
        &self,
        ctx: &Context<'_>,
        calendar: String,
        after: String,
        before: String,
    ) -> Result<String> {
        Ok(dav(ctx, false)?
            .free_busy(&calendar, &after, &before)
            .await?)
    }

    async fn calendar_scheduling(&self, ctx: &Context<'_>) -> Result<SchedulingEndpoints> {
        Ok(dav(ctx, false)?.scheduling().await?.into())
    }

    async fn files(
        &self,
        ctx: &Context<'_>,
        #[graphql(default = "/")] path: String,
    ) -> Result<Vec<FileEntry>> {
        let resources = dav(ctx, true)?
            .list(&crate::dav::DavClient::file_path(&format!(
                "{}/",
                path.trim_end_matches('/')
            ))?)
            .await?;
        resources
            .into_iter()
            .map(|r| {
                let url = reqwest::Url::parse("https://myfiles.fastmail.com/")?.join(&r.href)?;
                Ok(FileEntry {
                    path: percent_encoding::percent_decode_str(url.path())
                        .decode_utf8()?
                        .into_owned(),
                    href: r.href,
                    name: r.name,
                    collection: r.collection,
                    content_type: r.content_type,
                    size: r.size,
                    etag: r.etag,
                })
            })
            .collect()
    }

    async fn file_download(&self, ctx: &Context<'_>, path: String) -> Result<BlobDownload> {
        use base64::Engine;
        let (bytes, etag) = dav(ctx, true)?
            .get(&crate::dav::DavClient::file_path(&path)?)
            .await?;
        Ok(BlobDownload {
            size: bytes.len() as u64,
            base64: base64::engine::general_purpose::STANDARD.encode(bytes),
            etag,
        })
    }
    /// Native AddressBook/get response, including state. Use ids: [] for state only.
    async fn address_book_records(
        &self,
        ctx: &Context<'_>,
        ids: Option<Vec<String>>,
    ) -> Result<RecordPage> {
        let client = ctx.data::<SharedClient>()?.lock().await;
        Ok(client.contacts_get(true, ids).await?.into())
    }
    /// Full native JSContact Cards, preserving groups, photos, addresses, and anniversaries.
    /// Empty ids returns only the synchronization state; omit ids to fetch all.
    async fn contact_cards(
        &self,
        ctx: &Context<'_>,
        ids: Option<Vec<String>>,
    ) -> Result<RecordPage> {
        let client = ctx.data::<SharedClient>()?.lock().await;
        Ok(client.contacts_get(false, ids).await?.into())
    }

    /// Native ContactCard/query arguments, including filter, sort, pagination, and totals.
    async fn query_contact_cards(
        &self,
        ctx: &Context<'_>,
        arguments: async_graphql::Json<serde_json::Value>,
    ) -> Result<QueryResult> {
        let client = ctx.data::<SharedClient>()?.lock().await;
        Ok(client.contacts_query(arguments.0).await?.into())
    }

    /// One synchronization page; repeat with newState while hasMoreChanges is true.
    async fn contact_changes(
        &self,
        ctx: &Context<'_>,
        since_state: String,
        max_changes: Option<u32>,
        #[graphql(default = false)] address_books: bool,
    ) -> Result<ChangeResult> {
        let client = ctx.data::<SharedClient>()?.lock().await;
        Ok(ChangeResult::from(
            client
                .contacts_changes(address_books, &since_state, max_changes)
                .await?,
        ))
    }

    /// Native queryChanges arguments, including sinceQueryState and the original filter/sort.
    async fn contact_query_changes(
        &self,
        ctx: &Context<'_>,
        arguments: async_graphql::Json<serde_json::Value>,
    ) -> Result<QueryResult> {
        let client = ctx.data::<SharedClient>()?.lock().await;
        Ok(QueryResult::from(
            client.contacts_query_changes(arguments.0).await?,
        ))
    }

    async fn notes(
        &self,
        ctx: &Context<'_>,
        text: Option<String>,
        #[graphql(default = 50)] limit: u32,
    ) -> Result<NotePage> {
        let client = ctx.data::<SharedClient>()?.lock().await;
        Ok(NotePage::from(
            client.list_notes(text.as_deref(), limit).await?,
        ))
    }

    async fn note(&self, ctx: &Context<'_>, id: String) -> Result<Note> {
        let client = ctx.data::<SharedClient>()?.lock().await;
        Ok(client.get_note(&id).await?.into())
    }
    /// Current vacation response settings (JMAP singleton).
    async fn vacation_response(&self, ctx: &Context<'_>) -> Result<Option<VacationResponse>> {
        let client = ctx.data::<SharedClient>()?.lock().await;
        let value = client.vacation_response().await?;
        Ok(value
            .as_array()
            .and_then(|values| values.first())
            .cloned()
            .map(Into::into))
    }

    /// Query-specific highlighted subject and body excerpts for known email IDs.
    async fn search_snippets(
        &self,
        ctx: &Context<'_>,
        text: String,
        email_ids: Vec<String>,
    ) -> Result<SnippetResult> {
        let client = ctx.data::<SharedClient>()?.lock().await;
        Ok(SnippetResult::from(
            client
                .search_snippets(serde_json::json!({"text": text}), &email_ids)
                .await?,
        ))
    }

    /// List the JMAP contact address books, including default and access rights.
    async fn address_books(&self, ctx: &Context<'_>) -> Result<Vec<AddressBook>> {
        let client = ctx.data::<SharedClient>()?.lock().await;
        Ok(client
            .contact_addressbooks()
            .await?
            .as_array()
            .into_iter()
            .flatten()
            .cloned()
            .map(Into::into)
            .collect())
    }
    /// Whether the token still authenticates, and the account it reaches.
    ///
    /// Re-runs the JMAP session handshake rather than reporting what it said
    /// when this client was built, so it answers "is auth good *now*" — a token
    /// revoked since then shows up here. One API call, no mail touched.
    ///
    /// A rejected or unreachable server is reported in `status`, not raised as
    /// an error: not being connected is the answer to this question, not a
    /// failure to answer it. So a caller polling for connection state branches
    /// on an enum rather than pattern-matching error prose — and a dead token
    /// stays distinguishable from a Fastmail outage, which is the split anyone
    /// acting on this needs.
    async fn session(&self, ctx: &Context<'_>) -> Result<GqlSession> {
        let carddav_configured = ctx.data::<CardDavCreds>()?.is_complete();
        Ok(GqlSession::probe(ctx.data::<SharedClient>()?, carddav_configured).await)
    }

    /// List all mailboxes (folders) with unread counts. Start here to discover available folders.
    #[graphql(complexity = "page_complexity(first, last, child_complexity)")]
    async fn mailboxes(
        &self,
        ctx: &Context<'_>,
        after: Option<String>,
        before: Option<String>,
        first: Option<i32>,
        last: Option<i32>,
    ) -> Result<ListConnection<GqlMailbox>> {
        let mut mailboxes = all_mailboxes(ctx).await?.as_ref().clone();
        mailboxes.sort_by(|a, b| match (&a.role, &b.role) {
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            _ => a.name.cmp(&b.name),
        });
        paginate(
            mailboxes.into_iter().map(GqlMailbox::from).collect(),
            PageArgs {
                after,
                before,
                first,
                last,
            },
            |m| m.0.id.clone(),
        )
    }

    /// Look up a single mailbox by name or role, then navigate into its
    /// contents or around the folder tree from there.
    async fn mailbox(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Mailbox name (e.g., 'INBOX', 'Sent') or role (e.g., 'inbox', 'sent', 'drafts')"
        )]
        name: String,
    ) -> Result<Option<GqlMailbox>> {
        let all = all_mailboxes(ctx).await?;
        Ok(find_by_name_or_role(&all, &name)
            .cloned()
            .map(GqlMailbox::from))
    }

    /// Query emails with a composable filter, sorting and pagination.
    ///
    /// The filter is a tree: scalar fields on one filter object are AND-ed, and
    /// `and` / `or` / `not` nest arbitrarily, so a single query can express
    /// things like "unread, from either address, but not in Archive". Bodies,
    /// attachments and threads on the results resolve lazily and batched.
    ///
    /// Cursors are email IDs, so `after: "<last id you saw>"` resumes after that
    /// message and stays correct even if mail arrives in between. Select
    /// `totalCount` for the full match count — it is only computed when you ask
    /// for it.
    #[graphql(complexity = "page_complexity(first, last, child_complexity)")]
    async fn emails(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Which emails to match. Omit to match everything.")] filter: Option<
            EmailFilter,
        >,
        #[graphql(desc = "Sort order, most significant first. Defaults to newest first.")]
        sort: Option<Vec<EmailSort>>,
        #[graphql(desc = "Return one email per conversation instead of every message.")]
        collapse_threads: Option<bool>,
        after: Option<String>,
        before: Option<String>,
        first: Option<i32>,
        last: Option<i32>,
    ) -> Result<EmailConnection> {
        emails_connection(
            ctx,
            PageArgs {
                after,
                before,
                first,
                last,
            },
            filter,
            sort,
            None,
            collapse_threads.unwrap_or(false),
        )
        .await
    }

    /// Get a specific email by ID, with full content.
    ///
    /// Attachment payloads and the surrounding conversation resolve lazily, so
    /// pull them in this query rather than paying a second round trip.
    async fn email(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "The email ID (from emails or searchEmails queries)")] id: String,
    ) -> Result<Option<GqlEmail>> {
        let loader = ctx.data::<Emails>()?;
        Ok(loader
            .load_one(id)
            .await
            .map_err(to_gql_error)?
            .map(GqlEmail::full))
    }

    /// Get all emails in a thread/conversation with full content. Returns emails sorted
    /// oldest-first. Each email has full body and nested attachment access.
    async fn thread(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Any email ID in the thread")] email_id: String,
    ) -> Result<GqlThread> {
        let loader = ctx.data::<Emails>()?;
        let email = loader
            .load_one(email_id.clone())
            .await
            .map_err(to_gql_error)?
            .ok_or_else(|| async_graphql::Error::new(format!("Email {email_id} not found")))?;
        let thread_id = email
            .thread_id
            .ok_or_else(|| async_graphql::Error::new("Email has no thread ID"))?;
        GqlThread::load(ctx, thread_id).await
    }

    /// Search emails with flat filters.
    ///
    /// Superseded by `emails(filter: ...)`, which composes with `and`/`or`/`not`,
    /// sorts, and paginates. Kept as a shim: every argument here maps onto one
    /// leaf of an `EmailFilter`.
    #[graphql(
        deprecation = "Use `emails(filter: {...})` — it composes with and/or/not, sorts, and paginates.",
        complexity = "page_complexity(first, None, child_complexity)"
    )]
    #[allow(clippy::too_many_arguments)]
    async fn search_emails(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "General search — searches subject, body, from, and to fields")]
        query: Option<String>,
        #[graphql(desc = "Search sender address/name")] from: Option<String>,
        #[graphql(desc = "Search recipient address/name")] to: Option<String>,
        #[graphql(desc = "Search CC recipients")] cc: Option<String>,
        #[graphql(desc = "Search subject line only")] subject: Option<String>,
        #[graphql(desc = "Search email body only")] body: Option<String>,
        #[graphql(desc = "Limit search to a specific mailbox/folder")] mailbox: Option<String>,
        #[graphql(desc = "Only emails with attachments")] has_attachment: Option<bool>,
        #[graphql(desc = "Emails before this date (YYYY-MM-DD or ISO 8601)")] before: Option<
            String,
        >,
        #[graphql(desc = "Emails after this date (YYYY-MM-DD or ISO 8601)")] after: Option<String>,
        #[graphql(desc = "Only unread emails")] unread: Option<bool>,
        #[graphql(desc = "Only flagged/starred emails")] flagged: Option<bool>,
        #[graphql(desc = "Maximum number of results (default 25, max 100)")] first: Option<i32>,
    ) -> Result<EmailConnection> {
        let filter = EmailFilter {
            text: query,
            from,
            to,
            cc,
            subject,
            body,
            in_mailbox: mailbox,
            has_attachment,
            before,
            after,
            // The old arguments were `false`-means-unset, so only constrain on true.
            unread: unread.filter(|u| *u),
            flagged: flagged.filter(|f| *f),
            ..Default::default()
        };

        emails_connection(
            ctx,
            PageArgs {
                after: None,
                before: None,
                first,
                last: None,
            },
            Some(filter),
            None,
            None,
            false,
        )
        .await
    }

    /// List attachment metadata for an email. Metadata alone downloads nothing;
    /// select `base64`, `image` or `text` to pull the data. Equivalent to the
    /// `attachments` field on `email(id:)`.
    #[graphql(complexity = "page_complexity(first, last, child_complexity)")]
    async fn attachments(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "The email ID")] email_id: String,
        after: Option<String>,
        before: Option<String>,
        first: Option<i32>,
        last: Option<i32>,
    ) -> Result<ListConnection<GqlAttachment>> {
        let loader = ctx.data::<Emails>()?;
        let email = loader
            .load_one(email_id.clone())
            .await
            .map_err(to_gql_error)?
            .ok_or_else(|| async_graphql::Error::new(format!("Email {email_id} not found")))?;
        paginate(
            attachments_of(&email),
            PageArgs {
                after,
                before,
                first,
                last,
            },
            |a| a.blob_id.clone(),
        )
    }

    /// Get a single attachment by blob ID. Select `content` to fetch its data.
    async fn attachment(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "The email ID the attachment belongs to")] email_id: String,
        #[graphql(desc = "The blob ID of the attachment (from attachments query)")] blob_id: String,
    ) -> Result<Option<GqlAttachment>> {
        let loader = ctx.data::<Emails>()?;
        let email = loader
            .load_one(email_id.clone())
            .await
            .map_err(to_gql_error)?
            .ok_or_else(|| async_graphql::Error::new(format!("Email {email_id} not found")))?;
        Ok(attachments_of(&email)
            .into_iter()
            .find(|a| a.blob_id == blob_id))
    }

    /// List all sender identities on the account. Includes signatures and default reply-to/bcc.
    #[graphql(complexity = "page_complexity(first, last, child_complexity)")]
    async fn identities(
        &self,
        ctx: &Context<'_>,
        after: Option<String>,
        before: Option<String>,
        first: Option<i32>,
        last: Option<i32>,
    ) -> Result<ListConnection<GqlIdentity>> {
        let identities = ctx
            .data::<Identities>()?
            .load_one(())
            .await
            .map_err(to_gql_error)?
            .unwrap_or_default();
        paginate(
            identities.iter().cloned().map(GqlIdentity::from).collect(),
            PageArgs {
                after,
                before,
                first,
                last,
            },
            |i| i.id.clone(),
        )
    }

    /// List all masked email addresses.
    #[graphql(complexity = "page_complexity(first, last, child_complexity)")]
    async fn masked_emails(
        &self,
        ctx: &Context<'_>,
        after: Option<String>,
        before: Option<String>,
        first: Option<i32>,
        last: Option<i32>,
    ) -> Result<ListConnection<GqlMaskedEmail>> {
        let mut masked = ctx
            .data::<MaskedEmails>()?
            .load_one(())
            .await
            .map_err(to_gql_error)?
            .unwrap_or_default()
            .as_ref()
            .clone();
        masked.sort_by(|a, b| {
            let a_enabled = a.state.as_deref() == Some("enabled");
            let b_enabled = b.state.as_deref() == Some("enabled");
            match (a_enabled, b_enabled) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => a.email.cmp(&b.email),
            }
        });
        paginate(
            masked.into_iter().map(GqlMaskedEmail::from).collect(),
            PageArgs {
                after,
                before,
                first,
                last,
            },
            |m| m.id.clone(),
        )
    }

    /// Search contacts by name, email, or organization.
    ///
    /// Uses JMAP when advertised, otherwise CardDAV with request-scoped credentials.
    #[graphql(complexity = "page_complexity(first, last, child_complexity)")]
    async fn contacts(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Search query — matches name, email, or organization")] query: String,
        after: Option<String>,
        before: Option<String>,
        first: Option<i32>,
        last: Option<i32>,
    ) -> Result<ListConnection<GqlContact>> {
        let creds = ctx.data::<CardDavCreds>()?;
        let client = crate::contacts::ContactsClient::for_request(
            ctx.data::<SharedClient>()?.clone(),
            creds.username.clone(),
            creds.app_password.clone(),
        )
        .await?;
        let contacts = client.search(Some(&query)).await?;
        paginate(
            contacts.into_iter().map(GqlContact::from).collect(),
            PageArgs {
                after,
                before,
                first,
                last,
            },
            |c| c.id.clone(),
        )
    }
}
