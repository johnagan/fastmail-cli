use super::*;
use crate::carddav::{Contact, ContactEmail, ContactFields, ContactPhone};

const CONTACTS: &str = "urn:ietf:params:jmap:contacts";

fn values(value: &Value) -> impl Iterator<Item = &Value> {
    value.as_object().into_iter().flat_map(|map| map.values())
}

fn contact(card: &Value) -> Contact {
    let name = card["name"]["full"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| {
            card["name"]["components"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|c| c["value"].as_str())
                .collect::<Vec<_>>()
                .join(" ")
        });
    Contact {
        id: card["id"].as_str().unwrap_or_default().into(),
        name,
        emails: values(&card["emails"])
            .filter_map(|e| {
                Some(ContactEmail {
                    email: e["address"].as_str()?.into(),
                    label: e["label"].as_str().map(str::to_owned),
                })
            })
            .collect(),
        phones: values(&card["phones"])
            .filter_map(|p| {
                Some(ContactPhone {
                    number: p["number"].as_str()?.into(),
                    label: p["label"].as_str().map(str::to_owned),
                })
            })
            .collect(),
        organization: values(&card["organizations"])
            .find_map(|v| v["name"].as_str().map(str::to_owned)),
        title: values(&card["titles"]).find_map(|v| v["name"].as_str().map(str::to_owned)),
        notes: values(&card["notes"]).find_map(|v| v["note"].as_str().map(str::to_owned)),
    }
}

fn fields_json(fields: &ContactFields<'_>) -> Value {
    let mut value = json!({});
    if let Some(name) = fields.name {
        value["name"] = json!({"full": name});
    }
    if let Some(emails) = fields.emails {
        value["emails"] = Value::Object(
            emails
                .iter()
                .enumerate()
                .map(|(i, e)| {
                    let mut v = json!({"address": e.email});
                    if let Some(label) = &e.label {
                        v["label"] = json!(label);
                    }
                    (format!("e{i}"), v)
                })
                .collect(),
        );
    }
    if let Some(phones) = fields.phones {
        value["phones"] = Value::Object(
            phones
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    let mut v = json!({"number": p.number});
                    if let Some(label) = &p.label {
                        v["label"] = json!(label);
                    }
                    (format!("p{i}"), v)
                })
                .collect(),
        );
    }
    if let Some(org) = fields.organization {
        value["organizations"] = json!({"org": {"name": org}});
    }
    if let Some(title) = fields.title {
        value["titles"] = json!({"title": {"name": title, "kind": "title"}});
    }
    if let Some(notes) = fields.notes {
        value["notes"] = json!({"note": {"note": notes}});
    }
    value
}

impl JmapClient {
    /// Native RFC 9610 operations, retaining all JSContact properties.
    pub async fn contacts_get(
        &self,
        addressbooks: bool,
        ids: Option<Vec<String>>,
    ) -> Result<Value> {
        let kind = if addressbooks {
            "AddressBook"
        } else {
            "ContactCard"
        };
        self.call(
            &format!("{kind}/get"),
            json!({"accountId": self.capability_account(CONTACTS)?, "ids": ids}),
        )
        .await
    }

    pub async fn contacts_set(&self, addressbooks: bool, mut args: Value) -> Result<Value> {
        if !args.is_object() {
            return Err(Error::Config("Expected a JSON object".into()));
        }
        args["accountId"] = json!(self.capability_account(CONTACTS)?);
        self.call(
            if addressbooks {
                "AddressBook/set"
            } else {
                "ContactCard/set"
            },
            args,
        )
        .await
    }

    pub async fn contacts_query(&self, mut args: Value) -> Result<Value> {
        if !args.is_object() {
            return Err(Error::Config("Expected a JSON object".into()));
        }
        args["accountId"] = json!(self.capability_account(CONTACTS)?);
        self.call("ContactCard/query", args).await
    }

    pub async fn contacts_changes(
        &self,
        addressbooks: bool,
        since_state: &str,
        max_changes: Option<u32>,
    ) -> Result<Value> {
        let mut args =
            json!({"accountId": self.capability_account(CONTACTS)?, "sinceState": since_state});
        if let Some(max) = max_changes {
            args["maxChanges"] = json!(max);
        }
        self.call(
            if addressbooks {
                "AddressBook/changes"
            } else {
                "ContactCard/changes"
            },
            args,
        )
        .await
    }

    pub async fn contacts_query_changes(&self, mut args: Value) -> Result<Value> {
        if !args.is_object() {
            return Err(Error::Config("Expected a JSON object".into()));
        }
        args["accountId"] = json!(self.capability_account(CONTACTS)?);
        self.call("ContactCard/queryChanges", args).await
    }

    pub async fn copy_contacts(
        &self,
        from_account: &str,
        cards: Value,
        destroy_original: bool,
    ) -> Result<Value> {
        self.call("ContactCard/copy", json!({"accountId": self.capability_account(CONTACTS)?, "fromAccountId": from_account, "create": cards, "onSuccessDestroyOriginal": destroy_original})).await
    }

