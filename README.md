# Fastmail CLI for AI Agents

Command-line access to Fastmail: mail, memos, contacts, calendars, files, notes, vacation replies, masked email, and a live stream of new mail.

JMAP is the primary path. CalDAV, CardDAV, WebDAV, and IMAP cover what JMAP doesn't.

## Features

| Area | Commands | Protocol | Credential |
|---|---|---|---|
| Mail: search, read, attachments | `mail search`, `mail get`, `mail attachments` | JMAP | JMAP token (Email) |
| Mail: flag, read, move, trash | `mail read`, `mail unread`, `mail flag`, `mail unflag`, `mail move`, `mail trash` | JMAP | JMAP token (Email) |
| Drafts | `mail draft` | JMAP | JMAP token (Email) |
| Send | `mail send` | JMAP | JMAP token (Email, Email submission) |
| Memos | `memos list` | JMAP | JMAP token (Email) |
| Vacation replies | `vacation show`, `vacation set` (with `--on`/`--off`, `--from`, `--to`, `--subject`, `--file`, `--clear`) | JMAP | JMAP token (Vacation response) |
| Masked email | `masked list`, `masked create`, `masked enable`, `masked disable`, `masked delete --yes` | JMAP | JMAP token (Masked Email) |
| New mail stream | `watch` | JMAP push | JMAP token (Email) |
| Contacts | `contacts list`, `contacts create`, `contacts update`, `contacts delete --yes` | CardDAV | DAV app password |
| Calendars | `calendars list`, `calendars events`, `calendars create`, `calendars update`, `calendars delete --yes` | CalDAV | DAV app password |
| Files | `files ls`, `files get`, `files put`, `files mv`, `files rm --yes` | WebDAV | DAV app password |
| Notes | `notes list`, `notes get`, `notes create`, `notes put`, `notes delete --yes` | IMAP | IMAP app password |
| Health check | `doctor` | all | all configured |
| Shell completions | `completions bash\|zsh\|fish` | none | none |

Every delete needs `--yes`. Anything that changes an existing note, contact, or event shows the change first and writes only when you pass `--apply` (notes) or name the field to change (contacts, calendars).

## Install

You need Rust 1.85 or newer, and the system libraries in [Requirements](#requirements).

Install straight from GitHub. This puts `fastmail` in `~/.cargo/bin`:

```sh
cargo install --git https://github.com/johnagan/fastmail-cli
```

To leave out features you don't use, pass the same flags as [Build options](#build-options):

```sh
cargo install --git https://github.com/johnagan/fastmail-cli --no-default-features
cargo install --git https://github.com/johnagan/fastmail-cli --no-default-features --features dav
```

Or install from a clone:

```sh
git clone https://github.com/johnagan/fastmail-cli
cd fastmail-cli
cargo install --path .
```

To upgrade, run the same command again with `--force`. Check the install with `fastmail doctor`.

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

Everything is on by default. To keep dependencies lean, turn features off:

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

Create the JMAP token under Settings → Privacy & Security → API tokens, with only the permissions you need:

- Email, for mail, memos, and watch
- Email submission, for sending (requires Email)
- Contacts and contact groups, if you use JMAP for contacts
- Vacation response, for vacation replies
- Masked Email, for masked addresses

Create the DAV and IMAP passwords under Settings → Privacy & Security → app passwords. The DAV password covers CardDAV, CalDAV, and WebDAV. The IMAP password needs mail access.

The CLI refuses a config file that other users can read.

## Usage

Every command prints JSON to stdout: `{"ok": true, "data": ...}` or `{"ok": false, "error": ...}`. Errors exit with status 1.

```sh
fastmail doctor
fastmail mail search --mailbox inbox --limit 10
fastmail calendars events CALENDAR --from 2026-01-01 --to 2027-01-01
```

Run `fastmail --help` or `fastmail <command> --help` for every option.

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

### Shell completions

```sh
fastmail completions bash > ~/.local/share/bash-completion/completions/fastmail   # or zsh, fish
```

## Limitations

- Send needs the Email submission permission on the JMAP token. The send allowlist is opt-in.
- Drafts and sends don't support attachments yet.
- Plain-text email shows links as bare URLs. Use `--format html` if you need link text.
- Masked email delete marks the address deleted, so mail to it bounces, but the record stays in your list. The API can't erase it.
- Watch reports new mail only, not flag or move changes.
- Notes can't be edited through the CLI if they contain images added from Apple devices. The CLI refuses changes that would drop them.
- Dynamic DNS is not supported. It needs a domain whose DNS Fastmail hosts.

## Status

Tested against a live account: mail, memos, drafts, send, vacation replies, masked email, contacts, calendars, files, notes (including edits from an Apple device), and watch, including reconnecting after a dropped connection.

## License

MIT. See [LICENSE](LICENSE).
