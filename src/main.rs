use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::{Shell, generate};
use fastmail_cli::models::Output;
use fastmail_cli::{commands, jmap, mcp, util};
use std::io;
use tracing_subscriber::EnvFilter;

const DEFAULT_HTTP_ADDR: &str = "127.0.0.1:8080";

/// Build ComposeParams with resolved HTML and loaded attachments.
#[allow(clippy::too_many_arguments)]
fn build_compose_params<'a>(
    cc: Option<&'a str>,
    bcc: Option<&'a str>,
    from: Option<&'a str>,
    draft: bool,
    html_body: Option<String>,
    html_file: Option<String>,
    attachment_paths: &[String],
    attachment_manifests: &[String],
) -> anyhow::Result<jmap::ComposeParams<'a>> {
    let resolved_html = util::resolve_html(html_body, html_file)?;
    let attachments = util::load_attachments(attachment_paths, attachment_manifests)?;
    Ok(jmap::ComposeParams {
        cc: cc.map(util::parse_addresses).unwrap_or_default(),
        bcc: bcc.map(util::parse_addresses).unwrap_or_default(),
        from,
        draft,
        html_body: resolved_html,
        attachments,
    })
}

#[derive(Parser)]
#[command(name = "fastmail")]
#[command(version, about = "CLI for Fastmail's JMAP API", long_about = None)]
struct Cli {
    /// Select an accessible JMAP account (default: each capability's primary account)
    #[arg(long, global = true)]
    account: Option<String>,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Query {
        #[arg(value_enum)]
        kind: jmap::DataType,
        file: std::path::PathBuf,
    },
    Sync {
        #[arg(value_enum)]
        kind: jmap::DataType,
        #[arg(long)]
        cache: std::path::PathBuf,
        #[arg(long, value_name = "SECONDS")]
        watch: Option<u64>,
    },
    Accounts,
    #[command(subcommand)]
    Email(commands::EmailCommands),
    #[command(subcommand)]
    Identities(commands::IdentityCommands),
    #[command(subcommand)]
    Submissions(commands::SubmissionCommands),
    #[command(subcommand)]
    Push(commands::PushCommands),
    State {
        #[arg(value_enum)]
        kind: jmap::DataType,
    },
    Changes {
        #[arg(value_enum)]
        kind: jmap::DataType,
        since_state: String,
    },
    QueryChanges {
        #[arg(value_enum)]
        kind: jmap::DataType,
        file: std::path::PathBuf,
    },
    /// List, edit, send, and discard existing drafts
    #[command(subcommand)]
    Drafts(commands::DraftCommands),
    /// Upload an attachment and return a reusable blob manifest
    UploadAttachment {
        file: String,
        #[arg(long)]
        content_type: Option<String>,
    },
    /// Create, edit, and delete mailboxes
    #[command(subcommand)]
    Mailbox(commands::MailboxCommands),
    /// Manage vacation / out-of-office responses
    #[command(subcommand)]
    Vacation(commands::VacationCommands),
    /// Import an RFC 5322 .eml file without sending it
    Import {
        file: std::path::PathBuf,
        #[arg(long, default_value = "INBOX")]
        mailbox: String,
        #[arg(long = "keyword")]
        keywords: Vec<String>,
        #[arg(long)]
        received_at: Option<String>,
    },
    /// Get query-specific search excerpts (searches first when no IDs are given)
    Snippets {
        #[arg(long)]
        text: String,
        #[arg(long = "email-id")]
        ids: Vec<String>,
        #[arg(long)]
        mailbox: Option<String>,
        #[arg(long, default_value = "50")]
        limit: u32,
    },
    /// Star an email
    Star {
        email_id: String,
    },
    /// Remove an email's star
    Unstar {
        email_id: String,
    },
    /// Authenticate with Fastmail API token
    Auth {
        /// API token from Fastmail settings. If omitted, the token is read from
        /// stdin so it doesn't appear in `ps`, shell history, or the environment.
        token: Option<String>,
    },

    /// List resources
    #[command(subcommand)]
    List(ListCommands),

    /// Get a specific email by ID
    Get {
        /// Email ID
        email_id: String,
    },

