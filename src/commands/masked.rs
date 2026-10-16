use crate::jmap::authenticated_client;
use crate::models::Output;

pub async fn list_masked_emails() -> anyhow::Result<()> {
    let client = authenticated_client().await?;

    let masked_emails = client.list_masked_emails().await?;

    Output::success(masked_emails).print();
    Ok(())
}

pub async fn create_masked_email(
    for_domain: Option<&str>,
    description: Option<&str>,
    prefix: Option<&str>,
    url: Option<&str>,
    pending: bool,
) -> anyhow::Result<()> {
    let client = authenticated_client().await?;

    let mut properties = serde_json::json!({"state": if pending { "pending" } else { "enabled" }});
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
    let masked_email = client.create_masked_email_options(properties).await?;

    Output::success(masked_email).print();
    Ok(())
}

pub async fn edit_masked_email(
    id: &str,
    domain: Option<&str>,
    description: Option<&str>,
    url: Option<&str>,
    clear_url: bool,
) -> anyhow::Result<()> {
    let mut properties = serde_json::json!({});
    for (key, value) in [
        ("forDomain", domain),
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
    anyhow::ensure!(
        !properties.as_object().unwrap().is_empty(),
        "Provide at least one metadata change"
    );
    let client = authenticated_client().await?;
    Output::success(client.update_masked_email_options(id, properties).await?).print();
    Ok(())
}

pub async fn enable_masked_email(id: &str) -> anyhow::Result<()> {
    let client = authenticated_client().await?;

    client
        .update_masked_email(id, Some("enabled"), None, None)
        .await?;

    Output::<()>::success_msg(format!("Masked email {} enabled", id)).print();
    Ok(())
}

pub async fn disable_masked_email(id: &str) -> anyhow::Result<()> {
    let client = authenticated_client().await?;

    client
        .update_masked_email(id, Some("disabled"), None, None)
        .await?;

    Output::<()>::success_msg(format!("Masked email {} disabled", id)).print();
    Ok(())
}

pub async fn delete_masked_email(id: &str) -> anyhow::Result<()> {
    let client = authenticated_client().await?;

    client
        .update_masked_email(id, Some("deleted"), None, None)
        .await?;

    Output::<()>::success_msg(format!("Masked email {} deleted", id)).print();
    Ok(())
}
