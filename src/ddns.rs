use crate::http::{encode_segment, send};
use anyhow::{bail, Context, Result};
use reqwest::Client;
use serde_json::{json, Value};

/// Fastmail's DynDNS2 endpoint. Basic auth, with the account's login as the
/// username and a Dynamic DNS API token as the password.
const ENDPOINT: &str = "https://api.fastmail.com/dynamicdns/update";

/// One update. An omitted address leaves that record alone, unless both are
/// omitted: then Fastmail sets whichever family the request itself arrived on.
pub struct Update<'a> {
    pub hostname: &'a str,
    pub ipv4: Option<&'a str>,
    pub ipv6: Option<&'a str>,
}

pub async fn update(http: &Client, username: &str, token: &str, update: Update<'_>) -> Result<Value> {
    let mut url = format!("{ENDPOINT}?hostname={}", encode_segment(update.hostname));
    if let Some(ip) = update.ipv4 {
        url.push_str(&format!("&myip={}", encode_segment(ip)));
    }
    if let Some(ip) = update.ipv6 {
        url.push_str(&format!("&myipv6={}", encode_segment(ip)));
    }

    let req = http.get(url).basic_auth(username, Some(token));
    let resp = send(req).await?;
    let status = resp.status();
    let body = resp.text().await.context("reading the Dynamic DNS response")?;
    let (code, address) = split_body(&body);

    match code {
        "good" | "nochg" => Ok(json!({
            "hostname": update.hostname,
            "result": code,
            "changed": code == "good",
            "address": address,
        })),
        "badauth" => bail!(
            "Dynamic DNS rejected the credentials. The username is your Fastmail login and the token must be a Dynamic DNS API token, not a JMAP token"
        ),
        "nohost" => bail!(
            "no Dynamic DNS entry for {}. Add one under Settings → Domains → DNS, with the type set to Dynamic DNS",
            update.hostname
        ),
        "notfqdn" => bail!("{} is not a fully qualified domain name, for example home.example.com", update.hostname),
        "badip" => bail!(
            "Fastmail rejected the address. Pass IPv4 with --ip and IPv6 with --ipv6; leave both out to use this machine's address"
        ),
        "dnserr" => bail!("Fastmail had a DNS error updating {}; try again later", update.hostname),
        "abuse" => bail!("Fastmail blocked the update as abusive; wait before trying again"),
        "911" => bail!("Fastmail reported a server error; try again later"),
        "" => bail!("Dynamic DNS returned HTTP {status} with an empty body"),
        other => bail!("Dynamic DNS returned {other:?} (HTTP {status})"),
    }
}

/// A DynDNS2 reply is a status word, optionally followed by the address it set.
fn split_body(body: &str) -> (&str, Option<&str>) {
    let mut words = body.split_whitespace();
    (words.next().unwrap_or_default(), words.next())
}

#[cfg(test)]
mod tests {
    use super::split_body;

    #[test]
    fn reads_status_and_address() {
        assert_eq!(split_body("good 203.0.113.5"), ("good", Some("203.0.113.5")));
        assert_eq!(split_body("nochg 2001:db8::1\n"), ("nochg", Some("2001:db8::1")));
    }

    #[test]
    fn reads_status_alone() {
        assert_eq!(split_body("badauth"), ("badauth", None));
        assert_eq!(split_body(""), ("", None));
    }
}