    /// Stream newly arrived emails as newline-delimited JSON, until interrupted
    Watch {
        /// Only report mail landing in this mailbox (name or role)
        #[arg(short, long)]
        mailbox: Option<String>,

        /// Include bodies and attachment metadata, not just summaries
        #[arg(long)]
        full: bool,

        /// Check every N seconds instead of holding a push connection open
        #[arg(long, value_name = "SECONDS")]
        poll: Option<u64>,
    },

    /// Get all emails in a thread/conversation
    Thread {
        /// Email ID (will fetch entire thread this email belongs to)
        email_id: String,
    },

    /// Search emails with JMAP filters
    Search {
        #[command(flatten)]
        options: commands::SearchOptions,
        /// Full-text search (from, to, cc, bcc, subject, body)
        #[arg(short, long)]
        text: Option<String>,

        /// Filter by From header
        #[arg(long)]
        from: Option<String>,

        /// Filter by To header
        #[arg(long)]
        to: Option<String>,

        /// Filter by Cc header
        #[arg(long)]
        cc: Option<String>,

        /// Filter by Bcc header
        #[arg(long)]
        bcc: Option<String>,

        /// Filter by Subject
        #[arg(long)]
        subject: Option<String>,

        /// Filter by body content
        #[arg(long)]
        body: Option<String>,

        /// Filter by mailbox name
        #[arg(short, long)]
        mailbox: Option<String>,

        /// Only emails with attachments
        #[arg(long)]
        has_attachment: bool,

        /// Minimum email size in bytes
        #[arg(long)]
        min_size: Option<u32>,

        /// Maximum email size in bytes
        #[arg(long)]
        max_size: Option<u32>,

        /// Emails received before date (ISO 8601, e.g., 2024-01-01)
        #[arg(long)]
        before: Option<String>,

        /// Emails received on or after date (ISO 8601, e.g., 2024-01-01)
        #[arg(long)]
        after: Option<String>,

        /// Only unread emails
        #[arg(long)]
        unread: bool,

        /// Only flagged/starred emails
        #[arg(long)]
        flagged: bool,

        /// Maximum results
        #[arg(short, long, default_value = "50")]
        limit: u32,
    },

    /// Send an email
    Send {
        /// Recipient(s), comma-separated
        #[arg(long)]
        to: String,

        /// Subject line
        #[arg(long)]
        subject: String,

        /// Email body (plain text)
        #[arg(long)]
        body: String,

        /// CC recipient(s), comma-separated
        #[arg(long)]
        cc: Option<String>,

        /// BCC recipient(s), comma-separated
        #[arg(long)]
        bcc: Option<String>,

        /// In-Reply-To message ID (for threading)
        #[arg(long)]
        reply_to: Option<String>,

        /// Send from a specific identity (email address). Use `list identities` to see available.
        #[arg(long)]
        from: Option<String>,

        /// Save as draft instead of sending
        #[arg(long)]
        draft: bool,

        /// HTML body content
        #[arg(long, conflicts_with = "html_file")]
        html_body: Option<String>,

        /// Path to HTML file for email body
        #[arg(long, conflicts_with = "html_body")]
        html_file: Option<String>,

        /// File attachment (repeatable)
        #[arg(long = "attachment", short = 'a', action = clap::ArgAction::Append)]
        attachments: Vec<String>,
        #[arg(long = "attachment-manifest")]
        attachment_manifests: Vec<String>,
    },

    /// Move email to a mailbox
    Move {
        /// Email ID
        email_id: String,

        /// Destination mailbox name
        #[arg(long)]
        to: String,
    },

    /// Mark email as spam
    Spam {
        /// Email ID
        email_id: String,

        /// Skip confirmation
        #[arg(short = 'y', long)]
        yes: bool,
    },

    /// Mark email as read or unread
    MarkRead {
        /// Email ID
        email_id: String,

        /// Mark as unread instead of read
        #[arg(long)]
        unread: bool,
    },

    /// Download attachments from an email
    Download {
        /// Email ID
        email_id: String,

        /// Output directory (default: current directory)
        #[arg(short, long)]
        output: Option<String>,

        /// Output format: raw (save files) or json (extract text)
        #[arg(short, long)]
        format: Option<String>,

        /// Max size for images (e.g., 500K, 1M). Images larger than this are resized.
        #[arg(long)]
        max_size: Option<String>,
    },

