mod allowlist;
mod config;
#[cfg(feature = "dav")]
mod dav;
mod ddns;
mod http;
mod jmap;
mod markdown;
#[cfg(feature = "notes")]
mod notes;
#[cfg(any(feature = "dav", feature = "notes"))]
mod uid;
mod watch;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
#[cfg(feature = "completions")]
use clap::CommandFactory;
#[cfg(feature = "notes")]
use clap::ValueEnum;
#[cfg(feature = "completions")]
use clap_complete::{generate, Shell};
use config::Credentials;
#[cfg(feature = "dav")]
use dav::Dav;
use jmap::Jmap;
use serde_json::{json, Value};
use std::path::PathBuf;
use tracing_subscriber::EnvFilter;

/// Fastmail from the command line. JMAP first, with DAV fallbacks.
#[derive(Parser)]
#[command(name = "fastmail", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Masked email addresses
    Masked {
        #[command(subcommand)]
        command: MaskedCommand,
    },
    /// Vacation auto-reply settings
    Vacation {
        #[command(subcommand)]
        command: VacationCommand,
    },
    /// Show which features the configured credentials can reach
    Doctor,
    /// Dynamic DNS for a domain whose DNS is hosted at Fastmail
    #[command(visible_alias = "ddns")]
    DynamicDns {
        #[command(subcommand)]
        command: DynamicDnsCommand,
    },
    /// Mail over JMAP
    Mail {
        #[command(subcommand)]
        command: MailCommand,
    },
    /// Memos (private notes attached to emails), over JMAP
    Memos {
        #[command(subcommand)]
        command: MemosCommand,
    },
    /// Contacts over CardDAV
    #[cfg(feature = "dav")]
    Contacts {
        #[command(subcommand)]
        command: ContactsCommand,
    },
    /// Calendars over CalDAV
    #[cfg(feature = "dav")]
    Calendars {
        #[command(subcommand)]
        command: CalendarsCommand,
    },
    /// Files over WebDAV
    #[cfg(feature = "dav")]
    Files {
        #[command(subcommand)]
        command: FilesCommand,
    },
    /// Print each new email as one JSON line until interrupted
    Watch,
    /// Print a shell completion script (bash, zsh, fish, ...)
    #[cfg(feature = "completions")]
    Completions {
        shell: Shell,
    },
    /// Notes over IMAP
    #[cfg(feature = "notes")]
    Notes {
        #[command(subcommand)]
        command: NotesCommand,
    },
}

#[cfg(feature = "notes")]
#[derive(Subcommand)]
enum NotesCommand {
    /// List the newest notes
    List {
        #[arg(long, default_value_t = 25)]
        limit: usize,
    },
    /// Read one note by UID, as markdown (default) or raw HTML
    Get {
        uid: u32,
        #[arg(long, value_enum, default_value_t = NoteFormat::Markdown)]
        format: NoteFormat,
    },
    /// Delete a note (requires --yes). Fastmail keeps it in Trash for a while
    Delete {
        uid: u32,
        #[arg(long)]
        yes: bool,
    },
    /// Create a note from a file in markdown (default), HTML, or plain text
    Create {
        #[arg(long)]
        title: String,
        #[arg(long)]
        file: PathBuf,
        #[arg(long, value_enum, default_value_t = markdown::Format::Markdown)]
        format: markdown::Format,
    },
    /// Replace a note's body from a file in markdown (default), HTML, or plain text. Shows a diff; writes only with --apply
    Put {
        uid: u32,
        #[arg(long)]
        file: PathBuf,
        #[arg(long, value_enum, default_value_t = markdown::Format::Markdown)]
        format: markdown::Format,
        #[arg(long)]
        apply: bool,
    },
}

#[cfg(feature = "notes")]
#[derive(Clone, ValueEnum)]
enum NoteFormat {
    Markdown,
    Html,
}

