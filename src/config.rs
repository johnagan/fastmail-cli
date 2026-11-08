use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::{fmt, fs, path::PathBuf};

#[derive(Deserialize, Default)]
struct FileConfig {
    username: Option<String>,
    #[serde(default)]
    jmap: Secret,
    #[serde(default)]
    dav: Secret,
    #[serde(default)]
    imap: Secret,
}

#[derive(Deserialize, Default)]
struct Secret {
    token: Option<String>,
    password: Option<String>,
}

pub struct Credentials {
    pub username: String,
    pub jmap_token: Option<String>,
    pub dav_password: Option<String>,
    pub imap_password: Option<String>,
}

fn state(value: &Option<String>) -> &'static str {
    if value.is_some() { "<set>" } else { "<unset>" }
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credentials")
            .field("username", &self.username)
            .field("jmap_token", &state(&self.jmap_token))
            .field("dav_password", &state(&self.dav_password))
            .field("imap_password", &state(&self.imap_password))
            .finish()
    }
}

fn config_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("FASTMAIL_CONFIG") {
        return Some(PathBuf::from(p));
    }
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".config/fastmail/config.toml"))
}

fn check_private(path: &PathBuf) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = fs::metadata(path)?.permissions().mode() & 0o777;
    if mode & 0o077 != 0 {
        bail!(
            "{} is readable by other users (mode {:o}); run: chmod 600 {}",
            path.display(),
            mode,
            path.display()
        );
    }
    Ok(())
}

/// Environment variables win over the config file. Either source is optional
/// on its own, so CI can use env vars alone and a personal machine can use the file.
pub fn load() -> Result<Credentials> {
    let file = match config_path() {
        Some(path) if path.exists() => {
            check_private(&path)?;
            let text = fs::read_to_string(&path)
                .with_context(|| format!("reading {}", path.display()))?;
            toml::from_str::<FileConfig>(&text)
                .with_context(|| format!("parsing {}", path.display()))?
        }
        _ => FileConfig::default(),
    };

    let env = |key: &str| std::env::var(key).ok().filter(|v| !v.is_empty());
    let username = env("FASTMAIL_USERNAME")
        .or(file.username)
        .context("no username: set FASTMAIL_USERNAME or username in the config file")?;

    Ok(Credentials {
        username,
        jmap_token: env("FASTMAIL_JMAP_TOKEN").or(file.jmap.token),
        dav_password: env("FASTMAIL_DAV_PASSWORD").or(file.dav.password),
        imap_password: env("FASTMAIL_IMAP_PASSWORD").or(file.imap.password),
    })
}