    /// Reply to an email
    Reply {
        #[arg(long = "attachment-manifest")]
        attachment_manifests: Vec<String>,
        /// Email ID to reply to
        email_id: String,

        /// Reply body (plain text)
        #[arg(long)]
        body: String,

        /// Reply to all recipients
        #[arg(long)]
        all: bool,

        /// Additional CC recipient(s), comma-separated
        #[arg(long)]
        cc: Option<String>,

        /// BCC recipient(s), comma-separated
        #[arg(long)]
        bcc: Option<String>,

        /// Send from a specific identity (email address). Use `list identities` to see available.
        #[arg(long)]
        from: Option<String>,

        /// Save as draft instead of sending
        #[arg(long)]
        draft: bool,

        /// HTML body content
        #[arg(long, conflicts_with = "html_file")]
        html_body: Option<String>,

        /// Path to HTML file for email body
        #[arg(long, conflicts_with = "html_body")]
        html_file: Option<String>,

        /// File attachment (repeatable)
        #[arg(long = "attachment", short = 'a', action = clap::ArgAction::Append)]
        attachments: Vec<String>,
    },

    /// Forward an email
    Forward {
        #[arg(long = "attachment-manifest")]
        attachment_manifests: Vec<String>,
        /// Email ID to forward
        email_id: String,

        /// Recipient(s), comma-separated
        #[arg(long)]
        to: String,

        /// Message to include before forwarded content
        #[arg(long, default_value = "")]
        body: String,

        /// CC recipient(s), comma-separated
        #[arg(long)]
        cc: Option<String>,

        /// BCC recipient(s), comma-separated
        #[arg(long)]
        bcc: Option<String>,

        /// Send from a specific identity (email address). Use `list identities` to see available.
        #[arg(long)]
        from: Option<String>,

        /// Save as draft instead of sending
        #[arg(long)]
        draft: bool,

        /// HTML body content
        #[arg(long, conflicts_with = "html_file")]
        html_body: Option<String>,

        /// Path to HTML file for email body
        #[arg(long, conflicts_with = "html_body")]
        html_file: Option<String>,

        /// File attachment (repeatable)
        #[arg(long = "attachment", short = 'a', action = clap::ArgAction::Append)]
        attachments: Vec<String>,
    },

    /// Generate shell completions
    Completions {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: Shell,
    },

    /// Manage masked email addresses
    #[command(subcommand)]
    Masked(MaskedCommands),

    /// Manage contacts (JMAP preferred, CardDAV retained)
    Contacts {
        #[arg(long, value_enum, default_value = "auto", global = true)]
        backend: fastmail_cli::contacts::ContactBackend,
        #[command(subcommand)]
        command: ContactsCommands,
    },

    /// Manage calendars and iCalendar events via CalDAV
    #[command(subcommand)]
    Calendar(commands::CalendarCommands),

    /// Manage Fastmail file storage via WebDAV
    #[command(subcommand)]
    Files(commands::FilesCommands),

    /// Manage Fastmail notes via JMAP
    #[command(subcommand)]
    Notes(commands::NotesCommands),

    /// Run as MCP (Model Context Protocol) server for Claude integration
    Mcp {
        /// Serve MCP over streamable HTTP at /mcp instead of stdio, on this
        /// address (default 127.0.0.1:8080). The `X-Fastmail-Token` header
        /// overrides the configured token per request, which is how a hosted
        /// deployment serves many users.
        #[arg(
            long,
            value_name = "ADDR",
            num_args = 0..=1,
            default_missing_value = DEFAULT_HTTP_ADDR,
        )]
        http: Option<String>,

        /// Serve plain GraphQL-over-HTTP at /graphql
        #[arg(long)]
        graphql: bool,

        /// Serve the GraphiQL IDE at /, and the /graphql it talks to
        #[arg(long)]
        graphiql: bool,

        /// Open the GraphiQL IDE in your browser once listening (implies
        /// --graphiql)
        #[arg(long)]
        browser: bool,
    },
}

#[derive(Subcommand)]
enum MaskedCommands {
    /// Edit metadata without changing the address's state
    Update {
        id: String,
        #[arg(long)]
        domain: Option<String>,
        #[arg(long)]
        description: Option<String>,
        #[arg(long, conflicts_with = "clear_url")]
        url: Option<String>,
        #[arg(long)]
        clear_url: bool,
    },
    /// List all masked email addresses
    List,