#[derive(Subcommand)]
enum MailCommand {
    /// List mailboxes with message counts
    Mailboxes,
    /// Search mail, newest first
    Search {
        /// Mailbox role (inbox, sent, ...) or path (Business/Projects)
        #[arg(long)]
        mailbox: Option<String>,
        /// Full-text query
        #[arg(long)]
        text: Option<String>,
        #[arg(long, default_value_t = 25)]
        limit: u32,
    },
    /// Read one email, including its plain-text body
    Get { id: String },
    /// Save an email's attachments to a directory
    Attachments {
        id: String,
        #[arg(long, default_value = ".")]
        out: PathBuf,
    },
    /// Mark an email as read
    Read { id: String },
    /// Mark an email as unread
    Unread { id: String },
    /// Flag an email
    Flag { id: String },
    /// Remove the flag from an email
    Unflag { id: String },
    /// Move an email to a mailbox (role or path)
    Move {
        id: String,
        #[arg(long)]
        to: String,
    },
    /// Move an email to Trash
    Trash { id: String },
    /// Create a draft from a file in plain text (default), markdown, or HTML
    Draft {
        #[arg(long, value_delimiter = ',')]
        to: Vec<String>,
        #[arg(long, value_delimiter = ',', default_value = "")]
        cc: Vec<String>,
        #[arg(long)]
        subject: String,
        #[arg(long)]
        file: PathBuf,
        #[arg(long, value_enum, default_value_t = markdown::Format::Text)]
        format: markdown::Format,
        /// Attach a file (repeatable)
        #[arg(long = "attach")]
        attach: Vec<PathBuf>,
    },
    /// Reply to an email as a draft (or send with --send). Keeps the thread
    Reply {
        id: String,
        /// Reply to everyone on the original
        #[arg(long)]
        all: bool,
        #[arg(long)]
        file: PathBuf,
        #[arg(long, value_enum, default_value_t = markdown::Format::Text)]
        format: markdown::Format,
        #[arg(long = "attach")]
        attach: Vec<PathBuf>,
        /// Send instead of saving a draft (needs Email submission on the JMAP token)
        #[arg(long)]
        send: bool,
    },
    /// Send an email from a file (needs the Email submission scope on the JMAP token)
    Send {
        #[arg(long, value_delimiter = ',')]
        to: Vec<String>,
        #[arg(long, value_delimiter = ',', default_value = "")]
        cc: Vec<String>,
        #[arg(long)]
        subject: String,
        #[arg(long)]
        file: PathBuf,
        #[arg(long, value_enum, default_value_t = markdown::Format::Text)]
        format: markdown::Format,
        /// Attach a file (repeatable)
        #[arg(long = "attach")]
        attach: Vec<PathBuf>,
    },
}

#[derive(Subcommand)]
enum DynamicDnsCommand {
    /// Point a Dynamic DNS record at this machine's address, or at an address you pass
    Update {
        /// The subdomain from the Dynamic DNS entry, for example home.example.com
        hostname: String,
        /// IPv4 address, for example 203.0.113.5. Leave out to use this machine's address
        #[arg(long, conflicts_with = "ipv6")]
        ip: Option<String>,
        /// IPv6 address, for example 2001:db8::1. Leave out to use this machine's address
        #[arg(long)]
        ipv6: Option<String>,
    },
}

#[derive(Subcommand)]
enum MaskedCommand {
    /// List masked addresses
    List,
    /// Create a masked address for a site. It stays pending until mail arrives or it's enabled
    Create {
        /// The site it's for, as a URL, for example https://example.com
        #[arg(long)]
        domain: String,
        #[arg(long)]
        description: String,
        /// Optional start of the address, letters, digits, and underscores
        #[arg(long)]
        prefix: Option<String>,
    },
    /// Enable a masked address
    Enable { id: String },
    /// Disable a masked address. Mail sent to it goes to Trash
    Disable { id: String },
    /// Mark a masked address deleted (requires --yes). Mail sent to it bounces
    Delete {
        id: String,
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand)]
enum VacationCommand {
    /// Show the current vacation reply settings
    Show,
    /// Change the vacation reply. Only the fields you pass change. Replies start only with --on
    Set {
        /// Turn the reply on
        #[arg(long, conflicts_with = "off")]
        on: bool,
        /// Turn the reply off
        #[arg(long)]
        off: bool,
        /// Start date-time, UTC, for example 2026-12-20T00:00:00Z
        #[arg(long)]
        from: Option<String>,
        /// End date-time, UTC, for example 2027-01-05T00:00:00Z
        #[arg(long)]
        to: Option<String>,
        #[arg(long)]
        subject: Option<String>,
        /// Reply body from a file
        #[arg(long)]
        file: Option<PathBuf>,
        #[arg(long, value_enum, default_value_t = markdown::Format::Text)]
        format: markdown::Format,
        /// Clear fields: subject, body, from, to (comma-separated)
        #[arg(long, value_delimiter = ',', value_enum)]
        clear: Vec<VacationField>,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum VacationField {
    Subject,
    Body,
    From,
    To,
}

#[derive(Subcommand)]
enum MemosCommand {
    /// List memos
    List {
        #[arg(long, default_value_t = 25)]
        limit: u32,
    },
}

#[cfg(feature = "dav")]
#[derive(Subcommand)]
enum ContactsCommand {
    /// List contacts, optionally filtered by a substring
    List {
        #[arg(long)]
        query: Option<String>,
    },
    /// Create a contact
    Create {
        #[arg(long)]
        name: String,
        #[arg(long, value_delimiter = ',')]
        email: Vec<String>,
        #[arg(long, value_delimiter = ',')]
        phone: Vec<String>,
        #[arg(long)]
        org: Option<String>,
    },
    /// Update fields on a contact. Only the fields you pass change
    Update {
        href: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long, value_delimiter = ',')]
        email: Option<Vec<String>>,
        #[arg(long, value_delimiter = ',')]
        phone: Option<Vec<String>>,
        #[arg(long)]
        org: Option<String>,
    },
    /// Delete a contact (requires --yes)
    Delete {
        href: String,
        #[arg(long)]
        yes: bool,
    },
}

