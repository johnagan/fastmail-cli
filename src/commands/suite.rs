use crate::{config::Config, dav::DavClient, jmap::authenticated_client, models::Output};
use anyhow::{Context, Result, ensure};
use clap::Subcommand;
use serde_json::json;
use std::path::PathBuf;

#[derive(Subcommand)]
pub enum CalendarCommands {
    /// Create a calendar collection at the given href
    CreateCalendar {
        href: String,
        #[arg(long)]
        name: String,
        #[arg(long)]
        description: Option<String>,
        #[arg(long)]
        color: Option<String>,
    },
    UpdateCalendar {
        href: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        description: Option<String>,
        #[arg(long)]
        color: Option<String>,
    },
    /// Delete a calendar and its events
    DeleteCalendar {
        href: String,
        #[arg(short = 'y', long)]
        yes: bool,
    },
    FreeBusy {
        calendar: String,
        #[arg(long)]
        after: String,
        #[arg(long)]
        before: String,
    },
    /// Discover scheduling inbox, outbox, and calendar-user addresses
    Scheduling,
    /// Send an iTIP .ics REQUEST, REPLY, or CANCEL through the scheduling outbox
    Schedule {
        file: PathBuf,
        #[arg(long)]
        outbox: String,
        #[arg(long)]
        originator: String,
        #[arg(long = "recipient", required = true)]
        recipients: Vec<String>,
    },
    /// List calendars and their CalDAV hrefs
    List,
    /// List events as iCalendar data (use --expand for recurrence instances)
    Events {
        /// Expand recurring events within --after/--before on the server
        #[arg(long)]
        expand: bool,
        calendar: String,
        /// UTC lower bound, e.g. 20261016T000000Z
        #[arg(long)]
        after: Option<String>,
        /// UTC upper bound, e.g. 20261116T000000Z
        #[arg(long)]
        before: Option<String>,
    },
    /// Read an event by its CalDAV href
    Get { href: String },
    /// Create an event from a complete .ics file
    Create { calendar: String, file: PathBuf },
    /// Replace an event with a complete .ics file; requires the current ETag
    Update {
        href: String,
        file: PathBuf,
        #[arg(long)]
        etag: String,
    },
    /// Delete an event by its CalDAV href
    Delete {
        href: String,
        #[arg(long)]
        etag: Option<String>,
        #[arg(short = 'y', long)]
        yes: bool,
    },
}

#[derive(Subcommand)]
pub enum FilesCommands {
    /// Server-side move or rename
    #[command(alias = "rename")]
    Move {
        source: String,
        destination: String,
        #[arg(long)]
        overwrite: bool,
    },
    /// Server-side copy (including folders)
    Copy {
        source: String,
        destination: String,
        #[arg(long)]
        overwrite: bool,
    },
    /// List files and folders (paths are relative to your Fastmail Files root)
    List {
        #[arg(default_value = "/")]
        path: String,
    },
    /// Download a file to an explicit local path
    Download {
        path: String,
        #[arg(short, long)]
        output: PathBuf,
        #[arg(long)]
        overwrite: bool,
    },
    /// Upload a local file to a remote path
    Upload {
        file: PathBuf,
        path: String,
        #[arg(long)]
        overwrite: bool,
    },
    /// Create a folder
    Mkdir { path: String },
    /// Delete a file or folder (folders include all contents)
    Delete {
        path: String,
        #[arg(short = 'y', long)]
        yes: bool,
    },
}

#[derive(Subcommand)]
pub enum NotesCommands {
    /// List notes, newest first
    List {
        #[arg(short, long, default_value = "50")]
        limit: u32,
    },
    /// Search note titles and bodies
    Search {
        query: String,
        #[arg(short, long, default_value = "50")]
        limit: u32,
    },
    /// Read a note, including body values
    Get { id: String },
    /// Create a plain-text or HTML note with optional attachments
    Create {
        #[arg(long)]
        title: String,
        #[arg(long)]
        body: Option<String>,
        #[arg(long, conflicts_with = "html_file")]
        html_body: Option<String>,
        #[arg(long)]
        html_file: Option<String>,
        #[arg(short = 'a', long = "attachment")]
        attachments: Vec<String>,
        #[arg(long = "attachment-manifest")]
        attachment_manifests: Vec<String>,
    },
    /// Edit a rich note; returns a new message ID
    Update {
        id: String,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        body: Option<String>,
        #[arg(long, conflicts_with = "html_file")]
        html_body: Option<String>,
        #[arg(long)]
        html_file: Option<String>,
        #[arg(short = 'a', long = "attachment", conflicts_with = "clear_attachments")]
        attachments: Vec<String>,
        #[arg(long = "attachment-manifest", conflicts_with = "clear_attachments")]
        attachment_manifests: Vec<String>,
        #[arg(long)]
        clear_attachments: bool,
    },
    /// Permanently delete a note
    Delete {
        id: String,
        #[arg(short = 'y', long)]
        yes: bool,
    },
}