    /// Create a new masked email address
    Create {
        /// Domain this masked email is for (e.g., https://example.com)
        #[arg(long)]
        domain: Option<String>,

        /// Description for the masked email
        #[arg(long)]
        description: Option<String>,

        /// Custom prefix for the email address (max 64 chars, a-z/0-9/underscore)
        #[arg(long)]
        prefix: Option<String>,
        /// Deep link back to the related record
        #[arg(long)]
        url: Option<String>,
        /// Create pending (expires after 24h if unused)
        #[arg(long)]
        pending: bool,
    },

    /// Enable a masked email address
    Enable {
        /// Masked email ID
        id: String,
    },

    /// Disable a masked email address
    Disable {
        /// Masked email ID
        id: String,
    },

    /// Delete a masked email address
    Delete {
        /// Masked email ID
        id: String,

        /// Skip confirmation
        #[arg(short = 'y', long)]
        yes: bool,
    },
}

#[derive(Subcommand)]
enum ListCommands {
    /// List mailboxes (folders)
    Mailboxes,

    /// List emails in a mailbox
    Emails {
        /// Mailbox name (default: INBOX)
        #[arg(short, long, default_value = "INBOX")]
        mailbox: String,

        /// Maximum results
        #[arg(short, long, default_value = "50")]
        limit: u32,
    },

    /// List sender identities (for use with --from)
    Identities,
}

#[derive(Subcommand)]
enum ContactsCommands {
    #[command(flatten)]
    Native(commands::ContactNativeCommands),
    /// List all contacts
    List,

    /// Search contacts by name or email
    Search {
        /// Search query
        query: String,
    },

    /// Create a new contact
    Create {
        /// JMAP address-book ID; omitted uses a writable default
        #[arg(long)]
        address_book: Option<String>,
        /// Full name
        #[arg(long)]
        name: String,

        /// Email address(es), comma-separated
        #[arg(long)]
        email: Option<String>,

        /// Phone number(s), comma-separated
        #[arg(long)]
        phone: Option<String>,

        /// Organization/company
        #[arg(long)]
        organization: Option<String>,

        /// Job title
        #[arg(long)]
        title: Option<String>,

        /// Notes
        #[arg(long)]
        notes: Option<String>,
    },

    /// Update an existing contact
    Update {
        /// Contact ID
        contact_id: String,

        /// Full name
        #[arg(long)]
        name: Option<String>,

        /// Email address(es), comma-separated (replaces existing)
        #[arg(long)]
        email: Option<String>,

        /// Phone number(s), comma-separated (replaces existing)
        #[arg(long)]
        phone: Option<String>,

        /// Organization/company
        #[arg(long)]
        organization: Option<String>,

        /// Job title
        #[arg(long)]
        title: Option<String>,

        /// Notes
        #[arg(long)]
        notes: Option<String>,
    },

    /// Delete a contact
    Delete {
        /// Contact ID
        contact_id: String,

        /// Skip confirmation
        #[arg(short = 'y', long)]
        yes: bool,
    },
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_target(false)
        .init();

    let cli = Cli::parse();

