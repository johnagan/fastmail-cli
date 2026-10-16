use crate::carddav::{ContactEmail, ContactFields, ContactPhone};
use crate::contacts::{ContactBackend, ContactsClient};
use crate::models::Output;

/// Parse comma-separated emails into ContactEmail vec
fn parse_emails(input: Option<&str>) -> Vec<ContactEmail> {
    input
        .map(|e| {
            e.split(',')
                .filter(|addr| !addr.trim().is_empty())
                .map(|addr| ContactEmail {
                    email: addr.trim().to_string(),
                    label: None,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Parse comma-separated phones into ContactPhone vec
fn parse_phones(input: Option<&str>) -> Vec<ContactPhone> {
    input
        .map(|p| {
            p.split(',')
                .filter(|num| !num.trim().is_empty())
                .map(|num| ContactPhone {
                    number: num.trim().to_string(),
                    label: None,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// List all contacts from all address books
pub async fn list_contacts(backend: ContactBackend) -> anyhow::Result<()> {
    let client = ContactsClient::local(backend).await?;
    Output::success(client.search(None).await?).print();
    Ok(())
}

/// Search contacts by name or email
pub async fn search_contacts(query: &str, backend: ContactBackend) -> anyhow::Result<()> {
    let client = ContactsClient::local(backend).await?;
    let contacts = client.search(Some(query)).await?;

    Output::success(contacts).print();
    Ok(())
}

/// Create a new contact
#[allow(clippy::too_many_arguments)]
pub async fn create_contact(
    backend: ContactBackend,
    name: &str,
    email: Option<&str>,
    phone: Option<&str>,
    organization: Option<&str>,
    title: Option<&str>,
    notes: Option<&str>,
    address_book: Option<&str>,
) -> anyhow::Result<()> {
    let client = ContactsClient::local(backend).await?;
    let emails = parse_emails(email);
    let phones = parse_phones(phone);

    let contact = client
        .create_in(
            &ContactFields {
                name: Some(name),
                emails: Some(&emails),
                phones: Some(&phones),
                organization,
                title,
                notes,
            },
            address_book,
        )
        .await?;

    Output::success(contact).print();
    Ok(())
}

/// Update an existing contact
#[allow(clippy::too_many_arguments)]
pub async fn update_contact(
    backend: ContactBackend,
    contact_id: &str,
    name: Option<&str>,
    email: Option<&str>,
    phone: Option<&str>,
    organization: Option<&str>,
    title: Option<&str>,
    notes: Option<&str>,
) -> anyhow::Result<()> {
    let client = ContactsClient::local(backend).await?;
    let emails = parse_emails(email);
    let phones = parse_phones(phone);

    let emails_ref = email.map(|_| emails.as_slice());
    let phones_ref = phone.map(|_| phones.as_slice());

    let contact = client
        .update(
            contact_id,
            &ContactFields {
                name,
                emails: emails_ref,
                phones: phones_ref,
                organization,
                title,
                notes,
            },
        )
        .await?;

    Output::success(contact).print();
    Ok(())
}

/// Delete a contact
pub async fn delete_contact(contact_id: &str, backend: ContactBackend) -> anyhow::Result<()> {
    let client = ContactsClient::local(backend).await?;
    client.delete(contact_id).await?;
    Output::<()>::success_msg(format!("Contact {contact_id} deleted.")).print();
    Ok(())
}