#[cfg(feature = "dav")]
#[derive(Subcommand)]
enum CalendarsCommand {
    /// List calendars
    List,
    /// List events in a calendar between two dates (YYYY-MM-DD, inclusive start, exclusive end)
    Events {
        calendar: String,
        #[arg(long)]
        from: String,
        #[arg(long)]
        to: String,
    },
    /// Create an event. Times are RFC 3339, for example 2026-11-20T15:00:00Z
    Create {
        calendar: String,
        #[arg(long)]
        title: String,
        #[arg(long)]
        start: String,
        #[arg(long)]
        end: String,
        #[arg(long)]
        location: Option<String>,
        #[arg(long)]
        description: Option<String>,
        /// Show the event that would be created, without creating it
        #[arg(long)]
        dry_run: bool,
    },
    /// Update an event by href (from `calendars events`). Only the fields you pass change
    Update {
        href: String,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        start: Option<String>,
        #[arg(long)]
        end: Option<String>,
        #[arg(long)]
        location: Option<String>,
        #[arg(long)]
        description: Option<String>,
        /// Show the event as it would be written, without changing it
        #[arg(long)]
        dry_run: bool,
    },
    /// Delete an event by href (requires --yes)
    Delete {
        href: String,
        #[arg(long)]
        yes: bool,
    },
}

#[cfg(feature = "dav")]
#[derive(Subcommand)]
enum FilesCommand {
    /// List a folder (default: top level)
    Ls {
        #[arg(default_value = "")]
        path: String,
    },
    /// Download a file
    Get {
        path: String,
        #[arg(long)]
        out: PathBuf,
    },
    /// Upload a local file to a remote path. Refuses to overwrite unless --overwrite is set
    Put {
        local: PathBuf,
        remote: String,
        #[arg(long)]
        overwrite: bool,
    },
    /// Delete a file or folder (requires --yes)
    Rm {
        path: String,
        #[arg(long)]
        yes: bool,
    },
    /// Move or rename a file or folder
    Mv {
        from: String,
        to: String,
        #[arg(long)]
        overwrite: bool,
    },
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();
    #[cfg(feature = "completions")]
    if let Command::Completions { shell } = &cli.command {
        generate(*shell, &mut Cli::command(), "fastmail", &mut std::io::stdout());
        return;
    }
    match run(cli).await {
        Ok(data) => println!("{}", json!({ "ok": true, "data": data })),
        Err(err) => {
            println!("{}", json!({ "ok": false, "error": format!("{err:#}") }));
            std::process::exit(1);
        }
    }
}

async fn run(cli: Cli) -> Result<Value> {
    let creds = config::load()?;
    let http = http::client()?;
    match cli.command {
        Command::Doctor => doctor(&http, &creds).await,
        Command::DynamicDns { command } => dynamic_dns(&http, &creds, command).await,
        Command::Vacation { command } => vacation(&http, &creds, command).await,
        Command::Masked { command } => masked(&http, &creds, command).await,
        Command::Watch => {
            watch::run(&creds).await?;
            Ok(Value::Null)
        }
        Command::Mail { command } => mail(&http, &creds, command).await,
        Command::Memos { command } => memos(&http, &creds, command).await,
        #[cfg(feature = "dav")]
        Command::Contacts { command } => contacts(&http, &creds, command).await,
        #[cfg(feature = "dav")]
        Command::Calendars { command } => calendars(&http, &creds, command).await,
        #[cfg(feature = "dav")]
        Command::Files { command } => files(&http, &creds, command).await,
        #[cfg(feature = "notes")]
        Command::Notes { command } => notes_cmd(&creds, command).await,
        #[cfg(feature = "completions")]
        Command::Completions { .. } => Ok(Value::Null),
    }
}