    let result = jmap::with_account(cli.account, async move {
        match cli.command {
            Commands::Accounts => commands::accounts().await,
            Commands::Query { kind, file } => commands::query_records(kind, file).await,
            Commands::Sync { kind, cache, watch } => {
                commands::synchronize(kind, cache, watch).await
            }
            Commands::Email(command) => commands::email_operations(command).await,
            Commands::Identities(command) => commands::identities(command).await,
            Commands::Submissions(command) => commands::submissions(command).await,
            Commands::Push(command) => commands::push(command).await,
            Commands::State { kind } => commands::data_changes(kind, None, None).await,
            Commands::Changes { kind, since_state } => {
                commands::data_changes(kind, Some(since_state), None).await
            }
            Commands::QueryChanges { kind, file } => {
                commands::data_changes(kind, None, Some(file)).await
            }
            Commands::Drafts(command) => commands::drafts(command).await,
            Commands::UploadAttachment { file, content_type } => {
                commands::upload_attachment(&file, content_type).await
            }
            Commands::Mailbox(command) => commands::mailbox(command).await,
            Commands::Vacation(command) => commands::vacation(command).await,
            Commands::Import {
                file,
                mailbox,
                keywords,
                received_at,
            } => commands::import_message(file, &mailbox, keywords, received_at.as_deref()).await,
            Commands::Snippets {
                text,
                ids,
                mailbox,
                limit,
            } => commands::snippets(&text, ids, mailbox.as_deref(), limit).await,
            Commands::Star { email_id } => commands::star(&email_id, true).await,
            Commands::Unstar { email_id } => commands::star(&email_id, false).await,
            Commands::Auth { token } => {
                let resolved = match token {
                    Some(t) => Ok(t),
                    None => commands::read_token_from_stdin(),
                };
                match resolved {
                    Ok(t) => commands::auth(&t).await,
                    Err(e) => Err(e),
                }
            }

            Commands::List(cmd) => match cmd {
                ListCommands::Mailboxes => commands::list_mailboxes().await,
                ListCommands::Emails { mailbox, limit } => {
                    commands::list_emails(&mailbox, limit).await
                }
                ListCommands::Identities => commands::list_identities().await,
            },

            Commands::Get { email_id } => commands::get_email(&email_id).await,

            Commands::Thread { email_id } => commands::get_thread(&email_id).await,

            Commands::Watch {
                mailbox,
                full,
                poll,
            } => {
                commands::watch(commands::WatchOptions {
                    mailbox,
                    full,
                    poll,
                })
                .await
            }

            Commands::Search {
                options,
                text,
                from,
                to,
                cc,
                bcc,
                subject,
                body,
                mailbox,
                has_attachment,
                min_size,
                max_size,
                before,
                after,
                unread,
                flagged,
                limit,
            } => {
                commands::search_with_options(
                    commands::SearchFilter {
                        text,
                        from,
                        to,
                        cc,
                        bcc,
                        subject,
                        body,
                        mailbox,
                        has_attachment,
                        min_size,
                        max_size,
                        before,
                        after,
                        unread,
                        flagged,
                    },
                    limit,
                    options,
                )
                .await
            }

            Commands::Send {
                attachment_manifests,
                to,
                subject,
                body,
                cc,
                bcc,
                reply_to,
                from,
                draft,
                html_body,
                html_file,
                attachments,
            } => {
                async {
                    let params = build_compose_params(
                        cc.as_deref(),
                        bcc.as_deref(),
                        from.as_deref(),
                        draft,
                        html_body,
                        html_file,
                        &attachments,
                        &attachment_manifests,
                    )?;
                    commands::send(&to, &subject, &body, reply_to.as_deref(), params).await
                }
                .await
            }

            Commands::Move { email_id, to } => commands::move_email(&email_id, &to).await,

            Commands::Spam { email_id, yes } => {
                if !yes {
                    eprintln!("Mark email {} as spam? Use -y to confirm.", email_id);
                    std::process::exit(1);
                }
                commands::mark_spam(&email_id).await
            }

            Commands::MarkRead { email_id, unread } => {
                commands::mark_read(&email_id, !unread).await
            }

            Commands::Download {
                email_id,
                output,
                format,
                max_size,
            } => {
                commands::download_attachment(
                    &email_id,
                    output.as_deref(),
                    format.as_deref(),
                    max_size.as_deref(),
                )
                .await
            }

            Commands::Reply {
                attachment_manifests,
                email_id,
                body,
                all,
                cc,
                bcc,
                from,
                draft,
                html_body,
                html_file,
                attachments,
            } => {
                async {
                    let params = build_compose_params(
                        cc.as_deref(),
                        bcc.as_deref(),
                        from.as_deref(),
                        draft,
                        html_body,
                        html_file,
                        &attachments,
                        &attachment_manifests,
                    )?;
                    commands::reply(&email_id, &body, all, params).await
                }
                .await
            }

            Commands::Forward {
                attachment_manifests,
                email_id,
                to,
                body,
                cc,
                bcc,
                from,
                draft,
                html_body,
                html_file,
                attachments,
            } => {
                async {
                    let params = build_compose_params(
                        cc.as_deref(),
                        bcc.as_deref(),
                        from.as_deref(),
                        draft,
                        html_body,
                        html_file,
                        &attachments,
                        &attachment_manifests,
                    )?;
                    commands::forward(&email_id, &to, &body, params).await
                }
                .await
            }

            Commands::Completions { shell } => {
                generate(shell, &mut Cli::command(), "fastmail", &mut io::stdout());
                Ok(())
            }

            Commands::Masked(cmd) => match cmd {
                MaskedCommands::Update {
                    id,
                    domain,
                    description,
                    url,
                    clear_url,
                } => {
                    commands::edit_masked_email(
                        &id,
                        domain.as_deref(),
                        description.as_deref(),
                        url.as_deref(),
                        clear_url,
                    )
                    .await
                }
                MaskedCommands::List => commands::list_masked_emails().await,
                MaskedCommands::Create {
                    domain,
                    description,
                    prefix,
                    url,
                    pending,
                } => {
                    commands::create_masked_email(
                        domain.as_deref(),
                        description.as_deref(),
                        prefix.as_deref(),
                        url.as_deref(),
                        pending,
                    )
                    .await
                }
                MaskedCommands::Enable { id } => commands::enable_masked_email(&id).await,
                MaskedCommands::Disable { id } => commands::disable_masked_email(&id).await,
                MaskedCommands::Delete { id, yes } => {
                    if !yes {
                        eprintln!("Delete masked email {}? Use -y to confirm.", id);
                        std::process::exit(1);
                    }
                    commands::delete_masked_email(&id).await
                }
            },

            Commands::Contacts { command, backend } => match command {
                ContactsCommands::Native(command) => {
                    commands::contact_native(command, backend).await
                }
                ContactsCommands::List => commands::list_contacts(backend).await,
                ContactsCommands::Search { query } => {
                    commands::search_contacts(&query, backend).await
                }
                ContactsCommands::Create {
                    address_book,
                    name,
                    email,
                    phone,
                    organization,
                    title,
                    notes,
                } => {
                    commands::create_contact(
                        backend,
                        &name,
                        email.as_deref(),
                        phone.as_deref(),
                        organization.as_deref(),
                        title.as_deref(),
                        notes.as_deref(),
                        address_book.as_deref(),
                    )
                    .await
                }
                ContactsCommands::Update {
                    contact_id,
                    name,
                    email,
                    phone,
                    organization,
                    title,
                    notes,
                } => {
                    commands::update_contact(
                        backend,
                        &contact_id,
                        name.as_deref(),
                        email.as_deref(),
                        phone.as_deref(),
                        organization.as_deref(),
                        title.as_deref(),
                        notes.as_deref(),
                    )
                    .await
                }
                ContactsCommands::Delete { contact_id, yes } => {
                    if !yes {
                        eprintln!("Delete contact {}? Use -y to confirm.", contact_id);
                        std::process::exit(1);
                    }
                    commands::delete_contact(&contact_id, backend).await
                }
            },

            Commands::Calendar(cmd) => commands::calendar(cmd).await,
            Commands::Files(cmd) => commands::files(cmd).await,
            Commands::Notes(cmd) => commands::notes(cmd).await,

            Commands::Mcp {
                http,
                graphql,
                graphiql,
                browser,
            } => {
                let graphiql = graphiql || browser;
                let surfaces = mcp::HttpSurfaces {
                    graphql: graphql || graphiql,
                    graphiql,
                    browser,
                };
                // Any HTTP-only surface implies the HTTP transport, at its default address.
                let addr =
                    http.or_else(|| (surfaces.graphql).then(|| DEFAULT_HTTP_ADDR.to_string()));
                match addr {
                    Some(addr) => mcp::run_http_server(&addr, surfaces).await,
                    None => mcp::run_server().await,
                }
            }
        }
    })
    .await;

