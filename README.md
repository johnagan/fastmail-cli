# Fastmail CLI for AI Agents

Command-line access to Fastmail: mail, memos, contacts, calendars, files, notes, vacation replies, masked email, dynamic DNS, and a live stream of new mail.

JMAP is the primary path. CalDAV, CardDAV, WebDAV, and IMAP cover what JMAP doesn't.

## Features

| Area | Commands | Protocol | Credential |
|---|---|---|---|
| Mail: search, read, attachments | `mail search`, `mail get`, `mail attachments` | JMAP | JMAP token (Email) |
| Mail: flag, read, move, trash | `mail read`, `mail unread`, `mail flag`, `mail unflag`, `mail move`, `mail trash` | JMAP | JMAP token (Email) |
| Drafts | `mail draft`, with `--attach` for files | JMAP | JMAP token (Email) |
| Replies | `mail reply ID [--all]`, keeps the thread | JMAP | JMAP token (Email) |
| Send | `mail send`, with `--attach`; `mail reply --send` | JMAP | JMAP token (Email, Email submission) |
| Memos | `memos list` | JMAP | JMAP token (Email) |
| Vacation replies | `vacation show`, `vacation set` (with `--on`/`--off`, `--from`, `--to`, `--subject`, `--file`, `--clear`) | JMAP | JMAP token (Vacation response) |
| Masked email | `masked list`, `masked create`, `masked enable`, `masked disable`, `masked delete --yes` | JMAP | JMAP token (Masked Email) |
| Dynamic DNS | `dynamic-dns update HOSTNAME`, also `ddns`. Add `--ip` or `--ipv6` to set an address yourself | DynDNS2 | Dynamic DNS API token |
| New mail stream | `watch` | JMAP push | JMAP token (Email) |
| Contacts | `contacts list`, `contacts create`, `contacts update`, `contacts delete --yes` | CardDAV | DAV app password |
| Calendars | `calendars list`, `calendars events`, `calendars create`, `calendars update`, `calendars delete --yes`. `create` and `update` take `--dry-run` to show the change without writing it | CalDAV | DAV app password |
| Files | `files ls`, `files get`, `files put`, `files mv`, `files rm --yes` | WebDAV | DAV app password |
| Notes | `notes list`, `notes get`, `notes create`, `notes put`, `notes delete --yes` | IMAP | IMAP app password |
| Health check | `doctor` | all | all configured |
| Shell completions | `completions bash\|zsh\|fish` | none | none |

Every delete needs `--yes`. Anything that changes an existing note, contact, or event shows the change first and writes only when you pass `--apply` (notes) or name the field to change (contacts, calendars).

## Install

You need Rust 1.85 or newer.

### Default install (all features)