fn jmap_token(creds: &Credentials) -> Result<&str> {
    creds
        .jmap_token
        .as_deref()
        .context("no JMAP token: set FASTMAIL_JMAP_TOKEN or jmap.token in the config file")
}

#[cfg(feature = "dav")]
fn dav_for(http: &reqwest::Client, creds: &Credentials) -> Result<Dav> {
    let password = creds
        .dav_password
        .as_deref()
        .context("no DAV password: set FASTMAIL_DAV_PASSWORD or dav.password in the config file")?;
    Ok(Dav::new(http.clone(), &creds.username, password))
}

async fn connect(http: &reqwest::Client, creds: &Credentials) -> Result<Jmap> {
    Jmap::connect(http, jmap_token(creds)?).await
}

async fn doctor(http: &reqwest::Client, creds: &Credentials) -> Result<Value> {
    let mut report = serde_json::Map::new();
    report.insert("username".into(), json!(creds.username));

    match connect(http, creds).await {
        Ok(jmap) => {
            report.insert("jmap".into(), json!({
                "status": "ok",
                "submission_scope": jmap.has("urn:ietf:params:jmap:submission"),
                "capabilities": jmap.capabilities,
            }));
        }
        Err(err) => {
            report.insert("jmap".into(), json!({ "status": "error", "detail": format!("{err:#}") }));
        }
    }

    #[cfg(feature = "dav")]
    check_dav(http, creds, &mut report).await;

    #[cfg(feature = "notes")]
    check_imap(creds, &mut report).await?;

    report.insert("ddns".into(), json!({
        "status": if creds.ddns_token.is_some() { "configured" } else { "no token" },
    }));

    let mut features = Vec::new();
    if cfg!(feature = "dav") { features.push("dav"); }
    if cfg!(feature = "notes") { features.push("notes"); }
    if cfg!(feature = "completions") { features.push("completions"); }
    report.insert("features".into(), json!(features));
    Ok(Value::Object(report))
}

#[cfg(feature = "dav")]
async fn check_dav(http: &reqwest::Client, creds: &Credentials, report: &mut serde_json::Map<String, Value>) {
    let dav = dav_for(http, creds);
    for (name, host) in [
        ("caldav", "https://caldav.fastmail.com"),
        ("carddav", "https://carddav.fastmail.com"),
        ("webdav", "https://myfiles.fastmail.com"),
    ] {
        let status = match &dav {
            Ok(dav) => match dav.ping(host).await {
                Ok(207) => json!("ok"),
                Ok(code) => json!({ "http": code }),
                Err(err) => json!({ "error": format!("{err:#}") }),
            },
            Err(err) => json!({ "error": format!("{err:#}") }),
        };
        report.insert(name.into(), status);
    }
}

#[cfg(feature = "notes")]
async fn check_imap(creds: &Credentials, report: &mut serde_json::Map<String, Value>) -> Result<()> {
    let imap = match creds.imap_password.as_deref() {
        Some(password) => {
            let (user, pass) = (creds.username.clone(), password.to_string());
            match tokio::task::spawn_blocking(move || notes::ping(&user, &pass)).await? {
                Ok(()) => json!("ok"),
                Err(err) => json!({ "error": format!("{err:#}") }),
            }
        }
        None => json!("not configured (set FASTMAIL_IMAP_PASSWORD to enable notes)"),
    };
    report.insert("imap".into(), imap);
    Ok(())
}

