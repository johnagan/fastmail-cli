use crate::{
    jmap::{EmailQuery, authenticated_client},
    models::Output,
};
use anyhow::{Result, ensure};
use clap::Subcommand;
use serde_json::{Value, json};
use std::path::PathBuf;

#[derive(Subcommand)]
pub enum MailboxCommands {
    /// List mailboxes and their IDs
    List,
    /// Create a mailbox (parent is a mailbox ID)
    Create {
        name: String,
        #[arg(long)]
        parent: Option<String>,
        #[arg(long)]
        role: Option<String>,
        #[arg(long)]
        sort_order: Option<u32>,
    },
    /// Edit a mailbox by ID; --root removes its parent
    Update {
        id: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long, conflicts_with = "root")]
        parent: Option<String>,
        #[arg(long)]
        root: bool,
        #[arg(long)]
        sort_order: Option<u32>,
        #[arg(long, action = clap::ArgAction::Set)]
        subscribed: Option<bool>,
    },
    /// Delete a mailbox by ID
    Delete {
        id: String,
        /// Remove its emails too (otherwise nonempty mailboxes are rejected)
        #[arg(long)]
        remove_emails: bool,
        #[arg(short = 'y', long)]
        yes: bool,
    },
}

#[derive(Subcommand)]
pub enum VacationCommands {
    /// Read the current vacation response
    Get,
    /// Change response settings; omitted settings are preserved
    Set {
        #[arg(long, action = clap::ArgAction::Set)]
        enabled: Option<bool>,
        #[arg(long)]
        subject: Option<String>,
        #[arg(long)]
        text_body: Option<String>,
        #[arg(long)]
        html_body: Option<String>,
        /// Start instant in RFC 3339 format
        #[arg(long, conflicts_with = "clear_from")]
        from: Option<String>,
        /// End instant in RFC 3339 format
        #[arg(long, conflicts_with = "clear_to")]
        to: Option<String>,
        #[arg(long)]
        clear_from: bool,
        #[arg(long)]
        clear_to: bool,
    },
    Enable,
    Disable,
}

pub async fn mailbox(command: MailboxCommands) -> Result<()> {
    if matches!(command, MailboxCommands::Delete { yes: false, .. }) {
        anyhow::bail!("Deletion requires -y / --yes to confirm");
    }
    let mut client = authenticated_client().await?;
    let result = match command {
        MailboxCommands::List => serde_json::to_value(client.list_mailboxes().await?)?,
        MailboxCommands::Create {
            name,
            parent,
            role,
            sort_order,
        } => {
            let mut p = json!({"name": name});
            if let Some(parent) = parent {
                p["parentId"] = json!(parent);
            }
            if let Some(role) = role {
                p["role"] = json!(role);
            }
            if let Some(order) = sort_order {
                p["sortOrder"] = json!(order);
            }
            client.create_mailbox(p).await?
        }
        MailboxCommands::Update {
            id,
            name,
            parent,
            root,
            sort_order,
            subscribed,
        } => {
            let mut p = json!({});
            if let Some(name) = name {
                p["name"] = json!(name);
            }
            if let Some(parent) = parent {
                p["parentId"] = json!(parent);
            }
            if root {
                p["parentId"] = Value::Null;
            }
            if let Some(order) = sort_order {
                p["sortOrder"] = json!(order);
            }
            if let Some(subscribed) = subscribed {
                p["isSubscribed"] = json!(subscribed);
            }
            ensure!(
                !p.as_object().unwrap().is_empty(),
                "Provide at least one mailbox change"
            );
            client.update_mailbox(&id, p).await?
        }
        MailboxCommands::Delete {
            id, remove_emails, ..
        } => client.delete_mailbox(&id, remove_emails).await?,
    };
    Output::success(result).print();
    Ok(())
}

pub async fn vacation(command: VacationCommands) -> Result<()> {
    let client = authenticated_client().await?;
    let result = match command {
        VacationCommands::Get => client.vacation_response().await?,
        VacationCommands::Enable => {
            client
                .set_vacation_response(json!({"isEnabled": true}))
                .await?
        }
        VacationCommands::Disable => {
            client
                .set_vacation_response(json!({"isEnabled": false}))
                .await?
        }
        VacationCommands::Set {
            enabled,
            subject,
            text_body,
            html_body,
            from,
            to,
            clear_from,
            clear_to,
        } => {
            let mut p = json!({});
            if let Some(enabled) = enabled {
                p["isEnabled"] = json!(enabled);
            }
            for (key, value) in [
                ("subject", subject),
                ("textBody", text_body),
                ("htmlBody", html_body),
                ("fromDate", from),
                ("toDate", to),
            ] {
                if let Some(value) = value {
                    p[key] = json!(value);
                }
            }
            if clear_from {
                p["fromDate"] = Value::Null;
            }
            if clear_to {
                p["toDate"] = Value::Null;
            }
            ensure!(
                !p.as_object().unwrap().is_empty(),
                "Provide at least one vacation response change"
            );
            client.set_vacation_response(p).await?
        }
    };
    Output::success(result).print();
    Ok(())
}

pub async fn import_message(
    file: PathBuf,
    mailbox: &str,
    keywords: Vec<String>,
    received_at: Option<&str>,
) -> Result<()> {
    let bytes = tokio::fs::read(file).await?;
    ensure!(!bytes.is_empty(), "Message file is empty");
    let mut client = authenticated_client().await?;
    let mailbox = client.find_mailbox(mailbox).await?;
    let keywords = Value::Object(keywords.into_iter().map(|k| (k, json!(true))).collect());
    let result = client
        .import_email(bytes, &mailbox.id, keywords, received_at)
        .await?;
    Output::success(result).print();
    Ok(())
}

pub async fn snippets(
    text: &str,
    ids: Vec<String>,
    mailbox: Option<&str>,
    limit: u32,
) -> Result<()> {
    let mut client = authenticated_client().await?;
    let mut filter = json!({"text": text});
    if let Some(mailbox) = mailbox {
        filter["inMailbox"] = json!(client.find_mailbox(mailbox).await?.id);
    }
    let ids = if ids.is_empty() {
        client
            .query_emails(EmailQuery {
                filter: filter.clone(),
                fetch_summaries: false,
                ..EmailQuery::first(limit)
            })
            .await?
            .ids
    } else {
        ids
    };
    let result = if ids.is_empty() {
        json!({"list": [], "notFound": []})
    } else {
        client.search_snippets(filter, &ids).await?
    };
    Output::success(result).print();
    Ok(())
}

pub async fn star(id: &str, starred: bool) -> Result<()> {
    let client = authenticated_client().await?;
    Output::success(client.star_email(id, starred).await?).print();
    Ok(())
}
