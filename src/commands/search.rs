use crate::jmap::authenticated_client;
use crate::models::Output;
use anyhow::ensure;
use serde_json::{Value, json};

#[derive(clap::Args, Default)]
pub struct SearchOptions {
    /// Native JMAP FilterCondition/FilterOperator JSON; mailbox references are IDs
    #[arg(long, conflicts_with = "filter_file")]
    pub filter: Option<String>,
    #[arg(long)]
    pub filter_file: Option<std::path::PathBuf>,
    /// Native JMAP sort comparator array
    #[arg(long)]
    pub sort: Option<String>,
    #[arg(long, conflicts_with = "position")]
    pub anchor: Option<String>,
    #[arg(long, default_value = "0", requires = "anchor")]
    pub anchor_offset: i64,
    #[arg(long, allow_hyphen_values = true)]
    pub position: Option<i64>,
    #[arg(long)]
    pub total: bool,
    #[arg(long)]
    pub collapse_threads: bool,
}

pub async fn search_with_options(
    filter: SearchFilter,
    limit: u32,
    options: SearchOptions,
) -> anyhow::Result<()> {
    if options.filter.is_none()
        && options.filter_file.is_none()
        && options.sort.is_none()
        && options.anchor.is_none()
        && options.position.is_none()
        && !options.total
        && !options.collapse_threads
    {
        return search(filter, limit).await;
    }
    let mut client = authenticated_client().await?;
    let mailbox = match &filter.mailbox {
        Some(name) => Some(client.find_mailbox(name).await?.id),
        None => None,
    };
    let flat = crate::jmap::search_filter_to_jmap(&filter, mailbox.as_deref());
    let native: Option<Value> = match (options.filter, options.filter_file) {
        (Some(text), _) => Some(serde_json::from_str(&text)?),
        (_, Some(file)) => Some(serde_json::from_slice(&tokio::fs::read(file).await?)?),
        _ => None,
    };
    let filter = match native {
        Some(native) => {
            ensure!(native.is_object(), "Filter must be a JSON object");
            json!({"operator": "AND", "conditions": [flat, native]})
        }
        None => flat,
    };
    let sort = match options.sort {
        Some(sort) => {
            let sort: Value = serde_json::from_str(&sort)?;
            sort.as_array()
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("Sort must be an array"))?
        }
        None => vec![json!({"property": "receivedAt", "isAscending": false})],
    };
    let start = match options.anchor {
        Some(id) => crate::jmap::QueryStart::Anchor {
            id,
            offset: options.anchor_offset,
        },
        None => crate::jmap::QueryStart::Position(options.position.unwrap_or(0)),
    };
    let page = client
        .query_emails(crate::jmap::EmailQuery {
            filter,
            sort,
            start,
            limit,
            calculate_total: options.total,
            collapse_threads: options.collapse_threads,
            fetch_summaries: true,
        })
        .await?;
    Output::success(json!({"emails": page.emails, "ids": page.ids, "position": page.position, "total": page.total, "queryState": page.query_state})).print();
    Ok(())
}

/// Search filter matching JMAP Email/query FilterCondition
#[derive(Debug, Default)]
pub struct SearchFilter {
    pub text: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub cc: Option<String>,
    pub bcc: Option<String>,
    pub subject: Option<String>,
    pub body: Option<String>,
    pub mailbox: Option<String>,
    pub has_attachment: bool,
    pub min_size: Option<u32>,
    pub max_size: Option<u32>,
    pub before: Option<String>,
    pub after: Option<String>,
    pub unread: bool,
    pub flagged: bool,
}

pub async fn search(filter: SearchFilter, limit: u32) -> anyhow::Result<()> {
    let mut client = authenticated_client().await?;

    // Resolve mailbox name to ID if specified
    let mailbox_id = if let Some(ref mailbox_name) = filter.mailbox {
        Some(client.find_mailbox(mailbox_name).await?.id)
    } else {
        None
    };

    let emails = client
        .search_emails_filtered(&filter, mailbox_id.as_deref(), limit)
        .await?;
    Output::success(emails).print();

    Ok(())
}