async fn mail(http: &reqwest::Client, creds: &Credentials, command: MailCommand) -> Result<Value> {
    let jmap = connect(http, creds).await?;
    match command {
        MailCommand::Mailboxes => {
            let boxes = jmap.mailboxes().await?;
            Ok(json!(boxes.iter().map(|b| json!({
                "name": b["name"],
                "role": b["role"],
                "total": b["totalEmails"],
                "unread": b["unreadEmails"],
                "id": b["id"],
            })).collect::<Vec<_>>()))
        }
        MailCommand::Search { mailbox, text, limit } => {
            Ok(json!(jmap.search(mailbox.as_deref(), text.as_deref(), limit).await?))
        }
        MailCommand::Get { id } => jmap.get(&id).await,
        MailCommand::Attachments { id, out } => {
            let attachments = jmap.attachments(&id).await?;
            std::fs::create_dir_all(&out).with_context(|| format!("creating {}", out.display()))?;
            let mut saved = Vec::new();
            for att in attachments {
                let name = att["name"].as_str().unwrap_or("attachment");
                let mime = att["type"].as_str().unwrap_or("application/octet-stream");
                let blob = att["blobId"].as_str().context("attachment has no blobId")?;
                let bytes = jmap.download(blob, name, mime).await?;
                let path = unique_path(&out, &safe_filename(name));
                std::fs::write(&path, &bytes).with_context(|| format!("writing {}", path.display()))?;
                saved.push(json!({ "path": path, "bytes": bytes.len() }));
            }
            Ok(json!(saved))
        }
        MailCommand::Read { id } => {
            jmap.set_keyword(&id, "$seen", true).await?;
            Ok(json!({ "id": id, "read": true }))
        }
        MailCommand::Unread { id } => {
            jmap.set_keyword(&id, "$seen", false).await?;
            Ok(json!({ "id": id, "read": false }))
        }
        MailCommand::Flag { id } => {
            jmap.set_keyword(&id, "$flagged", true).await?;
            Ok(json!({ "id": id, "flagged": true }))
        }
        MailCommand::Unflag { id } => {
            jmap.set_keyword(&id, "$flagged", false).await?;
            Ok(json!({ "id": id, "flagged": false }))
        }
        MailCommand::Move { id, to } => {
            jmap.move_email(&id, &to).await?;
            Ok(json!({ "id": id, "moved_to": to }))
        }
        MailCommand::Trash { id } => {
            jmap.trash(&id).await?;
            Ok(json!({ "id": id, "trashed": true }))
        }
        MailCommand::Draft { to, cc, subject, file, format, attach } => {
            let text = std::fs::read_to_string(&file).with_context(|| format!("reading {}", file.display()))?;
            let body = markdown::body(format, &text);
            let cc: Vec<String> = cc.into_iter().filter(|c| !c.is_empty()).collect();
            let from = jmap.identity_email(&creds.username).await?;
            let draft = jmap::Draft {
                from: from.clone(),
                to,
                cc,
                subject,
                text: body.text,
                html: body.html,
                attachments: load_attachments(&attach)?,
                in_reply_to: vec![],
                references: vec![],
            };
            let id = jmap.create_draft(&draft).await?;
            Ok(json!({ "id": id, "from": from }))
        }
        MailCommand::Send { to, cc, subject, file, format, attach } => {
            let cc: Vec<String> = cc.into_iter().filter(|c| !c.is_empty()).collect();
            let all: Vec<String> = to.iter().chain(cc.iter()).cloned().collect();
            allowlist::check(&creds.allowed_recipients, &all)?;
            let text = std::fs::read_to_string(&file).with_context(|| format!("reading {}", file.display()))?;
            let body = markdown::body(format, &text);
            let from = jmap.identity_email(&creds.username).await?;
            let draft = jmap::Draft {
                from: from.clone(),
                to,
                cc,
                subject,
                text: body.text,
                html: body.html,
                attachments: load_attachments(&attach)?,
                in_reply_to: vec![],
                references: vec![],
            };
            let id = jmap.create_draft(&draft).await?;
            if let Err(err) = jmap.send_draft(&id, &from).await {
                bail!("{err:#}. The message was saved as a draft (id {id}) and not sent");
            }
            Ok(json!({ "id": id, "sent": true }))
        }
        MailCommand::Reply { id, all, file, format, attach, send } => {
            let orig = jmap.reply_context(&id).await?;
            let me = jmap.identity_email(&creds.username).await?;
            let mut to = if orig["replyTo"].is_null() { addresses(&orig["from"]) } else { addresses(&orig["replyTo"]) };
            let mut cc = Vec::new();
            if all {
                to.extend(addresses(&orig["to"]));
                cc = addresses(&orig["cc"]);
            }
            let mut seen = std::collections::HashSet::new();
            to.retain(|a| a.to_ascii_lowercase() != me.to_ascii_lowercase() && seen.insert(a.to_ascii_lowercase()));
            cc.retain(|a| a.to_ascii_lowercase() != me.to_ascii_lowercase() && seen.insert(a.to_ascii_lowercase()));
            let subject = reply_subject(orig["subject"].as_str().unwrap_or(""));
            let msg_ids: Vec<String> = strings(&orig["messageId"]);
            let mut references = strings(&orig["references"]);
            references.extend(msg_ids.iter().cloned());
            let text = std::fs::read_to_string(&file).with_context(|| format!("reading {}", file.display()))?;
            let body = markdown::body(format, &text);
            let draft = jmap::Draft {
                from: me.clone(),
                to: to.clone(),
                cc: cc.clone(),
                subject: subject.clone(),
                text: body.text,
                html: body.html,
                attachments: load_attachments(&attach)?,
                in_reply_to: msg_ids,
                references,
            };
            let draft_id = jmap.create_draft(&draft).await?;
            if send {
                let all_recipients: Vec<String> = to.iter().chain(cc.iter()).cloned().collect();
                allowlist::check(&creds.allowed_recipients, &all_recipients)?;
                if let Err(err) = jmap.send_draft(&draft_id, &me).await {
                    bail!("{err:#}. The reply was saved as a draft (id {draft_id}) and not sent");
                }
            }
            Ok(json!({ "id": draft_id, "to": to, "cc": cc, "subject": subject, "sent": send }))
        }
    }
}

