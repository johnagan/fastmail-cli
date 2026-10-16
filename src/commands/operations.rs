use crate::{
    jmap::{DataType, authenticated_client},
    models::Output,
};
use anyhow::{Result, ensure};
use clap::Subcommand;
use serde_json::json;
use std::{collections::HashMap, path::PathBuf};

#[derive(Debug, thiserror::Error)]
#[error("Some records could not be changed; inspect the per-record errors")]
pub struct BulkFailure(pub serde_json::Value);

fn report(result: serde_json::Value) -> Result<()> {
    if ["notCreated", "notUpdated", "notDestroyed"]
        .iter()
        .any(|key| result[*key].as_object().is_some_and(|m| !m.is_empty()))
    {
        return Err(BulkFailure(result).into());
    }
    Output::success(result).print();
    Ok(())
}

#[derive(Subcommand)]
pub enum EmailCommands {
    /// Bulk keyword, flag, and mailbox updates. Mailboxes are IDs.
    Update {
        #[arg(required = true)]
        ids: Vec<String>,
        #[arg(long = "add-keyword")]
        add_keywords: Vec<String>,
        #[arg(long = "remove-keyword")]
        remove_keywords: Vec<String>,
        #[arg(long, action = clap::ArgAction::Set)]
        read: Option<bool>,
        #[arg(long, action = clap::ArgAction::Set)]
        starred: Option<bool>,
        #[arg(long = "add-mailbox", conflicts_with = "move_to")]
        add_mailboxes: Vec<String>,
        #[arg(long = "remove-mailbox", conflicts_with = "move_to")]
        remove_mailboxes: Vec<String>,
        #[arg(long)]
        move_to: Option<String>,
    },
    /// Permanently destroy messages
    Delete {
        #[arg(required = true)]
        ids: Vec<String>,
        #[arg(short = 'y', long)]
        yes: bool,
    },
    EmptyTrash {
        #[arg(short = 'y', long)]
        yes: bool,
    },
    /// Copy creation-ID to email map from another account
    Copy {
        from_account: String,
        file: PathBuf,
        #[arg(long)]
        destroy_original: bool,
    },
    Parse {
        file: PathBuf,
    },
    Export {
        id: String,
        #[arg(short, long)]
        output: PathBuf,
        #[arg(long)]
        overwrite: bool,
    },
}

