use crate::{contacts::ContactBackend, jmap::authenticated_client, models::Output};
use anyhow::{Result, ensure};
use clap::Subcommand;
use serde_json::{Value, json};
use std::path::PathBuf;

#[derive(Subcommand)]
pub enum AddressbookCommands {
    List,
    Get {
        id: String,
    },
    Create {
        name: String,
        #[arg(long)]
        description: Option<String>,
    },
    /// Change any writable AddressBook properties using a JSON PatchObject file
    Update {
        id: String,
        file: PathBuf,
    },
    Default {
        id: String,
    },
    Subscribe {
        id: String,
        #[arg(long, action = clap::ArgAction::Set)]
        subscribed: bool,
    },
    Delete {
        id: String,
        #[arg(long)]
        remove_contents: bool,
        #[arg(short = 'y', long)]
        yes: bool,
    },
    /// A page of changes; repeat with newState while hasMoreChanges is true
    Changes {
        since_state: String,
        #[arg(long)]
        max_changes: Option<u32>,
    },
    /// Get a synchronization state without fetching objects
    State,
}

#[derive(Subcommand)]
pub enum ContactNativeCommands {
    /// Edit common rich-contact fields without constructing JSContact JSON
    Details {
        id: String,
        #[arg(long, conflicts_with = "clear_birthday")]
        birthday: Option<String>,
        #[arg(long)]
        clear_birthday: bool,
        #[arg(long, conflicts_with = "clear_address")]
        address: Option<String>,
        #[arg(long, conflicts_with = "clear_address")]
        country_code: Option<String>,
        #[arg(long, default_value = "home")]
        address_id: String,
        #[arg(long)]
        clear_address: bool,
        #[arg(long)]
        nickname: Option<String>,
    },
    /// Read a full JSContact Card, including groups, media, addresses and anniversaries
    Get {
        id: String,
    },
    #[command(subcommand)]
    Addressbooks(AddressbookCommands),
    /// Create a full JSContact Card from a JSON file
    CardCreate {
        file: PathBuf,
        #[arg(long)]
        address_book: Option<String>,
    },
    /// Apply a native JMAP PatchObject (omitted fields are preserved)
    CardUpdate {
        id: String,
        file: PathBuf,
    },
    /// Native ContactCard/query arguments (filter, sort, position/anchor, limit)
    Query {
        file: PathBuf,
    },
    State,
    Changes {
        since_state: String,
        #[arg(long)]
        max_changes: Option<u32>,
    },
    /// Native queryChanges arguments, including sinceQueryState and original query
    QueryChanges {
        file: PathBuf,
    },
    /// Copy contacts from another account; file is the creation-ID to card map
    Copy {
        from_account: String,
        file: PathBuf,
        #[arg(long)]
        destroy_original: bool,
    },
    Photo {
        id: String,
        file: PathBuf,
        #[arg(long)]
        content_type: Option<String>,
    },
    GroupCreate {
        name: String,
        #[arg(long = "member")]
        members: Vec<String>,
        #[arg(long)]
        address_book: Option<String>,
    },
    /// Replace group membership with contact UIDs (not ContactCard IDs)
    GroupMembers {
        id: String,
        #[arg(long = "member")]
        members: Vec<String>,
    },
}

pub async fn read_json(file: &PathBuf) -> Result<Value> {
    Ok(serde_json::from_slice(&tokio::fs::read(file).await?)?)
}

