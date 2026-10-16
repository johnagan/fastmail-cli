//! Contacts backend selection shared by the CLI and GraphQL.
use crate::error::{Error, Result};
use crate::{
    carddav::{CardDavClient, Contact, ContactFields},
    config::Config,
    jmap::{JmapClient, SharedJmapClient},
};
use clap::ValueEnum;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
pub enum ContactBackend {
    /// Prefer JMAP; use CardDAV if contacts are not advertised or no API token is configured
    #[default]
    Auto,
    Jmap,
    Carddav,
}

pub enum ContactsClient {
    Jmap(SharedJmapClient),
    Carddav(CardDavClient),
}

impl ContactsClient {
    pub async fn local(backend: ContactBackend) -> Result<Self> {
        let config = Config::load()?;
        if !matches!(backend, ContactBackend::Carddav) {
            if config.get_token().is_ok() {
                let mut client = JmapClient::from_local_config().await?;
                client.authenticate().await?;
                if client.supports("urn:ietf:params:jmap:contacts") {
                    return Ok(Self::Jmap(Arc::new(Mutex::new(client))));
                }
                if matches!(backend, ContactBackend::Jmap) {
                    return Err(Error::Config(
                        "Server/token does not advertise JMAP contacts".into(),
                    ));
                }
            } else if matches!(backend, ContactBackend::Jmap) {
                return Err(Error::NotAuthenticated);
            }
        }
        Ok(Self::Carddav(CardDavClient::new(
            config.get_username()?,
            config.get_app_password()?,
        )))
    }

    /// Request-scoped credentials prevent GraphQL mutations using a different user's CardDAV account.
    pub async fn for_request(
        client: SharedJmapClient,
        username: Option<String>,
        password: Option<String>,
    ) -> Result<Self> {
        if client
            .lock()
            .await
            .supports("urn:ietf:params:jmap:contacts")
        {
            return Ok(Self::Jmap(client));
        }
        let username = username.ok_or_else(|| {
            Error::Config("Username not configured. Set FASTMAIL_USERNAME env var.".into())
        })?;
        let password = password.ok_or_else(|| Error::Config("App password not configured. Set FASTMAIL_APP_PASSWORD env var (API tokens don't work for CardDAV).".into()))?;
        Ok(Self::Carddav(CardDavClient::new(username, password)))
    }

    pub async fn search(&self, query: Option<&str>) -> Result<Vec<Contact>> {
        match self {
            Self::Jmap(client) => client.lock().await.query_contacts(query).await,
            Self::Carddav(client) => {
                if let Some(query) = query.filter(|q| !q.is_empty()) {
                    return client.search_contacts(query).await;
                }
                let mut contacts = Vec::new();
                for book in client.list_addressbooks().await? {
                    contacts.extend(client.list_contacts(&book.href).await?);
                }
                Ok(contacts)
            }
        }
    }

    pub async fn create(&self, fields: &ContactFields<'_>) -> Result<Contact> {
        self.create_in(fields, None).await
    }

    pub async fn create_in(
        &self,
        fields: &ContactFields<'_>,
        book: Option<&str>,
    ) -> Result<Contact> {
        match self {
            Self::Jmap(client) => {
                client
                    .lock()
                    .await
                    .create_jmap_contact_in(fields, book)
                    .await
            }
            Self::Carddav(client) => {
                if book.is_some() {
                    return Err(Error::Config(
                        "--address-book requires JMAP contacts".into(),
                    ));
                }
                client.create_contact(fields).await
            }
        }
    }

    pub async fn update(&self, id: &str, fields: &ContactFields<'_>) -> Result<Contact> {
        match self {
            Self::Jmap(client) => client.lock().await.update_jmap_contact(id, fields).await,
            Self::Carddav(client) => client.update_contact(id, fields).await,
        }
    }

    pub async fn delete(&self, id: &str) -> Result<()> {
        match self {
            Self::Jmap(client) => client.lock().await.delete_jmap_contact(id).await,
            Self::Carddav(client) => client.delete_contact(id).await,
        }
    }
}