#[derive(Subcommand)]
pub enum IdentityCommands {
    List,
    Update {
        id: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        text_signature: Option<String>,
        #[arg(long)]
        html_signature: Option<String>,
        #[arg(long)]
        reply_to: Option<String>,
        #[arg(long)]
        bcc: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum SubmissionCommands {
    List {
        #[arg(long, default_value = "50")]
        limit: u32,
    },
    Get {
        #[arg(required = true)]
        ids: Vec<String>,
    },
    Query {
        file: PathBuf,
    },
    Cancel {
        #[arg(required = true)]
        ids: Vec<String>,
    },
}

#[derive(Subcommand)]
pub enum PushCommands {
    List,
    /// Native PushSubscription creation object (deviceClientId, url, types, etc.)
    Create {
        file: PathBuf,
    },
    /// Native PatchObject, e.g. verificationCode/expires/types
    Update {
        id: String,
        file: PathBuf,
    },
    Delete {
        id: String,
    },
}

pub async fn accounts() -> Result<()> {
    let client = authenticated_client().await?;
    let session = client.session()?;
    Output::success(json!({"username": session.username, "accounts": session.accounts, "primaryAccounts": session.primary_accounts})).print();
    Ok(())
}

pub async fn synchronize(kind: DataType, cache: PathBuf, watch: Option<u64>) -> Result<()> {
    ensure!(watch != Some(0), "Watch interval must be positive");
    let client = authenticated_client().await?;
    let interrupt = tokio::signal::ctrl_c();
    tokio::pin!(interrupt);
    loop {
        let result = tokio::select! { result = crate::sync::synchronize(&client,kind,&cache) => result?, _ = &mut interrupt => return Ok(()) };
        Output::success(json!({"cache": cache, "state": result.state, "records": result.records.len(), "accountId": result.account_id})).print();
        match watch {
            Some(seconds) => {
                tokio::select! { _ = tokio::time::sleep(std::time::Duration::from_secs(seconds)) => {}, _ = &mut interrupt => return Ok(()) }
            }
            None => return Ok(()),
        }
    }
}

pub async fn email_operations(command: EmailCommands) -> Result<()> {
    if matches!(
        &command,
        EmailCommands::Delete { yes: false, .. } | EmailCommands::EmptyTrash { yes: false }
    ) {
        anyhow::bail!("Permanent deletion requires -y / --yes");
    }
    let mut client = authenticated_client().await?;
    let result = match command {
        EmailCommands::Update {
            ids,
            add_keywords,
            remove_keywords,
            read,
            starred,
            add_mailboxes,
            remove_mailboxes,
            move_to,
        } => {
            let mut keywords: HashMap<_, _> = add_keywords.into_iter().map(|k| (k, true)).collect();
            keywords.extend(remove_keywords.into_iter().map(|k| (k, false)));
            if let Some(read) = read {
                keywords.insert("$seen".into(), read);
            }
            if let Some(starred) = starred {
                keywords.insert("$flagged".into(), starred);
            }
            client
                .patch_emails(
                    &ids,
                    &keywords,
                    &add_mailboxes,
                    &remove_mailboxes,
                    move_to.as_deref(),
                )
                .await?
        }
        EmailCommands::Delete { ids, .. } => client.destroy_emails(&ids).await?,
        EmailCommands::EmptyTrash { .. } => client.empty_trash().await?,
        EmailCommands::Copy {
            from_account,
            file,
            destroy_original,
        } => {
            client
                .copy_emails(
                    &from_account,
                    super::read_json(&file).await?,
                    destroy_original,
                )
                .await?
        }
        EmailCommands::Parse { file } => client.parse_email(tokio::fs::read(file).await?).await?,
        EmailCommands::Export {
            id,
            output,
            overwrite,
        } => {
            use tokio::io::AsyncWriteExt;
            let bytes = client.export_email(&id).await?;
            let mut options = tokio::fs::OpenOptions::new();
            options.write(true);
            if overwrite {
                options.create(true).truncate(true);
            } else {
                options.create_new(true);
            }
            options.open(&output).await?.write_all(&bytes).await?;
            json!({"id": id, "output": output, "size": bytes.len()})
        }
    };
    report(result)
}

pub async fn identities(command: IdentityCommands) -> Result<()> {
    let client = authenticated_client().await?;
    let result = match command {
        IdentityCommands::List => client.records(DataType::Identity, None).await?,
        IdentityCommands::Update {
            id,
            name,
            text_signature,
            html_signature,
            reply_to,
            bcc,
        } => {
            let mut patch = json!({});
            for (key, value) in [
                ("name", name),
                ("textSignature", text_signature),
                ("htmlSignature", html_signature),
            ] {
                if let Some(value) = value {
                    patch[key] = json!(value);
                }
            }
            if let Some(value) = reply_to {
                patch["replyTo"] = json!(crate::util::parse_addresses(&value));
            }
            if let Some(value) = bcc {
                patch["bcc"] = json!(crate::util::parse_addresses(&value));
            }
            ensure!(
                !patch.as_object().unwrap().is_empty(),
                "Provide at least one identity change"
            );
            client.update_identity(&id, patch).await?
        }
    };
    Output::success(result).print();
    Ok(())
}

pub async fn submissions(command: SubmissionCommands) -> Result<()> {
    let client = authenticated_client().await?;
    let result = match command {
        SubmissionCommands::List { limit } => {
            client
                .query_records(
                    DataType::Submission,
                    json!({"limit": limit, "sort": [{"property": "sentAt", "isAscending": false}]}),
                    false,
                )
                .await?
        }
        SubmissionCommands::Get { ids } => client.records(DataType::Submission, Some(&ids)).await?,
        SubmissionCommands::Query { file } => {
            client
                .query_records(DataType::Submission, super::read_json(&file).await?, false)
                .await?
        }
        SubmissionCommands::Cancel { ids } => client.cancel_submission(&ids).await?,
    };
    report(result)
}

pub async fn data_changes(
    kind: DataType,
    since: Option<String>,
    file: Option<PathBuf>,
) -> Result<()> {
    let client = authenticated_client().await?;
    let result = match (since, file) {
        (Some(since), _) => client.changes(kind, &since, Some(100)).await?,
        (_, Some(file)) => {
            client
                .query_records(kind, super::read_json(&file).await?, true)
                .await?
        }
        _ => client.records(kind, Some(&[])).await?,
    };
    Output::success(result).print();
    Ok(())
}

pub async fn query_records(kind: DataType, file: PathBuf) -> Result<()> {
    let client = authenticated_client().await?;
    Output::success(
        client
            .query_records(kind, super::read_json(&file).await?, false)
            .await?,
    )
    .print();
    Ok(())
}

pub async fn push(command: PushCommands) -> Result<()> {
    let client = authenticated_client().await?;
    let result = match command {
        PushCommands::List => client.push_subscriptions().await?,
        PushCommands::Create { file } => {
            client
                .set_push_subscriptions(json!({"create": {"new": super::read_json(&file).await?}}))
                .await?
        }
        PushCommands::Update { id, file } => {
            client
                .set_push_subscriptions(json!({"update": {id: super::read_json(&file).await?}}))
                .await?
        }
        PushCommands::Delete { id } => {
            client
                .set_push_subscriptions(json!({"destroy": [id]}))
                .await?
        }
    };
    Output::success(result).print();
    Ok(())
}