async fn dynamic_dns(http: &reqwest::Client, creds: &Credentials, command: DynamicDnsCommand) -> Result<Value> {
    let token = creds
        .ddns_token
        .as_deref()
        .context("no Dynamic DNS token: set FASTMAIL_DDNS_TOKEN or ddns.token in the config file")?;
    match command {
        DynamicDnsCommand::Update { hostname, ip, ipv6 } => {
            ddns::update(
                http,
                &creds.username,
                token,
                ddns::Update { hostname: &hostname, ipv4: ip.as_deref(), ipv6: ipv6.as_deref() },
            )
            .await
        }
    }
}

async fn masked(http: &reqwest::Client, creds: &Credentials, command: MaskedCommand) -> Result<Value> {
    let jmap = connect(http, creds).await?;
    match command {
        MaskedCommand::List => Ok(json!(jmap.masked_list().await?)),
        MaskedCommand::Create { domain, description, prefix } => {
            Ok(jmap.masked_create(&domain, &description, prefix.as_deref()).await?)
        }
        MaskedCommand::Enable { id } => {
            jmap.masked_set_state(&id, "enabled").await?;
            Ok(json!({ "id": id, "state": "enabled" }))
        }
        MaskedCommand::Disable { id } => {
            jmap.masked_set_state(&id, "disabled").await?;
            Ok(json!({ "id": id, "state": "disabled" }))
        }
        MaskedCommand::Delete { id, yes } => {
            require_yes(yes, "masked address")?;
            jmap.masked_set_state(&id, "deleted").await?;
            Ok(json!({ "id": id, "state": "deleted" }))
        }
    }
}

async fn vacation(http: &reqwest::Client, creds: &Credentials, command: VacationCommand) -> Result<Value> {
    let jmap = connect(http, creds).await?;
    match command {
        VacationCommand::Show => jmap.vacation().await,
        VacationCommand::Set { on, off, from, to, subject, file, format, clear } => {
            let mut patch = serde_json::Map::new();
            for field in clear {
                match field {
                    VacationField::Subject => patch.insert("subject".into(), Value::Null),
                    VacationField::Body => {
                        patch.insert("textBody".into(), Value::Null);
                        patch.insert("htmlBody".into(), Value::Null)
                    }
                    VacationField::From => patch.insert("fromDate".into(), Value::Null),
                    VacationField::To => patch.insert("toDate".into(), Value::Null),
                };
            }
            if on {
                patch.insert("isEnabled".into(), json!(true));
            }
            if off {
                patch.insert("isEnabled".into(), json!(false));
            }
            if let Some(f) = from {
                patch.insert("fromDate".into(), json!(f));
            }
            if let Some(t) = to {
                patch.insert("toDate".into(), json!(t));
            }
            if let Some(s) = subject {
                patch.insert("subject".into(), json!(s));
            }
            if let Some(path) = file {
                let text = std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
                let body = markdown::body(format, &text);
                patch.insert("textBody".into(), json!(body.text));
                patch.insert("htmlBody".into(), json!(body.html));
            }
            if patch.is_empty() {
                bail!("nothing to change: pass --on/--off, --from, --to, --subject, --file, or --clear");
            }
            jmap.set_vacation(Value::Object(patch)).await?;
            jmap.vacation().await
        }
    }
}