This installs everything: mail, contacts, calendars, files, notes, and completions. Notes need OpenSSL headers on Linux, so install those first (see [Requirements](#requirements)).

```sh
cargo install fastmail
```

The binary goes to `~/.cargo/bin/fastmail`. Make sure that directory is on your PATH.

### Lean installs

To leave features out, use the flags below. A lean build drops the features it doesn't include, and any system libraries they need. Lean builds don't include notes.

```sh
# Mail, memos, vacation, masked email, and watch only. No system libraries needed.
cargo install fastmail --no-default-features

# Adds contacts, calendars, and files. No system libraries needed.
cargo install fastmail --no-default-features --features dav
```

To install straight from git instead of crates.io, add `--git https://github.com/johnagan/fastmail-cli` to any of these.

### From a clone

```sh
git clone https://github.com/johnagan/fastmail-cli
cd fastmail-cli
cargo install --path .                                           # all features
cargo install --path . --no-default-features --features dav      # lean
```

### Upgrading

Run the same install command again with `--force`. Check which features the binary has with `fastmail doctor`. Its `features` field lists what was compiled in.

## Requirements

- Rust stable, 1.85 or newer.
- On Linux, building with notes support needs OpenSSL headers (see below). Other platforms need nothing extra.

Install the Linux packages:

```sh
sudo apt-get install -y build-essential pkg-config libssl-dev   # Debian/Ubuntu
sudo dnf install -y gcc pkgconf-pkg-config openssl-devel        # Fedora/RHEL
```

macOS needs only the Xcode command-line tools.

## Build options

Everything is on by default. See [Install](#install) for the install commands.

To build from a clone without installing:

```sh
cargo build --no-default-features                 # mail, memos, vacation, masked, watch; no TLS-for-IMAP, no XML
cargo build --no-default-features --features dav  # add contacts, calendars, files
```

| Feature | Adds | Needs |
|---|---|---|
| `dav` | contacts, calendars, files | none |
| `notes` | notes over IMAP | OpenSSL headers on Linux |
| `completions` | `fastmail completions` | none |

Markdown support is always compiled in. The lean build needs no system libraries.

## Credentials

Set these as environment variables, or in `~/.config/fastmail/config.toml` (mode 0600). Environment variables win over the file. Only the features you use need their credential.

| Variable | Config key | Used for |
|---|---|---|
| `FASTMAIL_USERNAME` | `username` | DAV and IMAP login |
| `FASTMAIL_JMAP_TOKEN` | `[jmap] token` | Mail, memos, vacation, masked email, watch |
| `FASTMAIL_DAV_PASSWORD` | `[dav] password` | Contacts, calendars, files |
| `FASTMAIL_IMAP_PASSWORD` | `[imap] password` | Notes |
| `FASTMAIL_DDNS_TOKEN` | `[ddns] token` | Dynamic DNS |

Create the JMAP token under Settings → Privacy & Security → API tokens, with only the permissions you need:

- Email, for mail, memos, and watch
- Email submission, for sending (requires Email)
- Contacts and contact groups, if you use JMAP for contacts
- Vacation response, for vacation replies
- Masked Email, for masked addresses

Create the DAV and IMAP passwords under Settings → Privacy & Security → app passwords. The DAV password covers CardDAV, CalDAV, and WebDAV. The IMAP password needs mail access.

Create the Dynamic DNS token under Settings → Privacy & Security → API tokens, with the type set to **Dynamic DNS**. That type only appears if the account has a custom domain. It's a separate token from the JMAP one, and it works only for Dynamic DNS.

The CLI refuses a config file that other users can read.

## Usage

Every command prints JSON to stdout: `{"ok": true, "data": ...}` or `{"ok": false, "error": ...}`. Errors exit with status 1.

```sh
fastmail doctor
fastmail mail search --mailbox inbox --limit 10
fastmail calendars events CALENDAR --from 2026-01-01 --to 2027-01-01
```

Run `fastmail --help` or `fastmail <command> --help` for every option.

### Replies

`mail reply ID` creates a draft reply to an email. It sets the reply headers so the draft stays in the original's thread, and the subject gets "Re:" if it doesn't already have it. `--all` replies to everyone on the original. Your own address is left out of the recipients.

### Email format

Email is plain text by default. `mail draft` and `mail send` accept `--format text`, `markdown`, or `html`. Markdown and HTML also produce a plain-text part, so every client can read the message.

### Notes format

Notes are markdown by default. `notes get` returns markdown plus `warnings` for any formatting markdown can't express. `notes create` and `notes put` accept `--format markdown`, `html`, or `text`.

`notes put` shows a diff of the change. Nothing is written until you add `--apply`. It also refuses changes that would drop embedded images.

### Send allowlist

To limit where `mail send` can deliver, set an allowlist. It's off by default. When it's set, any message with a recipient not on the list is refused before a draft is created. Entries are full addresses, or a domain written as `@example.com`:

```toml
# ~/.config/fastmail/config.toml
allowed_recipients = ["me@example.com", "@team.example"]
```

Or set `FASTMAIL_ALLOWED_RECIPIENTS` to a comma-separated list. Drafts aren't affected.

### Watching for new mail

`fastmail watch` prints one JSON line per new email until interrupted:

```sh
fastmail watch | jq -r '.subject'
```

It uses JMAP push (server-sent events) and reconnects with backoff. A dropped connection is caught up on reconnect, so mail that arrives during an outage is still reported. The last-seen state is kept in `~/.config/fastmail/state` (override with `FASTMAIL_STATE`). The first run records the current state and prints nothing. Flag and move changes aren't reported, only new mail.

### Dynamic DNS

`fastmail dynamic-dns update home.example.com` points a Dynamic DNS record at the address the request came from. The record has to exist first: in Settings → Domains, open the domain, choose Customise DNS, and add an entry with the type set to Dynamic DNS and the subdomain you want.

To set an address yourself, pass `--ip` for IPv4 or `--ipv6` for IPv6:

```sh
fastmail dynamic-dns update home.example.com --ip 203.0.113.5
```

Leaving both out lets Fastmail use the address it sees. The output says whether the record changed:

```json
{"ok": true, "data": {"hostname": "home.example.com", "result": "good", "changed": true, "address": "203.0.113.5"}}
```

`result` is `good` when the record moved, `nochg` when it already pointed there. Both count as success. From cron, run it every five minutes or so:

```cron
*/5 * * * * /Users/you/.cargo/bin/fastmail dynamic-dns update home.example.com
```

Credentials come from the environment or the config file, so the cron job needs `FASTMAIL_USERNAME` and `FASTMAIL_DDNS_TOKEN` set.

### Shell completions

```sh
fastmail completions bash > ~/.local/share/bash-completion/completions/fastmail   # or zsh, fish
```

## Limitations

- Send needs the Email submission permission on the JMAP token. The send allowlist is opt-in.
- Calendar writes have no approval step inside the CLI. Use `--dry-run` to show a change first.
- Plain-text email shows links as bare URLs. Use `--format html` if you need link text.
- Masked email delete marks the address deleted, so mail to it bounces, but the record stays in your list. The API can't erase it.
- Watch reports new mail only, not flag or move changes.
- Notes can't be edited through the CLI if they contain images added from Apple devices. The CLI refuses changes that would drop them.

## Status

Tested against a live account: mail, memos, drafts, send, vacation replies, masked email, contacts, calendars, files, notes (including edits from an Apple device), and watch, including reconnecting after a dropped connection.

## License

MIT. See [LICENSE](LICENSE).
