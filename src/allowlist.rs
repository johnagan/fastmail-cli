use anyhow::{bail, Result};

/// Checks recipients against an optional allowlist. An empty list allows everyone.
/// Entries are full addresses, or a domain written as `@example.com`.
pub fn check(allowed: &[String], recipients: &[String]) -> Result<()> {
    if allowed.is_empty() {
        return Ok(());
    }
    let refused: Vec<&String> = recipients
        .iter()
        .filter(|r| !allowed.iter().any(|a| matches(a, r)))
        .collect();
    if refused.is_empty() {
        return Ok(());
    }
    let list: Vec<&str> = refused.iter().map(|s| s.as_str()).collect();
    bail!(
        "recipients not on the allowed list: {}. Add them to allowed_recipients (or FASTMAIL_ALLOWED_RECIPIENTS) to send",
        list.join(", ")
    )
}

fn matches(entry: &str, address: &str) -> bool {
    let entry = entry.trim().to_ascii_lowercase();
    let address = address.trim().to_ascii_lowercase();
    match entry.strip_prefix('@') {
        Some(domain) => address.rsplit_once('@').is_some_and(|(_, d)| d == domain),
        None => address == entry,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn empty_list_allows_everyone() {
        assert!(check(&[], &s(&["anyone@example.com"])).is_ok());
    }

    #[test]
    fn exact_addresses_and_domains() {
        let allowed = s(&["me@example.com", "@team.example"]);
        assert!(check(&allowed, &s(&["ME@example.com", "bob@team.example"])).is_ok());
        assert!(check(&allowed, &s(&["stranger@example.com"])).is_err());
        assert!(check(&allowed, &s(&["bob@team.example.org"])).is_err());
    }

    #[test]
    fn names_every_refused_address() {
        let err = check(&s(&["ok@example.com"]), &s(&["a@x.com", "ok@example.com", "b@y.com"])).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("a@x.com") && msg.contains("b@y.com") && !msg.contains("ok@example.com"));
    }
}