fn dav_client(base: &str) -> Result<DavClient> {
    let config = Config::load()?;
    let (username, password) = config.get_dav_credentials()?;
    DavClient::new(base, username, password)
}

fn confirm(yes: bool) -> Result<()> {
    ensure!(yes, "Deletion requires -y / --yes to confirm");
    Ok(())
}

async fn calendar_file(file: &PathBuf) -> Result<Vec<u8>> {
    let data = tokio::fs::read(file)
        .await
        .context("Could not read iCalendar file")?;
    let text = std::str::from_utf8(&data).context("iCalendar file must be UTF-8")?;
    ensure!(
        text.lines().any(|l| l.trim_end() == "BEGIN:VCALENDAR")
            && text.lines().any(|l| l.trim_end() == "BEGIN:VEVENT")
            && text.lines().any(|l| l.trim_end() == "END:VCALENDAR"),
        "Expected a complete VCALENDAR containing a VEVENT"
    );
    Ok(data)
}

pub async fn calendar(command: CalendarCommands) -> Result<()> {
    if let CalendarCommands::Delete { yes, .. } | CalendarCommands::DeleteCalendar { yes, .. } =
        &command
    {
        confirm(*yes)?;
    }
    let client = dav_client("https://caldav.fastmail.com/")?;
    let result = match command {
        CalendarCommands::CreateCalendar {
            href,
            name,
            description,
            color,
        } => {
            client
                .create_calendar(&href, &name, description.as_deref(), color.as_deref())
                .await?;
            json!({"href": href})
        }
        CalendarCommands::UpdateCalendar {
            href,
            name,
            description,
            color,
        } => {
            ensure!(
                href.ends_with('/'),
                "Use a calendar collection href ending with /"
            );
            client
                .update_calendar(
                    &href,
                    name.as_deref(),
                    description.as_deref(),
                    color.as_deref(),
                )
                .await?;
            json!({"href": href})
        }
        CalendarCommands::DeleteCalendar { href, .. } => {
            let calendars = client.calendars().await?;
            ensure!(
                calendars.iter().any(|c| c.href == href),
                "Use a calendar href returned by calendar list"
            );
            client.delete(&href, None).await?;
            json!({"deleted": href})
        }
        CalendarCommands::FreeBusy {
            calendar,
            after,
            before,
        } => json!({"calendarData": client.free_busy(&calendar, &after, &before).await?}),
        CalendarCommands::Scheduling => client.scheduling().await?,
        CalendarCommands::Schedule {
            file,
            outbox,
            originator,
            recipients,
        } => {
            let data = calendar_file(&file).await?;
            let text = std::str::from_utf8(&data)?;
            ensure!(
                text.lines()
                    .any(|line| line.trim_end().starts_with("METHOD:")),
                "Scheduling requires an iTIP METHOD property"
            );
            client
                .schedule(&outbox, &originator, &recipients, data)
                .await?
        }
        CalendarCommands::List => serde_json::to_value(client.calendars().await?)?,
        CalendarCommands::Events {
            expand,
            calendar,
            after,
            before,
        } => serde_json::to_value(
            client
                .events_expanded(&calendar, after.as_deref(), before.as_deref(), expand)
                .await?,
        )?,
        CalendarCommands::Get { href } => {
            let (data, etag) = client.get(&href).await?;
            json!({"href": href, "etag": etag, "calendarData": String::from_utf8(data)?})
        }
        CalendarCommands::Create { calendar, file } => {
            ensure!(calendar.ends_with('/'), "Calendar href must end with /");
            let href = format!("{calendar}{}.ics", uuid::Uuid::new_v4());
            let etag = client
                .put(
                    &href,
                    calendar_file(&file).await?,
                    "text/calendar; charset=utf-8",
                    None,
                    false,
                )
                .await?;
            json!({"href": href, "etag": etag})
        }
        CalendarCommands::Update { href, file, etag } => {
            let etag = client
                .put(
                    &href,
                    calendar_file(&file).await?,
                    "text/calendar; charset=utf-8",
                    Some(&etag),
                    false,
                )
                .await?;
            json!({"href": href, "etag": etag})
        }
        CalendarCommands::Delete { href, etag, .. } => {
            ensure!(
                href.trim_end_matches('/').ends_with(".ics"),
                "Use an event .ics href, not a calendar collection"
            );
            client.delete(&href, etag.as_deref()).await?;
            json!({"deleted": href})
        }
    };
    Output::success(result).print();
    Ok(())
}