    if let Err(e) = result {
        if let Some(bulk) = e.downcast_ref::<commands::BulkFailure>() {
            Output {
                success: false,
                data: Some(&bulk.0),
                error: Some(e.to_string()),
                message: None,
            }
            .print();
        } else {
            Output::<()>::error(e.to_string()).print();
        }
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn management_commands_parse_and_reject_conflicting_options() {
        Cli::command().debug_assert();
        for args in [
            vec![
                "fastmail", "mailbox", "create", "Projects", "--parent", "parent",
            ],
            vec![
                "fastmail",
                "mailbox",
                "update",
                "folder",
                "--subscribed",
                "false",
                "--root",
            ],
            vec![
                "fastmail",
                "vacation",
                "set",
                "--enabled",
                "false",
                "--clear-from",
            ],
            vec!["fastmail", "import", "message.eml", "--keyword", "$seen"],
            vec![
                "fastmail",
                "snippets",
                "--text",
                "project",
                "--email-id",
                "e1",
            ],
            vec!["fastmail", "star", "e1"],
            vec!["fastmail", "unstar", "e1"],
            vec!["fastmail", "contacts", "--backend", "carddav", "list"],
            vec!["fastmail", "contacts", "list", "--backend", "jmap"],
            vec!["fastmail", "accounts"],
            vec![
                "fastmail",
                "--account",
                "shared",
                "email",
                "update",
                "e1",
                "e2",
                "--add-keyword",
                "project",
                "--starred",
                "true",
                "--add-mailbox",
                "folder",
            ],
            vec!["fastmail", "email", "delete", "e1", "e2", "-y"],
            vec!["fastmail", "email", "empty-trash", "-y"],
            vec![
                "fastmail",
                "email",
                "export",
                "e1",
                "--output",
                "message.eml",
            ],
            vec!["fastmail", "state", "submission"],
            vec!["fastmail", "changes", "email", "old-state"],
            vec!["fastmail", "query-changes", "email", "query.json"],
            vec![
                "fastmail",
                "sync",
                "contact",
                "--cache",
                "contacts.json",
                "--watch",
                "30",
            ],
            vec![
                "fastmail",
                "identities",
                "update",
                "i1",
                "--text-signature",
                "Regards",
            ],
            vec!["fastmail", "submissions", "cancel", "s1"],
            vec!["fastmail", "push", "create", "push.json"],
            vec![
                "fastmail",
                "contacts",
                "details",
                "c1",
                "--birthday",
                "1990-01-02",
                "--address",
                "123 Test St",
                "--country-code",
                "US",
            ],
            vec!["fastmail", "contacts", "get", "c1"],
            vec!["fastmail", "contacts", "addressbooks", "create", "Team"],
            vec![
                "fastmail",
                "contacts",
                "card-create",
                "card.json",
                "--address-book",
                "book",
            ],
            vec![
                "fastmail",
                "contacts",
                "group-create",
                "Team",
                "--member",
                "person-uid",
            ],
            vec!["fastmail", "contacts", "changes", "old-state"],
            vec![
                "fastmail",
                "masked",
                "create",
                "--pending",
                "--url",
                "app://record",
            ],
            vec!["fastmail", "masked", "update", "m1", "--clear-url"],
            vec![
                "fastmail",
                "calendar",
                "events",
                "/cal/",
                "--expand",
                "--after",
                "20261016T000000Z",
                "--before",
                "20261017T000000Z",
            ],
            vec![
                "fastmail",
                "calendar",
                "create-calendar",
                "/cal/",
                "--name",
                "Work",
            ],
            vec!["fastmail", "files", "rename", "/a", "/b"],
            vec![
                "fastmail",
                "notes",
                "update",
                "n1",
                "--html-body",
                "<b>New</b>",
                "--clear-attachments",
            ],
            vec![
                "fastmail",
                "drafts",
                "update",
                "d1",
                "--attachment-manifest",
                "upload.json",
            ],
            vec!["fastmail", "upload-attachment", "file.png"],
            vec![
                "fastmail",
                "search",
                "--filter",
                r#"{"operator":"OR","conditions":[{"from":"alice"},{"from":"bob"}]}"#,
                "--total",
            ],
            vec![
                "fastmail",
                "search",
                "--anchor",
                "e1",
                "--anchor-offset",
                "1",
            ],
            vec![
                "fastmail",
                "send",
                "--to",
                "a@example.com",
                "--subject",
                "Hi",
                "--body",
                "Body",
                "--attachment-manifest",
                "upload.json",
            ],
        ] {
            assert!(Cli::try_parse_from(&args).is_ok(), "{args:?}");
        }
        assert!(Cli::try_parse_from(["fastmail", "oauth", "login"]).is_err());
        assert!(
            Cli::try_parse_from([
                "fastmail", "mailbox", "update", "folder", "--root", "--parent", "parent"
            ])
            .is_err()
        );
        assert!(
            Cli::try_parse_from([
                "fastmail",
                "vacation",
                "set",
                "--from",
                "2026-10-16T00:00:00Z",
                "--clear-from"
            ])
            .is_err()
        );
    }
}