pub async fn contact_native(command: ContactNativeCommands, backend: ContactBackend) -> Result<()> {
    ensure!(
        !matches!(backend, ContactBackend::Carddav),
        "This operation requires the JMAP contacts backend"
    );
    if matches!(
        &command,
        ContactNativeCommands::Addressbooks(AddressbookCommands::Delete { yes: false, .. })
    ) {
        anyhow::bail!("Deletion requires -y / --yes to confirm");
    }
    let client = authenticated_client().await?;
    let result = match command {
        ContactNativeCommands::Details { id, birthday, clear_birthday, address, country_code, address_id, clear_address, nickname } => {
            let current = client.contacts_get(false, Some(vec![id.clone()])).await?;
            let card = current["list"].as_array().and_then(|l| l.first()).ok_or_else(|| anyhow::anyhow!("Contact not found"))?;
            let mut patch = json!({});
            if birthday.is_some() || clear_birthday {
                let mut anniversaries = card["anniversaries"].as_object().cloned().unwrap_or_default();
                anniversaries.retain(|_, v| v["kind"] != "birth");
                if let Some(date) = birthday { anniversaries.insert("birthday".into(), json!({"kind": "birth", "date": birthday_date(&date)?})); }
                else { anniversaries.retain(|_, v| v["kind"] != "birth"); }
                patch["anniversaries"] = if anniversaries.is_empty() { Value::Null } else { json!(anniversaries) };
            }
            if address.is_some() || country_code.is_some() || clear_address {
                let mut addresses = card["addresses"].as_object().cloned().unwrap_or_default();
                if clear_address { addresses.remove(&address_id); }
                else {
                    let entry = addresses.entry(address_id).or_insert(json!({}));
                    if let Some(address) = address { entry["full"] = json!(address); entry.as_object_mut().ok_or_else(|| anyhow::anyhow!("Invalid address object"))?.remove("components"); }
                    if let Some(code) = country_code { ensure!(code.len() == 2 && code.bytes().all(|c| c.is_ascii_alphabetic()), "Country code must be two letters"); entry["countryCode"] = json!(code.to_uppercase()); }
                }
                patch["addresses"] = if addresses.is_empty() { Value::Null } else { json!(addresses) };
            }
            if let Some(nickname) = nickname {
                let mut names = card["nicknames"].as_object().cloned().unwrap_or_default();
                names.insert("cli".into(), json!({"name": nickname})); patch["nicknames"] = json!(names);
            }
            ensure!(!patch.as_object().unwrap().is_empty(), "Provide at least one contact detail");
            client.contacts_set(false, json!({"update": {id: patch}})).await?
        }
        ContactNativeCommands::Get { id } => client.contacts_get(false, Some(vec![id])).await?,
        ContactNativeCommands::CardCreate { file, address_book } => client.create_contact_card(read_json(&file).await?, address_book.as_deref()).await?,
        ContactNativeCommands::CardUpdate { id, file } => client.contacts_set(false, json!({"update": {id: read_json(&file).await?}})).await?,
        ContactNativeCommands::Query { file } => client.contacts_query(read_json(&file).await?).await?,
        ContactNativeCommands::State => client.contacts_get(false, Some(vec![])).await?,
        ContactNativeCommands::Changes { since_state, max_changes } => client.contacts_changes(false, &since_state, max_changes).await?,
        ContactNativeCommands::QueryChanges { file } => client.contacts_query_changes(read_json(&file).await?).await?,
        ContactNativeCommands::Copy { from_account, file, destroy_original } => client.copy_contacts(&from_account, read_json(&file).await?, destroy_original).await?,
        ContactNativeCommands::Photo { id, file, content_type } => {
            let content_type = content_type.unwrap_or_else(|| crate::util::mime_from_filename(&file.to_string_lossy()));
            client.set_contact_photo(&id, tokio::fs::read(file).await?, &content_type).await?
        }
        ContactNativeCommands::GroupCreate { name, members, address_book } => {
            let members = Value::Object(members.into_iter().map(|uid| (uid, json!(true))).collect());
            client.create_contact_card(json!({"kind": "group", "name": {"full": name}, "members": members}), address_book.as_deref()).await?
        }
        ContactNativeCommands::GroupMembers { id, members } => {
            let members = Value::Object(members.into_iter().map(|uid| (uid, json!(true))).collect());
            client.contacts_set(false, json!({"update": {id: {"members": members}}})).await?
        }
        ContactNativeCommands::Addressbooks(command) => match command {
            AddressbookCommands::List => client.contacts_get(true, None).await?,
            AddressbookCommands::Get { id } => client.contacts_get(true, Some(vec![id])).await?,
            AddressbookCommands::State => client.contacts_get(true, Some(vec![])).await?,
            AddressbookCommands::Create { name, description } => client.contacts_set(true, json!({"create": {"new": {"name": name, "description": description, "isSubscribed": true}}})).await?,
            AddressbookCommands::Update { id, file } => client.contacts_set(true, json!({"update": {id: read_json(&file).await?}})).await?,
            AddressbookCommands::Default { id } => client.contacts_set(true, json!({"onSuccessSetIsDefault": id})).await?,
            AddressbookCommands::Subscribe { id, subscribed } => client.contacts_set(true, json!({"update": {id: {"isSubscribed": subscribed}}})).await?,
            AddressbookCommands::Delete { id, remove_contents, .. } => client.contacts_set(true, json!({"destroy": [id], "onDestroyRemoveContents": remove_contents})).await?,
            AddressbookCommands::Changes { since_state, max_changes } => client.contacts_changes(true, &since_state, max_changes).await?,
        },
    };
    Output::success(result).print();
    Ok(())
}

fn birthday_date(value: &str) -> Result<Value> {
    let parts: Vec<_> = value.split('-').collect();
    ensure!(
        parts.len() == 3 && parts[0].len() == 4 && parts[1].len() == 2 && parts[2].len() == 2,
        "Birthday must be YYYY-MM-DD"
    );
    let year: u32 = parts[0].parse()?;
    let month: u32 = parts[1].parse()?;
    let day: u32 = parts[2].parse()?;
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let days = match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        _ => 0,
    };
    ensure!(year > 0 && day > 0 && day <= days, "Invalid birthday date");
    Ok(json!({"@type": "PartialDate", "year": year, "month": month, "day": day}))
}