pub async fn files(command: FilesCommands) -> Result<()> {
    if let FilesCommands::Delete { yes, .. } = &command {
        confirm(*yes)?;
    }
    let client = dav_client("https://myfiles.fastmail.com/")?;
    let result = match command {
        FilesCommands::Move {
            source,
            destination,
            overwrite,
        } => {
            client
                .transfer(
                    &DavClient::file_path(&source)?,
                    &DavClient::file_path(&destination)?,
                    false,
                    overwrite,
                )
                .await?;
            json!({"source": source, "destination": destination, "moved": true})
        }
        FilesCommands::Copy {
            source,
            destination,
            overwrite,
        } => {
            client
                .transfer(
                    &DavClient::file_path(&source)?,
                    &DavClient::file_path(&destination)?,
                    true,
                    overwrite,
                )
                .await?;
            json!({"source": source, "destination": destination, "copied": true})
        }
        FilesCommands::List { path } => {
            let folder = format!("{}/", path.trim_end_matches('/'));
            let resources = client.list(&DavClient::file_path(&folder)?).await?;
            let base = reqwest::Url::parse("https://myfiles.fastmail.com/")?;
            let mut entries = Vec::new();
            for resource in resources {
                let url = base.join(&resource.href)?;
                let path = percent_encoding::percent_decode_str(url.path()).decode_utf8()?;
                let mut entry = serde_json::to_value(resource)?;
                entry["path"] = json!(path);
                entries.push(entry);
            }
            json!(entries)
        }
        FilesCommands::Download {
            path,
            output,
            overwrite,
        } => {
            let (data, _) = client.get(&DavClient::file_path(&path)?).await?;
            let mut options = tokio::fs::OpenOptions::new();
            options.write(true);
            if overwrite {
                options.create(true).truncate(true);
            } else {
                options.create_new(true);
            }
            let mut file = options
                .open(&output)
                .await
                .context("Cannot open output file (use --overwrite to replace)")?;
            use tokio::io::AsyncWriteExt;
            file.write_all(&data).await?;
            file.flush().await?;
            json!({"path": path, "output": output, "size": data.len()})
        }
        FilesCommands::Upload {
            file,
            path,
            overwrite,
        } => {
            let data = tokio::fs::read(&file).await?;
            let size = data.len();
            let etag = client
                .put(
                    &DavClient::file_path(&path)?,
                    data,
                    "application/octet-stream",
                    None,
                    overwrite,
                )
                .await?;
            json!({"path": path, "etag": etag, "size": size})
        }
        FilesCommands::Mkdir { path } => {
            client.mkdir(&DavClient::file_path(&path)?).await?;
            json!({"path": path})
        }
        FilesCommands::Delete { path, .. } => {
            client.delete(&DavClient::file_path(&path)?, None).await?;
            json!({"deleted": path})
        }
    };
    Output::success(result).print();
    Ok(())
}

pub async fn notes(command: NotesCommands) -> Result<()> {
    if let NotesCommands::Delete { yes, .. } = &command {
        confirm(*yes)?;
    }
    let client = authenticated_client().await?;
    let result = match command {
        NotesCommands::List { limit } => client.list_notes(None, limit).await?,
        NotesCommands::Search { query, limit } => client.list_notes(Some(&query), limit).await?,
        NotesCommands::Get { id } => client.get_note(&id).await?,
        NotesCommands::Create {
            title,
            body,
            html_body,
            html_file,
            attachments,
            attachment_manifests,
        } => {
            let html_body = crate::util::resolve_html(html_body, html_file)?;
            ensure!(
                body.is_some() || html_body.is_some(),
                "Provide --body or --html-body/--html-file"
            );
            client
                .create_rich_note(crate::jmap::NoteEdit {
                    title: Some(title),
                    body,
                    html_body,
                    attachments: Some(crate::util::load_attachments(
                        &attachments,
                        &attachment_manifests,
                    )?),
                })
                .await?
        }
        NotesCommands::Update {
            id,
            title,
            body,
            html_body,
            html_file,
            attachments,
            attachment_manifests,
            clear_attachments,
        } => {
            let replacing =
                clear_attachments || !attachments.is_empty() || !attachment_manifests.is_empty();
            let attachments = if replacing {
                Some(crate::util::load_attachments(
                    &attachments,
                    &attachment_manifests,
                )?)
            } else {
                None
            };
            client
                .update_rich_note(
                    &id,
                    crate::jmap::NoteEdit {
                        title,
                        body,
                        html_body: crate::util::resolve_html(html_body, html_file)?,
                        attachments,
                    },
                )
                .await?
        }
        NotesCommands::Delete { id, .. } => {
            client.delete_note(&id).await?;
            json!({"deleted": id})
        }
    };
    Output::success(result).print();
    Ok(())
}