async fn memos(http: &reqwest::Client, creds: &Credentials, command: MemosCommand) -> Result<Value> {
    let jmap = connect(http, creds).await?;
    match command {
        MemosCommand::List { limit } => Ok(json!(jmap.search(Some("memos"), None, limit).await?)),
    }
}

#[cfg(feature = "dav")]
async fn contacts(http: &reqwest::Client, creds: &Credentials, command: ContactsCommand) -> Result<Value> {
    let dav = dav_for(http, creds)?;
    match command {
        ContactsCommand::List { query } => Ok(json!(dav.contacts(query.as_deref()).await?)),
        ContactsCommand::Create { name, email, phone, org } => {
            let href = dav.contact_create(&name, &email, &phone, org.as_deref()).await?;
            Ok(json!({ "href": href }))
        }
        ContactsCommand::Update { href, name, email, phone, org } => {
            dav.contact_update(&href, name.as_deref(), email.as_deref(), phone.as_deref(), org.as_deref()).await?;
            Ok(json!({ "href": href, "updated": true }))
        }
        ContactsCommand::Delete { href, yes } => {
            require_yes(yes, "contact")?;
            dav.contact_delete(&href).await?;
            Ok(json!({ "deleted": href }))
        }
    }
}

#[cfg(feature = "dav")]
async fn calendars(http: &reqwest::Client, creds: &Credentials, command: CalendarsCommand) -> Result<Value> {
    let dav = dav_for(http, creds)?;
    match command {
        CalendarsCommand::List => Ok(json!(dav.calendars().await?)),
        CalendarsCommand::Create { calendar, title, start, end, location, description, dry_run } => {
            let href = find_calendar(&dav, &calendar).await?;
            dav.event_create(&href, &title, &start, &end, location.as_deref(), description.as_deref(), dry_run).await
        }
        CalendarsCommand::Update { href, title, start, end, location, description, dry_run } => {
            dav.event_update(&href, title.as_deref(), start.as_deref(), end.as_deref(), location.as_deref(), description.as_deref(), dry_run).await
        }
        CalendarsCommand::Delete { href, yes } => {
            require_yes(yes, "event")?;
            dav.event_delete(&href).await?;
            Ok(json!({ "deleted": href }))
        }
        CalendarsCommand::Events { calendar, from, to } => {
            let href = find_calendar(&dav, &calendar).await?;
            let start = ical_date(&from)?;
            let end = ical_date(&to)?;
            Ok(json!(dav.events(&href, &start, &end).await?))
        }
    }
}

#[cfg(feature = "notes")]
async fn notes_cmd(creds: &Credentials, command: NotesCommand) -> Result<Value> {
    let password = creds
        .imap_password
        .clone()
        .context("no IMAP password: set FASTMAIL_IMAP_PASSWORD or imap.password in the config file")?;
    let user = creds.username.clone();
    match command {
        NotesCommand::List { limit } => {
            let notes = tokio::task::spawn_blocking(move || notes::list(&user, &password, limit)).await??;
            Ok(json!(notes))
        }
        NotesCommand::Get { uid, format } => {
            let note = tokio::task::spawn_blocking(move || notes::get(&user, &password, uid)).await??;
            let html = note["html"].as_str().unwrap_or_default();
            match format {
                NoteFormat::Html => Ok(note),
                NoteFormat::Markdown => {
                    let converted = markdown::to_markdown(html);
                    Ok(json!({
                        "uid": uid,
                        "subject": note["subject"],
                        "date": note["date"],
                        "markdown": converted.markdown,
                        "warnings": converted.warnings,
                    }))
                }
            }
        }
        NotesCommand::Delete { uid, yes } => {
            require_yes(yes, "note")?;
            tokio::task::spawn_blocking(move || notes::delete(&user, &password, uid)).await??;
            Ok(json!({ "deleted": uid }))
        }
        NotesCommand::Create { title, file, format } => {
            let body = note_html(&file, format)?;
            let uid = tokio::task::spawn_blocking(move || notes::create(&user, &password, &title, &body)).await??;
            Ok(json!({ "uid": uid }))
        }
        NotesCommand::Put { uid, file, format, apply } => {
            let body = note_html(&file, format)?;
            Ok(tokio::task::spawn_blocking(move || notes::put(&user, &password, uid, &body, apply)).await??)
        }
    }
}

fn require_yes(yes: bool, what: &str) -> Result<()> {
    if !yes {
        bail!("refusing to delete the {what} without --yes");
    }
    Ok(())
}