    pub async fn create_contact_card(&self, mut card: Value, book: Option<&str>) -> Result<Value> {
        if !card.is_object() {
            return Err(Error::Config("Expected a JSContact Card object".into()));
        }
        card["@type"] = json!("Card");
        card["version"] = json!("1.0");
        if card["uid"].is_null() {
            card["uid"] = json!(uuid::Uuid::new_v4().to_string());
        }
        if let Some(book) = book {
            card["addressBookIds"] = json!({book: true});
        }
        if card["addressBookIds"].is_null() {
            let books = self.contact_addressbooks().await?;
            let books = books
                .as_array()
                .ok_or_else(|| Error::Server("Missing address books".into()))?;
            let book = books
                .iter()
                .find(|b| b["isDefault"] == true && b["myRights"]["mayWrite"] == true)
                .or_else(|| books.iter().find(|b| b["myRights"]["mayWrite"] == true))
                .and_then(|b| b["id"].as_str())
                .ok_or_else(|| Error::Config("No writable contact address book".into()))?;
            card["addressBookIds"] = json!({book: true});
        }
        self.contacts_set(false, json!({"create": {"new": card}}))
            .await
    }

    pub async fn set_contact_photo(
        &self,
        id: &str,
        data: Vec<u8>,
        content_type: &str,
    ) -> Result<Value> {
        if !content_type.starts_with("image/") {
            return Err(Error::Config("Contact photo must be an image".into()));
        }
        let blob = self
            .upload_blob_to(self.capability_account(CONTACTS)?, data, content_type)
            .await?;
        let current = self.contacts_get(false, Some(vec![id.into()])).await?;
        let card = current["list"]
            .as_array()
            .and_then(|l| l.first())
            .ok_or_else(|| Error::Config(format!("Contact not found: {id}")))?;
        let mut media = card["media"].as_object().cloned().unwrap_or_default();
        media.retain(|_, value| value["kind"] != "photo");
        media.insert(
            uuid::Uuid::new_v4().to_string(),
            json!({"kind": "photo", "blobId": blob, "mediaType": content_type}),
        );
        self.contacts_set(false, json!({"update": {id: {"media": media}}}))
            .await
    }
    pub async fn contact_addressbooks(&self) -> Result<Value> {
        let account = self.capability_account(CONTACTS)?;
        let result = self
            .call("AddressBook/get", json!({"accountId": account}))
            .await?;
        Ok(result["list"].clone())
    }

    pub async fn get_contact(&self, id: &str) -> Result<Contact> {
        let result = self
            .call(
                "ContactCard/get",
                json!({"accountId": self.capability_account(CONTACTS)?, "ids": [id]}),
            )
            .await?;
        result["list"]
            .as_array()
            .and_then(|l| l.first())
            .map(contact)
            .ok_or_else(|| Error::Config(format!("Contact not found: {id}")))
    }

    /// Query in pages so list/search never silently truncate at the server's limit.
    pub async fn query_contacts(&self, text: Option<&str>) -> Result<Vec<Contact>> {
        let account = self.capability_account(CONTACTS)?;
        let mut position = 0;
        let mut contacts = Vec::new();
        loop {
            let filter = text
                .filter(|s| !s.is_empty())
                .map(|s| json!({"text": s}))
                .unwrap_or(json!({}));
            let result = self.call("ContactCard/query", json!({"accountId": account, "filter": filter, "position": position, "limit": 100})).await?;
            let ids = result["ids"]
                .as_array()
                .ok_or_else(|| Error::Server("Missing contact query ids".into()))?;
            if ids.is_empty() {
                break;
            }
            let cards = self
                .call("ContactCard/get", json!({"accountId": account, "ids": ids}))
                .await?;
            contacts.extend(cards["list"].as_array().into_iter().flatten().map(contact));
            position += ids.len();
        }
        Ok(contacts)
    }

    pub async fn create_jmap_contact(&self, fields: &ContactFields<'_>) -> Result<Contact> {
        self.create_jmap_contact_in(fields, None).await
    }

    pub async fn create_jmap_contact_in(
        &self,
        fields: &ContactFields<'_>,
        book: Option<&str>,
    ) -> Result<Contact> {
        let result = self.create_contact_card(fields_json(fields), book).await?;
        let id = result["created"]["new"]["id"]
            .as_str()
            .ok_or_else(|| Error::Server("No contact ID returned".into()))?;
        self.get_contact(id).await
    }

    pub async fn update_jmap_contact(
        &self,
        id: &str,
        fields: &ContactFields<'_>,
    ) -> Result<Contact> {
        let mut patch = fields_json(fields);
        // Change the display name without discarding structured name components.
        if let Some(name) = fields.name {
            patch.as_object_mut().unwrap().remove("name");
            // Fetch to distinguish an absent optional name from an existing object.
            let current = self
                .call(
                    "ContactCard/get",
                    json!({"accountId": self.capability_account(CONTACTS)?, "ids": [id]}),
                )
                .await?;
            if current["list"][0]["name"].is_object() {
                patch["name/full"] = json!(name);
            } else {
                patch["name"] = json!({"full": name});
            }
        }
        self.call(
            "ContactCard/set",
            json!({"accountId": self.capability_account(CONTACTS)?, "update": {id: patch}}),
        )
        .await?;
        self.get_contact(id).await
    }

    pub async fn delete_jmap_contact(&self, id: &str) -> Result<()> {
        self.call(
            "ContactCard/set",
            json!({"accountId": self.capability_account(CONTACTS)?, "destroy": [id]}),
        )
        .await?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "contacts_tests.rs"]
mod tests;