/// Reads content from a file and returns the HTML to store in a note.
#[cfg(feature = "notes")]
fn note_html(file: &std::path::Path, format: markdown::Format) -> Result<String> {
    let text = std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
    let body = markdown::body(format, &text);
    Ok(body.html.unwrap_or_else(|| markdown::text_to_html(&text)))
}

#[cfg(feature = "dav")]
async fn find_calendar(dav: &Dav, name: &str) -> Result<String> {
    let calendars = dav.calendars().await?;
    let matched = calendars
        .iter()
        .find(|c| c["name"].as_str().is_some_and(|n| n.eq_ignore_ascii_case(name)));
    match matched {
        Some(c) => Ok(c["href"].as_str().unwrap_or_default().to_string()),
        None => {
            let names: Vec<&str> = calendars.iter().filter_map(|c| c["name"].as_str()).collect();
            bail!("no calendar named '{name}'. Available: {}", names.join(", "))
        }
    }
}

/// Converts YYYY-MM-DD to the iCalendar UTC form used in CalDAV time ranges.
#[cfg(feature = "dav")]
fn ical_date(input: &str) -> Result<String> {
    let date = chrono::NaiveDate::parse_from_str(input, "%Y-%m-%d")
        .with_context(|| format!("'{input}' is not a YYYY-MM-DD date"))?;
    Ok(date.format("%Y%m%dT000000Z").to_string())
}

#[cfg(feature = "dav")]
async fn files(http: &reqwest::Client, creds: &Credentials, command: FilesCommand) -> Result<Value> {
    let dav = dav_for(http, creds)?;
    match command {
        FilesCommand::Ls { path } => Ok(json!(dav.files_ls(&path).await?)),
        FilesCommand::Rm { path, yes } => {
            require_yes(yes, "file")?;
            dav.file_delete(&path).await?;
            Ok(json!({ "deleted": path }))
        }
        FilesCommand::Mv { from, to, overwrite } => {
            let dst = dav.file_move(&from, &to, overwrite).await?;
            Ok(json!({ "from": from, "to": dst }))
        }
        FilesCommand::Put { local, remote, overwrite } => {
            let bytes = std::fs::read(&local).with_context(|| format!("reading {}", local.display()))?;
            let size = bytes.len();
            let url = dav.files_put(&remote, bytes, overwrite).await?;
            Ok(json!({ "url": url, "bytes": size }))
        }
        FilesCommand::Get { path, out } => {
            let bytes = dav.files_get(&path).await?;
            let target = if out.is_dir() {
                let name = path.rsplit('/').next().unwrap_or("file");
                unique_path(&out, &safe_filename(name))
            } else {
                out
            };
            std::fs::write(&target, &bytes).with_context(|| format!("writing {}", target.display()))?;
            Ok(json!({ "path": target, "bytes": bytes.len() }))
        }
    }
}

/// Email addresses from a JMAP address list.
fn addresses(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|list| list.iter().filter_map(|a| a["email"].as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|list| list.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

/// "Re: " in front of a subject, unless it's already there.
fn reply_subject(subject: &str) -> String {
    if subject.to_ascii_lowercase().starts_with("re:") {
        subject.to_string()
    } else {
        format!("Re: {subject}")
    }
}

/// Reads files to attach, taking the MIME type from the file name.
fn load_attachments(paths: &[PathBuf]) -> Result<Vec<jmap::Attachment>> {
    paths
        .iter()
        .map(|path| {
            let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("attachment").to_string();
            let mime = mime_for(&name).to_string();
            Ok(jmap::Attachment { name, mime, bytes })
        })
        .collect()
}

fn mime_for(name: &str) -> &'static str {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "pdf" => "application/pdf",
        "txt" => "text/plain",
        "md" => "text/markdown",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "json" => "application/json",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "heic" => "image/heic",
        "zip" => "application/zip",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        _ => "application/octet-stream",
    }
}

/// Keeps only the final path component, so an attachment named `../../x` can't escape the output folder.
fn safe_filename(raw: &str) -> String {
    let base = raw.rsplit(['/', '\\']).next().unwrap_or("").trim();
    if base.is_empty() || base == "." || base == ".." {
        "attachment".to_string()
    } else {
        base.to_string()
    }
}

/// Returns a path in `dir` that doesn't exist yet, adding " (2)", " (3)", ... if needed.
fn unique_path(dir: &std::path::Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (name.to_string(), String::new()),
    };
    (2..)
        .map(|n| dir.join(format!("{stem} ({n}){ext}")))
        .find(|p| !p.exists())
        .expect("an unused name exists")
}
