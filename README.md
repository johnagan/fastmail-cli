# fastmail-cli

CLI for Fastmail. Read, search, send, manage and *watch* email from your terminal or AI assistant, and manage calendars, files, and notes from the CLI.

## Features

| Feature               | Description                                                            |
| --------------------- | ---------------------------------------------------------------------- |
| **Email**             | List, search, read, send, reply, forward, threads, identity selection, HTML bodies, file attachments |
| **Mailboxes**         | Create, rename, reparent, subscribe, reorder, and delete folders; move emails |
| **Mail management**   | Mark spam/read, star/unstar, import raw messages, and query search snippets |
| **Vacation responses** | Read, edit, enable/disable, and schedule out-of-office responses       |
| **Real-time**         | `fastmail watch` streams arriving mail as NDJSON over JMAP push — pipe it into a shell loop. Also a GraphQL subscription |
| **Contacts**          | JMAP contacts, address books, groups, photos, native JSContact editing, copy and sync; CardDAV fallback |
| **Calendars**         | CLI + MCP: calendar/event CRUD, recurrence expansion, free/busy and iTIP scheduling via CalDAV |
| **Files**             | CLI + MCP: browse, upload, download, move/rename/copy, create folders, and delete via WebDAV |
| **Notes**             | CLI + MCP: list, search, read, edit plain-text/HTML notes and attachments, and delete via JMAP |
| **Drafts**            | CLI + MCP: list/read, edit, send and discard existing drafts           |
| **Bulk mail**         | Keywords, flags, mailbox memberships, permanent deletion, and Trash cleanup |
| **Synchronization**   | Full snapshots and created/updated/deleted changes, with persistent account-bound caches |
| **API access**        | API-token authentication, account selection, submission tracking/cancellation, identity editing, raw parse/export, push subscriptions |
| **Attachments**       | Download files, extract text, resize images                            |
| **Text Extraction**   | 56 formats via [kreuzberg](https://github.com/kreuzberg-dev/kreuzberg) |
| **Image Resizing**    | `--max-size` to resize images on download                              |
| **Masked Email**      | Create, list, enable/disable aliases                                   |
| **MCP Server**        | Claude integration via Model Context Protocol                          |
| **Shell Completions** | Bash, Zsh, Fish, PowerShell                                            |
| **JSON Output**       | All commands output JSON for scripting                                 |

## Compared to Fastmail's official MCP

Fastmail [ships an official MCP server](https://www.fastmail.com/blog/an-mcp-server-for-fastmail/)
(hosted at `api.fastmail.com/mcp`, OAuth with read/write/send scopes). It's
zero-setup and exposes calendar, notes, and org directory tools directly to AI
clients. `fastmail-cli` offers calendar, file, and note commands in its CLI and is the self-hosted
alternative: a CLI *and* an MCP server you run yourself, open source, with
masked email, attachment text extraction, spam-filter training, and a real-time
stream of incoming mail the official server doesn't offer — plus full custody of
the data path.

|                                                            | `fastmail-cli`                     | Fastmail official MCP            |
| ---------------------------------------------------------- | ---------------------------------- | -------------------------------- |
| Interface                                                  | CLI **and** MCP (stdio / HTTP)     | MCP only                         |
| Setup                                                      | install + run the binary           | add URL + OAuth, nothing to run  |
| Auth                                                       | API token (+ app password for DAV services) | OAuth: `read` / `write` / `send` |
| Hosting / data path                                        | your machine or your own server    | Fastmail-hosted                  |
| Source                                                     | open source (MIT), extensible      | proprietary                      |
| Email: read / search / threads                             | ✅ (rich search filters)           | ✅                               |
| Send / reply / forward (preview → confirm/draft)           | ✅                                 | ✅                               |
| Drafts: edit / send / discard                              | ✅ (CLI + MCP)                     | —                                |
| Move / archive / mark read                                 | ✅                                 | ✅                               |
| Real-time stream of incoming mail                          | ✅ (CLI + GraphQL subscription)    | —                                |
| Mark as spam (+ trains the filter)                         | ✅                                 | —                                |
| Masked Email (create / enable / disable / delete)          | ✅                                 | —                                |
| Attachments: text extraction (56 formats) + image resize   | ✅                                 | —                                |
| Contacts (create / update / delete / search)               | ✅ (JMAP; CardDAV fallback)        | ✅                               |
| Org directory search                                       | —                                  | ✅                               |
| Calendar                                                   | ✅ (CLI, CalDAV)                   | ✅                               |
| Notes                                                      | ✅ (CLI + MCP; HTML/attachments)   | ✅                               |
| Files                                                      | ✅ (CLI, WebDAV)                   | —                                |
| Identities / signatures                                    | ✅                                 | ✅                               |

Use Fastmail's for zero maintenance and calendar/notes access through MCP;
use `fastmail-cli` for a scriptable CLI, mail that streams as it arrives, the
masked-email / attachment-extraction / spam-training tooling, or to self-host and
keep the data path yours. Streaming is the one difference that isn't a matter of
scope: MCP tools are request/response, so a hosted MCP server cannot hand you a
subscription that never returns, whichever tools it grows.
(`fastmail-cli`'s column covers its CLI and GraphQL schema; Fastmail's is
its current hosted tool set and may grow.)

## Quick Start

### Installation

#### From GitHub Releases (recommended for mise)

```bash
# Add to mise config
mise use -g "github:radiosilence/fastmail-cli"
```

#### From Source

```bash
cargo install --git https://github.com/radiosilence/fastmail-cli
```

### Authentication

1. Generate an API token at [Fastmail Settings > Privacy & Security > Integrations > API tokens](https://app.fastmail.com/settings/security/tokens)
2. Auth with the CLI — the token is read from stdin so it stays out of shell history, `ps`, and the process environment:

```bash
# interactive — paste the token at the prompt
fastmail auth

# non-interactive — pipe from a password manager, file, or env var
echo "$FASTMAIL_TOKEN" | fastmail auth
```

The positional form `fastmail auth YOUR_TOKEN` still works for backward compatibility, but the stdin form is preferred.

Token is stored in `~/.config/fastmail-cli/config.toml` with `0600` permissions (directory `0700`). The file is written atomically via rename, and the path is refused if it's a symlink.

#### Account selection

```bash
fastmail accounts
fastmail --account SHARED_ACCOUNT_ID list mailboxes
fastmail --account CONTACTS_ACCOUNT_ID contacts list
```

`--account` is global; `FASTMAIL_ACCOUNT_ID` provides its environment default.
Otherwise, each capability uses its primary account. The selected account must
advertise the operation's capability. HTTP MCP/GraphQL accepts
`X-Fastmail-Account`; cached clients are isolated by token and account.
`activeAccount(kind: EMAIL)` exposes the effective account through GraphQL.

### Configuration

Credentials can be set via environment variables or config file. Env vars take precedence.

**Environment variables:**

```bash
export FASTMAIL_API_TOKEN="fmu1-..."      # JMAP: email, notes, contacts, vacation responses
export FASTMAIL_USERNAME="you@fastmail.com"  # DAV: contact fallback, calendars, files
export FASTMAIL_APP_PASSWORD="xxxx..."    # App password with the relevant DAV permissions
```

**Config file** (`~/.config/fastmail-cli/config.toml`):

```toml
[core]
api_token = "fmu1-..."

[contacts]
username = "you@fastmail.com"
app_password = "xxxx..."
```

The `auth` command only sets `[core].api_token`. JMAP contacts use this token when
the session advertises `urn:ietf:params:jmap:contacts`. For the CardDAV fallback,
add the `[contacts]` section manually or use the environment variables.

Calendar and file commands also use `FASTMAIL_USERNAME` and
`FASTMAIL_APP_PASSWORD`. Generate an app password with **Calendar (CalDAV)** and
**Files (WebDAV)** access for these commands. API tokens cannot authenticate to
these DAV services. Use your account's original login username, not an alias.

For config-file credentials, add:

```toml
[dav]
username = "you@fastmail.com"
app_password = "xxxx..."
```

Calendar/files resolve each credential from the environment first, then `[dav]`,
then the existing `[contacts]` fields. Notes use the JMAP API token, with mail
read/write access for mutations.

## Usage

All output is JSON for easy scripting with `jq`.

### List Mailboxes

```bash
fastmail list mailboxes
```

### Manage Mailboxes

Use mailbox IDs returned by `mailbox list` for edits, deletion, and parent references.

```bash
fastmail mailbox list
fastmail mailbox create Projects --parent PARENT_ID
fastmail mailbox update MAILBOX_ID --name Work --sort-order 5 --subscribed true
fastmail mailbox update MAILBOX_ID --root
fastmail mailbox delete MAILBOX_ID -y
```

Nonempty mailbox deletion is rejected unless `--remove-emails` is supplied.
That option removes the mailbox's messages as well.

### Vacation Responses

```bash
fastmail vacation get
fastmail vacation set --subject "Out of office" --text-body "Back on Monday." \
  --from 2026-10-16T00:00:00Z --to 2026-10-20T00:00:00Z --enabled true
fastmail vacation disable
fastmail vacation enable
fastmail vacation set --clear-from --clear-to
```

Only supplied settings change. Dates are UTC RFC 3339 instants; clearing a date
removes that bound. Requires the `urn:ietf:params:jmap:vacationresponse` capability.

### Import Raw Messages

```bash
fastmail import message.eml --mailbox Archive --keyword '$seen' \
  --received-at 2026-10-16T12:00:00Z
```

Imports the original RFC 5322 bytes through `Email/import`, without sending the
message. The default destination is INBOX. Repeat `--keyword` to set more keywords;
omit `--received-at` to let the server choose the reception time.

### Search Snippets

```bash
# Search, then fetch query-specific highlighted excerpts
fastmail snippets --text "project launch" --mailbox INBOX --limit 20

# Fetch snippets for known message IDs using the same search text
fastmail snippets --text "project launch" --email-id EMAIL_ID
```

Returns the `SearchSnippet/get` result, including `list` and `notFound`.
Snippets may contain server-generated highlight markup.

### Star / Unstar

```bash
fastmail star EMAIL_ID
fastmail unstar EMAIL_ID
fastmail search --flagged
```

Changes only `$flagged`, preserving read state and all other keywords.

### Bulk Mail, Tags, Memberships, and Deletion

```bash
fastmail email update EMAIL_ID OTHER_ID --add-keyword project --remove-keyword '$answered'
fastmail email update EMAIL_ID OTHER_ID --read true --starred false
fastmail email update EMAIL_ID --add-mailbox ARCHIVE_ID --remove-mailbox INBOX_ID
fastmail email update EMAIL_ID OTHER_ID --move-to DESTINATION_ID
fastmail email delete EMAIL_ID OTHER_ID -y
fastmail email empty-trash -y
fastmail email export EMAIL_ID --output message.eml
fastmail email parse message.eml
fastmail --account DESTINATION_ACCOUNT email copy SOURCE_ACCOUNT copies.json
```

Keyword and membership patches preserve unmentioned values. `--move-to` replaces
memberships; mailbox references here are IDs. Bulk writes respect the server's
`maxObjectsInSet` and return per-record errors. If some records fail, the CLI
returns `success:false`, retains the result/error maps in `data`, and exits with
status 1. Operations across multiple requests are not atomic.

`email delete` permanently destroys the specified messages. `empty-trash`
destroys messages that are only in Trash; messages in other folders retain those
memberships and are removed from Trash. Each cleanup batch checks the current
state before writing. Export saves the original message bytes, refusing an
existing destination unless `--overwrite` is supplied. Parse uploads a message
for `Email/parse` without importing or sending it.

Copy files are native creation-ID maps, for example
`{"copy1":{"id":"SOURCE_EMAIL_ID","mailboxIds":{"DESTINATION_MAILBOX_ID":true}}}`.
`--destroy-original` requests removal from the source after successful copying.

### Identities and Submission Tracking

```bash
fastmail identities list
fastmail identities update IDENTITY_ID --name 'My Name' --text-signature 'Regards'
fastmail identities update IDENTITY_ID --html-signature '<b>Regards</b>' --reply-to reply@example.com
fastmail submissions list --limit 50
fastmail submissions get SUBMISSION_ID OTHER_ID
fastmail submissions query submission-query.json
fastmail submissions cancel SUBMISSION_ID
```

Submission records include `sendAt`, `undoStatus`, and server-provided delivery
status. Cancellation sets `undoStatus` to `canceled`; the server rejects it if
the message is no longer cancellable. Identity mutations preserve omitted fields
and are subject to server policy. An empty `--reply-to` or `--bcc` clears the list.

### Full Changes and Persistent Synchronization

```bash
fastmail state email
fastmail changes email STATE
fastmail query-changes email original-query-with-sinceQueryState.json
fastmail query mailbox mailbox-query.json
fastmail sync email --cache ./cache/emails.json
fastmail sync contact --cache ./cache/contacts.json --watch 60
```

Supported state/change/cache types: `email`, `mailbox`, `thread`, `identity`,
`submission`, `contact`, and `addressbook`. Query/queryChanges support `email`,
`mailbox`, `submission`, and `contact`; JSON files contain the native arguments.
Manual change calls return one protocol page; follow `hasMoreChanges` with the
returned `newState`.

`sync` maintains a private JSON snapshot containing complete returned records,
account identity, type, and cursor. It follows all changes pages, reconciles
updates/deletions, replays changes that occurred during initial enumeration, and
rebuilds after `cannotCalculateChanges`. Enumeration uses anchors to tolerate
arrivals and removals before the current page. Cursor updates are atomic and
are saved only after successful reconciliation. Cache files are bound to their
account/type and locked against concurrent writers. `--watch SECONDS` repeats
the same reconciliation until interrupted. This is a one-way local snapshot;
attachment blobs remain on the server and local JSON edits are not uploaded.

### Push Subscriptions

```bash
fastmail push list
fastmail push create push.json
fastmail push update SUBSCRIPTION_ID verification.json
fastmail push delete SUBSCRIPTION_ID
```

Creation/update files use native `PushSubscription` fields such as
`deviceClientId`, `url`, `types`, `keys`, `expires`, and `verificationCode`.
Deliver and acknowledge verification codes through your push receiver before
expecting notifications. These subscriptions are session-scoped; account
selection does not add an `accountId` to their requests. `fastmail watch`
continues to provide SSE-based arrival watching independently.

### List Emails

```bash
# Default: INBOX, 50 emails
fastmail list emails

# Specific mailbox and limit
fastmail list emails --mailbox Sent --limit 10
```

### Get Email Details

```bash
fastmail get EMAIL_ID
```

### Search

Search uses JMAP filter flags (all filters are ANDed together):

```bash
# Full-text search
fastmail search --text "meeting notes"

# Filter by header fields
fastmail search --from "alice@example.com"
fastmail search --to "bob" --subject "project"

# Filter by mailbox
fastmail search --mailbox Sent --limit 10

# Attachments and size
fastmail search --has-attachment
fastmail search --min-size 1000000  # > 1MB

# Date range (ISO 8601)
fastmail search --after 2024-01-01 --before 2024-12-31

# Status filters
fastmail search --unread
fastmail search --flagged

# Combine filters
fastmail search --from "boss" --has-attachment --after 2024-06-01 --limit 20
```

Available flags: `--text`, `--from`, `--to`, `--cc`, `--bcc`, `--subject`, `--body`, `--mailbox`, `--has-attachment`, `--min-size`, `--max-size`, `--before`, `--after`, `--unread`, `--flagged`

#### Nested filters and pagination

`--filter` accepts native JMAP filter JSON; `--filter-file` reads it from a file.
Use `AND`, `OR`, and `NOT` operators to nest conditions. Mailbox references inside
the JSON are IDs; `--mailbox` remains a name/role and is ANDed with the JSON.
Native conditions also support `header`, `hasKeyword`, and thread-keyword filters.

```bash
fastmail search --filter '{"operator":"OR","conditions":[{"from":"alice@example.com"},{"from":"bob@example.com"}]}' \
  --mailbox INBOX --sort '[{"property":"size","isAscending":false}]' --total
fastmail search --filter-file filter.json --anchor EMAIL_ID --anchor-offset 1 --limit 25
fastmail search --position -25 --collapse-threads
```

Advanced queries return `{emails, ids, position, total, queryState}` in `data`.
`total` is requested only with `--total`; ordinary flag-only search retains its
existing output format. Anchors are email IDs in the same filtered/sorted query.

### Watch for New Mail

Block and emit one JSON object per line as mail arrives, so a shell loop can act
on it:

```bash
# Everything that arrives, anywhere in the account
fastmail watch

# Just the inbox
fastmail watch --mailbox inbox

# Pipe into a loop
fastmail watch --mailbox inbox | while read -r line; do
  echo "$line" | jq -r '.data.subject'
done

# Include bodies and attachment metadata, not just summaries
fastmail watch --full

# Fall back to polling every 60s where a long-lived connection won't survive
fastmail watch --poll 60
```

Output is the same `{"success":true,"data":{...}}` envelope as every other
command, one compact line per email, flushed as it is written — so `jq` filters
and `read` loops both work unbuffered.

It uses JMAP's push channel (`eventSourceUrl`), but treats a notification purely
as a signal to look again: the state cursor lives in the CLI, and each wake-up
runs `Email/changes` against it. Dropped connections are reconciled on reconnect
and `--poll` takes the identical path, so a missed notification costs latency
rather than mail. Only *new* messages are reported — flag and folder changes to
existing mail are not arrivals.

Reconnects, and the rare case where the server has discarded change history and
the cursor has to resync, are reported on stderr; stdout stays pure NDJSON.

### List Identities

View available sender identities (useful for `--from`):

```bash
fastmail list identities
```

### Send Email

```bash
fastmail send \
  --to "alice@example.com, bob@example.com" \
  --subject "Hello" \
  --body "Message body here"

# With CC/BCC
fastmail send \
  --to "alice@example.com" \
  --cc "bob@example.com" \
  --bcc "secret@example.com" \
  --subject "Hello" \
  --body "Message"

# Send from a specific identity/alias
fastmail send \
  --to "alice@example.com" \
  --from "alias@yourdomain.com" \
  --subject "Hello" \
  --body "Message"

# HTML email body (inline or from file)
fastmail send \
  --to "alice@example.com" \
  --subject "Newsletter" \
  --body "Plain text fallback" \
  --html-body "<h1>Hello</h1><p>Rich content here</p>"

fastmail send \
  --to "alice@example.com" \
  --subject "Report" \
  --body "See attached" \
  --html-file ./email.html

# File attachments (repeatable)
fastmail send \
  --to "alice@example.com" \
  --subject "Documents" \
  --body "Please review" \
  -a report.pdf -a data.xlsx
```

### Move Email

```bash
fastmail move EMAIL_ID --to Archive
fastmail move EMAIL_ID --to Trash
```

### Mark as Spam

```bash
# Requires confirmation
fastmail spam EMAIL_ID

# Skip confirmation
fastmail spam EMAIL_ID -y
```

### Mark as Read/Unread

```bash
# Mark as read
fastmail mark-read EMAIL_ID

# Mark as unread
fastmail mark-read EMAIL_ID --unread
```

### Download Attachments

```bash
# Download to current directory
fastmail download EMAIL_ID

# Download to specific directory
fastmail download EMAIL_ID --output ~/Downloads

# Extract text content as JSON (PDF, DOCX, DOC, TXT)
fastmail download EMAIL_ID --format json

# Resize images to max 500KB
fastmail download EMAIL_ID --max-size 500K
```

Text extraction uses [kreuzberg](https://github.com/kreuzberg-dev/kreuzberg) and supports 56 formats:

- **Documents**: PDF, DOC, DOCX, ODT, RTF
- **Spreadsheets**: XLS, XLSX, ODS, CSV, TSV
- **Presentations**: PPT, PPTX
- **eBooks**: EPUB, FB2
- **Markup**: HTML, XML, Markdown, RST, Org
- **Data**: JSON, YAML, TOML
- **Email**: EML, MSG
- **Archives**: ZIP, TAR, GZ, 7z
- **Academic**: BibTeX, LaTeX, Typst, Jupyter notebooks

### Reply to Email

```bash
# Reply to sender only
fastmail reply EMAIL_ID --body "Thanks for your message"

# Reply all
fastmail reply EMAIL_ID --body "Thanks everyone" --all

# Reply with additional CC/BCC
fastmail reply EMAIL_ID --body "Response" --cc "boss@example.com"

# Reply from a specific identity
fastmail reply EMAIL_ID --body "Thanks" --from "alias@yourdomain.com"
```

### Forward Email

```bash
fastmail forward EMAIL_ID \
  --to "colleague@example.com" \
  --body "FYI - see below"

# Forward from a specific identity
fastmail forward EMAIL_ID \
  --to "colleague@example.com" \
  --from "alias@yourdomain.com" \
  --body "FYI"
```

### Shell Completions

```bash
# Bash
fastmail completions bash >> ~/.bashrc

# Zsh
fastmail completions zsh >> ~/.zshrc

# Fish
fastmail completions fish > ~/.config/fish/completions/fastmail.fish
```

### Contacts

Contact CRUD prefers JMAP (RFC 9610) using the API token. When no API token is
configured, or the session does not advertise JMAP contacts, `auto` uses CardDAV.
Choose `--backend jmap` to require JMAP, or `--backend carddav` to explicitly use
the retained CardDAV implementation. Authentication/server errors on JMAP are
reported directly rather than retried on another backend.

```bash
# Optional CardDAV fallback credentials
export FASTMAIL_USERNAME="you@fastmail.com"
export FASTMAIL_APP_PASSWORD="your-app-password"

# List all contacts
fastmail contacts list
fastmail contacts --backend jmap list
fastmail contacts --backend carddav list

# Search by name, email, or organization
fastmail contacts search "alice"

# Create a new contact
fastmail contacts create --name "Jane Doe" --email "jane@example.com" --organization "Acme Corp"

# Update an existing contact (only provided fields are changed)
fastmail contacts update CONTACT_ID --organization "New Corp" --title "CEO"

# Delete a contact (requires -y confirmation)
fastmail contacts delete CONTACT_ID -y
```

Generate an app password at [Fastmail Settings > Privacy & Security > Integrations > App passwords](https://app.fastmail.com/settings/security/devicekeys).

JMAP creates contacts in the writable default address book, falling back to the
first writable book. Lists/searches follow all query pages. Output keeps the
existing contact shape; JMAP IDs are `ContactCard.id` values, while CardDAV IDs
are vCard UIDs. Use IDs with the same backend that returned them. Unspecified
update fields are preserved; `--email ''` and `--phone ''` clear those lists.

#### Address books and full JSContact data

The following commands require JMAP, including when the default backend is `auto`.

```bash
fastmail contacts get CONTACT_ID
fastmail contacts addressbooks list
fastmail contacts addressbooks get BOOK_ID
fastmail contacts addressbooks create Team --description 'Team contacts'
fastmail contacts addressbooks update BOOK_ID book-patch.json
fastmail contacts addressbooks default BOOK_ID
fastmail contacts addressbooks subscribe BOOK_ID --subscribed false
fastmail contacts addressbooks delete BOOK_ID -y
fastmail contacts create --name Alice --email alice@example.com --address-book BOOK_ID

fastmail contacts card-create card.json --address-book BOOK_ID
fastmail contacts card-update CONTACT_ID card-patch.json
fastmail contacts photo CONTACT_ID portrait.png
fastmail contacts details CONTACT_ID --birthday 1990-01-02 \
  --address '123 Test St, Example City' --country-code US --nickname Ally
fastmail contacts details CONTACT_ID --clear-birthday --clear-address
fastmail contacts group-create Team --member PERSON_UID --address-book BOOK_ID
fastmail contacts group-members GROUP_ID --member PERSON_UID --member OTHER_UID
```

`get` returns the native `ContactCard/get` response. `card-create` accepts a JSContact
Card; the CLI supplies `@type`, version, and a new UID if omitted, and chooses a
writable default book when no membership is supplied. `card-update` accepts a JMAP
PatchObject, preserving omitted properties. Native files can represent all
JSContact fields: structured names, addresses, anniversaries/birthdays, media,
groups, and custom properties. For example, a patch can contain:

```json
{"name/full":"Alice Smith","anniversaries/birthday":{"kind":"birth","date":{"@type":"PartialDate","year":1990,"month":3,"day":15}}}
```

Group members are **contact UIDs**, not ContactCard IDs; `contacts get` exposes
both. `group-members` replaces the member set. Photos upload into the contacts
account and retain non-photo media. Address-book update files accept writable
properties such as `name`, `description`, `sortOrder`, `isSubscribed`, and
`shareWith`. Deleting a nonempty book requires `--remove-contents`; contacts
belonging to no other book are then destroyed.

#### Contact query, copy, and synchronization

```bash
fastmail contacts query query.json
fastmail contacts state
fastmail contacts changes STATE --max-changes 100
fastmail contacts addressbooks state
fastmail contacts addressbooks changes STATE --max-changes 100
fastmail contacts query-changes query-changes.json
fastmail contacts copy SOURCE_ACCOUNT_ID copies.json
```

Query files contain native `ContactCard/query` arguments, such as
`{"filter":{"kind":"group"},"limit":50,"calculateTotal":true}`. They support
sorting, filters, anchors, and positions. Query-changes files include
`sinceQueryState` and the original filter/sort arguments. The selected primary
contacts account supplies `accountId`.

Changes return one protocol page, including creations, updates, deletions,
`newState`, and `hasMoreChanges`. Keep requesting with `newState` until
`hasMoreChanges` is false before persisting your cursor. A discarded history
(`cannotCalculateChanges`) requires a fresh snapshot. Copy files are creation-ID
maps, e.g. `{"copy1":{"id":"SOURCE_CONTACT_ID","addressBookIds":{"DEST_BOOK_ID":true}}}`.
`--destroy-original` requests removal from the source after successful copying.

### Calendars

Calendar commands use CalDAV. Use the `href` values returned by `calendar list`
and `calendar events` as identifiers; retain URL encoding in these hrefs.

```bash
fastmail calendar list

# Bounds are UTC iCalendar timestamps, with an exclusive upper bound.
fastmail calendar events '/dav/calendars/user/you@fastmail.com/CALENDAR_ID/' \
  --after 20261001T000000Z --before 20261101T000000Z

fastmail calendar get '/dav/calendars/user/you@fastmail.com/CALENDAR_ID/EVENT.ics'
fastmail calendar create '/dav/calendars/user/you@fastmail.com/CALENDAR_ID/' event.ics

# Use the ETag returned by get/events, including the quotes in its value.
fastmail calendar update '/dav/calendars/user/you@fastmail.com/CALENDAR_ID/EVENT.ics' \
  event.ics --etag '"CURRENT_ETAG"'
fastmail calendar delete '/dav/calendars/user/you@fastmail.com/CALENDAR_ID/EVENT.ics' -y
```

Events are returned in `calendarData` as complete iCalendar text, including
recurrence rules and time-zone definitions. Queries return matching calendar
resources rather than expanding recurring events into individual occurrences.
Pass `--expand` with both `--after` and `--before` to request server-side recurrence
expansion instead.
Create/update accept complete UTF-8 `.ics` files containing `VCALENDAR` and
`VEVENT`. Updates replace the whole resource; preserve its UID when editing.
An outdated ETag produces an HTTP 412 error so you can fetch the current event
before retrying. Deletes optionally accept `--etag` too.

Calendar collections, availability, and scheduling:

```bash
fastmail calendar create-calendar '/dav/calendars/user/you@fastmail.com/work/' \
  --name Work --description 'Work events' --color '#336699'
fastmail calendar update-calendar CALENDAR_HREF --name Projects
fastmail calendar delete-calendar CALENDAR_HREF -y
fastmail calendar events CALENDAR_HREF --expand --after 20261001T000000Z --before 20261101T000000Z
fastmail calendar free-busy CALENDAR_HREF --after 20261001T000000Z --before 20261101T000000Z
fastmail calendar scheduling
fastmail calendar schedule reply.ics --outbox OUTBOX_HREF \
  --originator mailto:you@fastmail.com --recipient mailto:organizer@example.com
```

`scheduling` discovers the principal, inbox, outbox, and calendar-user addresses.
`schedule` sends a complete iTIP VCALENDAR/VEVENT with a `METHOD` such as REQUEST,
REPLY, or CANCEL; retain event UID, organizer, attendees, and sequence in the file.
It returns per-recipient `requestStatus` values, which callers must inspect.
Free/busy returns iCalendar VFREEBUSY data. These operations use the server's
CalDAV support; unsupported methods/properties are returned as errors. Calendar
deletion removes the collection and its events.

### Files

File paths are relative to your own Fastmail Files root at
`https://myfiles.fastmail.com/`. Pass literal paths, including spaces, rather
than URL-encoded paths. Listings include a decoded `path` for subsequent commands
and the original DAV `href`.

```bash
fastmail files list
fastmail files mkdir /Reports
fastmail files upload ./report.pdf '/Reports/Quarterly report.pdf'
fastmail files list /Reports/
fastmail files download '/Reports/Quarterly report.pdf' --output ./report-copy.pdf
fastmail files copy '/Reports/Quarterly report.pdf' '/Reports/Backup.pdf'
fastmail files move '/Reports/Backup.pdf' '/Reports/Renamed.pdf'
fastmail files delete '/Reports/Quarterly report.pdf' -y
```

Uploads and downloads refuse to replace existing files unless you pass
`--overwrite`. Deleting a folder with `-y` removes its contents too.
Move/copy are server-side and also work for folders; `files rename` is an alias
for `files move`. Replacing a destination requires `--overwrite`. Partial DAV
failures are reported as errors.

### Notes

Notes use the existing JMAP email connection and the mailbox with the `memos`
role (named `Memos` by default), falling back to a mailbox named `Notes`.
The commands distinguish notes from ordinary email using the
`X-Uniform-Type-Identifier: com.apple.mail-note` header.

```bash
fastmail notes list --limit 25
fastmail notes search 'shopping' --limit 10
fastmail notes get NOTE_ID
fastmail notes create --title 'Shopping list' --body $'Coffee\nBread'
fastmail notes update NOTE_ID --body $'Coffee\nBread\nMilk'
fastmail notes create --title 'Rich note' --html-body '<b>Important</b>' -a photo.png
fastmail notes update NOTE_ID --html-file note.html
fastmail notes update NOTE_ID --attachment report.pdf
fastmail notes update NOTE_ID --clear-attachments
fastmail notes delete NOTE_ID -y
```

Get returns the message's `textBody`, `htmlBody`, attachments, and `bodyValues`.
Create accepts plain text, HTML, and attachments. Update preserves omitted bodies,
mailbox memberships, the UUID, and keywords (including pinned status). Supplying
a new plain-text body drops the old HTML alternative unless new HTML is supplied
too. Attachment flags replace regular attachments; `--clear-attachments` removes
them, while referenced inline images are retained. `--attachment-manifest` also
accepts uploaded blobs. Incomplete/truncated bodies are rejected before replacement.
An update saves a new message before deleting the old one and returns a **new note
ID**. Delete is permanent and requires `-y`.

Calendars, files, and notes are also available through MCP/GraphQL. DAV fields
use request-scoped username/app-password credentials, falling back to `[dav]`
and then `[contacts]` configuration for local use. Notes use JMAP credentials.

### Masked Email

Create disposable email addresses for signups. Requires Fastmail's masked email feature.

```bash
# List all masked emails
fastmail masked list

# Create a new masked email
fastmail masked create --domain "https://example.com" --description "Example Site"

# Create with custom prefix
fastmail masked create --prefix "shopping" --description "Shopping sites"
fastmail masked create --pending --url 'passwordmanager://record/123'
fastmail masked update MASKED_EMAIL_ID --description 'New description' --domain https://example.com
fastmail masked update MASKED_EMAIL_ID --url 'passwordmanager://record/456'
fastmail masked update MASKED_EMAIL_ID --clear-url

# Enable/disable a masked email
fastmail masked enable MASKED_EMAIL_ID
fastmail masked disable MASKED_EMAIL_ID

# Delete (requires confirmation)
fastmail masked delete MASKED_EMAIL_ID -y
```

Metadata updates preserve omitted fields and do not change the address's state.
Pending addresses auto-enable on receiving mail; unused pending addresses expire
after 24 hours. GraphQL `createMaskedEmail` also accepts `pending` and `url`, and
`updateMaskedEmail` edits metadata.

### Existing Drafts and Reusable Uploads

```bash
fastmail drafts list --limit 25
fastmail drafts get DRAFT_ID
fastmail drafts update DRAFT_ID --subject 'Revised' --body 'New body'
fastmail drafts update DRAFT_ID --cc '' --clear-attachments
fastmail drafts send DRAFT_ID
fastmail drafts delete DRAFT_ID -y

fastmail upload-attachment report.pdf > upload.json
fastmail send --to alice@example.com --subject Report --body Attached \
  --attachment-manifest upload.json
fastmail drafts update DRAFT_ID --attachment-manifest upload.json
```

Draft edits preserve omitted fields and return a **new draft ID**; use it for
subsequent operations. `drafts delete` (alias `discard`) moves drafts to Trash.
Non-drafts are rejected. Upload returns `{name, contentType, blobId, size}` inside
the standard envelope. Manifest files accept that envelope, the bare object, or
an array of such objects; they reference existing blobs without re-uploading.
Repeat `--attachment-manifest` to attach more. Send, reply, forward, draft updates,
and notes accept manifests alongside local `--attachment` files. Blobs must be
available in the same account and within the server's upload retention period.

Native contacts are also exposed through GraphQL typed results with JSON inputs: `contactCards`,
`queryContactCards`, `contactChanges`, `contactQueryChanges`, `addressBookRecords`,
`createContactCard`, `updateContactCard`, `setAddressBooks`, `copyContactCards`, and
`setContactPhoto`. Use `schema_sdl` to inspect selection fields and native JSON input shapes.

## Output Format

All commands output JSON with this structure:

```json
{
  "success": true,
  "data": { ... },
  "message": "optional status message",
  "error": "error message if success=false"
}
```

### Parsing with jq

```bash
# Get unread count for INBOX
fastmail list mailboxes | jq '.data[] | select(.role == "inbox") | .unreadEmails'

# List email subjects
fastmail list emails | jq '.data.emails[].subject'

# Get email body
fastmail get EMAIL_ID | jq -r '.data.bodyValues | to_entries[0].value.value'
```

## Claude Code Skills

If you're using [Claude Code](https://claude.ai/claude-code), this repo ships skills that teach agents how to use the CLI — no need to explain flags or workflows manually.

Copy the skills into your project's `.claude/skills/` directory (or anywhere Claude Code loads skills from), then invoke them:

```
/fastmail              # full command reference + common patterns
/fastmail/search       # search filters, date ranges, workflows
/fastmail/compose      # send, reply, forward, drafts, identities
/fastmail/conversations # list, get, thread, mark-read, triage
/fastmail/attachments  # download, raw vs json, text extraction
/fastmail/masked       # masked email CRUD
/fastmail/contacts     # contact setup, list/search
```

Skills are in `.claude/skills/` in this repo. Each one includes concrete examples and agent-oriented workflow patterns.

## MCP Server (Claude Integration)

Run as an MCP server for use with Claude Desktop or other MCP clients:

```bash
fastmail mcp
```

Configure in Claude Desktop's `claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "fastmail": {
      "command": "mise",
      "args": ["x", "--", "fastmail", "mcp"],
      "env": {
        "FASTMAIL_API_TOKEN": "your-token-here",
        "FASTMAIL_USERNAME": "you@fastmail.com",
        "FASTMAIL_APP_PASSWORD": "your-app-password"
      }
    }
  }
}
```

Username and app password are optional, used for the CardDAV contacts fallback.
JMAP contacts use the API token when the contacts capability is advertised.

### HTTP transport

Three independent surfaces, each opt-in, sharing one port (default
`127.0.0.1:8080`, or pass an address to `--http`):

| Flag         | Serves                                                      |
| ------------ | ----------------------------------------------------------- |
| `--http`     | MCP streamable-HTTP at `/mcp`                               |
| `--graphql`  | plain GraphQL-over-HTTP at `/graphql`, subscriptions at `/graphql/stream` |
| `--graphiql` | the GraphiQL IDE at `/`, and its `/graphql`                 |
| `--browser`  | opens the IDE once the port is bound, implying `--graphiql` |

```bash
fastmail mcp                                   # stdio MCP, no listener
fastmail mcp --browser                         # just the IDE, opened for you
fastmail mcp --http                            # just /mcp
fastmail mcp --http 0.0.0.0:8080 --graphql     # both, explicit address
```

Asking for any surface binds the listener; there is nowhere to mount an HTTP
route over stdio. Only `--http` puts MCP on it — the transport a model connects
through and a browsable endpoint for you are separate things. `--browser`
implies `--graphiql`, since the IDE is what it opens.

`/graphql` is plain GraphQL-over-HTTP, which is what a browser speaks; `/mcp` is
MCP JSON-RPC, which it doesn't. That is why GraphiQL needs its own route rather
than pointing at the MCP one. Both share the schema, the client cache and the
credential resolution below, so the IDE sees exactly what a model sees.

`/graphql/stream` carries subscriptions over Server-Sent Events — POST the
operation, read events off the response:

```bash
curl -N http://127.0.0.1:8080/graphql/stream \
  -H 'Content-Type: application/json' \
  -d '{"query":"subscription { emails(mailbox: \"inbox\") { id subject from { email } } }"}'
```

SSE rather than WebSockets because the only subscription is a server-to-client
firehose: nothing is ever sent back up the socket, and SSE reconnects on its
own. Body fields (`textBody`, `attachments`) want `full: true` — a subscription
has no request boundary at which the batching loaders reset, so the lazy path
resolves through a loader that lives as long as the subscription. `pollSeconds`
is the same fallback as the CLI's `--poll`. MCP has no equivalent: tools are
request/response, and a subscription never returns.

**Credential resolution is the same everywhere:** the request's own header
wins, otherwise the local config (or the matching environment variable) is used.
Running it yourself, that means your own credentials with no ceremony. In a
hosted deployment there is no local config, so the fallback is absent and every
request must carry its headers, injected by a trusted upstream after it has
authenticated the caller. Authenticated JMAP clients are cached per token, so
the JMAP session handshake runs once per distinct token rather than per call.

| Header                     | Falls back to           | Needed for              |
| -------------------------- | ----------------------- | ----------------------- |
| `X-Fastmail-Token`         | `FASTMAIL_API_TOKEN`    | everything over JMAP    |
| `X-Fastmail-Username`      | `FASTMAIL_USERNAME`     | contacts (CardDAV)      |
| `X-Fastmail-App-Password`  | `FASTMAIL_APP_PASSWORD` | contacts (CardDAV)      |

JMAP contacts use `X-Fastmail-Token`. The CardDAV fallback needs the two other
headers because that protocol rejects API tokens. Each resolves on its own: a
request carrying a username header but no password gets exactly that — half a
credential, which `session { carddavConfigured }` reports as `false` — rather
than quietly completing itself from the host's local config and mixing two
users together.

Do **not** expose this to the internet without such an auth layer in front —
the header is trusted unconditionally. Equally, do not run it with local
credentials present on a non-loopback address: anything that can reach the port
gets your mailbox without needing a token at all.

The token is resolved on first query rather than at startup, so an expired one
shows up as an error in the response pane rather than a server that won't boot —
run `fastmail auth` to refresh it, or ask `session` first. **Introspection needs
no token**: it is answered from the schema without touching Fastmail, so
GraphiQL's docs, autocomplete and explorer work before you have working
credentials. Queries that select any real field still authenticate as normal.

#### Checking a connection

`session` answers whether a token still works, and needs no mail to exist to do
it:

```graphql
{ session { status username primaryAccountId capabilities carddavConfigured detail } }
```

One `GET /jmap/session` — the handshake Fastmail only completes for a request it
has authenticated. `status` is `CONNECTED`, `INVALID_CREDENTIALS`, or
`UNREACHABLE`, and a bad connection is reported there rather than raised as a
GraphQL error: it is the answer to this question, not a failure to answer it.
The split is the one a UI acts on — tell the user to re-authenticate, or tell
them to wait. A 401 is the only thing that reads as a credential verdict;
everything else (5xx, rate limiting, timeouts, DNS) is `UNREACHABLE`, because
telling someone their token is dead over an outage would be a lie. `detail`
carries the reason in prose for a tooltip; branch on `status`, which won't be
reworded.

It re-runs the handshake rather than reading the cached client — clients are
cached per token for the life of the process, so a cached answer would keep
reporting success long after a revocation, which is the case this exists to
catch. When connected it also reports the accounts the token reaches and the
capability URNs it was granted, which is what says whether masked email and
sending are available at all.

Contacts prefer JMAP when `capabilities` includes `urn:ietf:params:jmap:contacts`.
Otherwise, `carddavConfigured` reports whether credentials for the CardDAV
fallback are present. It does not validate those credentials and is independent
of the JMAP connection status.

#### Mailbox, vacation, import, snippets, and stars over GraphQL

These operations are also available through MCP's `graphql` tool. Management
operations return typed objects with selectable fields. Native record properties
and arbitrary protocol input objects remain available as JSON fields.

```graphql
mutation { createMailbox(name: "Projects") { id } }
mutation { updateMailbox(id: "MAILBOX_ID", input: {name: "Work", isSubscribed: true}) { success updated } }
mutation { deleteMailbox(id: "MAILBOX_ID", removeEmails: false) { success destroyed } }
{ vacationResponse { isEnabled subject textBody } }
mutation { setVacationResponse(input: {isEnabled: true, textBody: "Back Monday."}) { success } }
mutation { importEmail(messageBase64: "BASE64_RFC5322_BYTES", mailboxId: "MAILBOX_ID") { id blobId } }
{ searchSnippets(text: "project", emailIds: ["EMAIL_ID"]) { snippets { emailId subject preview } notFound } }
{ addressBooks { id name isDefault myRights { mayWrite } } }
mutation { starEmail(emailId: "EMAIL_ID", starred: true) { success } }
mutation { starEmail(emailId: "EMAIL_ID", starred: false) { success } }
```

#### Bulk mail, synchronization, calendars, and files over GraphQL

```graphql
mutation { patchEmails(ids: ["EMAIL_ID", "OTHER_ID"], keywords: {project: true}) {
  success updated notUpdated { id kind description }
} }
mutation { destroyEmails(ids: ["EMAIL_ID"]) { success destroyed notDestroyed { id kind } } }
mutation { emptyTrash { success updated destroyed } }
mutation { updateIdentity(id: "IDENTITY_ID", patch: {textSignature: "Regards"}) { success } }
{ submissions(arguments: {limit: 20, filter: {undoStatus: "pending"}}) { ids queryState } }
{ records(kind: SUBMISSION, ids: ["SUBMISSION_ID"]) { records { id properties } notFound } }
mutation { cancelSubmissions(ids: ["SUBMISSION_ID"]) { success updated notUpdated { id kind } } }
{ changes(kind: EMAIL, sinceState: "STATE") { newState hasMoreChanges created updated destroyed } }
{ queryChanges(kind: EMAIL, arguments: {sinceQueryState: "QUERY_STATE"}) {
  newQueryState removed added { id index }
} }
{ rawEmail(id: "EMAIL_ID") { base64 size } }
{ pushSubscriptions { records { id properties } } }
{ contactCards(ids: ["CONTACT_ID"]) { state records { id properties } notFound } }
{ notes(limit: 10) { total notes { id subject preview } } }
{ note(id: "NOTE_ID") { id uuid textBody htmlBody attachments { name contentType blobId } } }
mutation { createNote(title: "Notes", htmlBody: "<b>Important</b>") { id } }
{ calendars { href name etag } }
{ calendarEvents(calendar: "CALENDAR_HREF", after: "20300101T000000Z", before: "20300201T000000Z", expand: true) {
  href etag calendarData
} }
{ files(path: "/Reports/") { path name collection size etag } }
{ fileDownload(path: "/Reports/report.pdf") { base64 size } }
mutation { uploadFile(path: "/hello.txt", base64: "aGVsbG8=") { success href etag } }
mutation { transferFile(source: "/hello.txt", destination: "/copy.txt", copy: true) { success href } }
```

`SetResult` separates successful IDs from per-record errors. Arbitrary keyword
names (including `$` or `/`) can be supplied through GraphQL JSON variables.
`records(kind: SUBMISSION)` exposes delivery-status fields in each record's
`properties`. Native record properties remain JSON so extensions and full
JSContact data are preserved. `copyEmails`, `parseEmail`, `setPushSubscriptions`,
calendar collection/event mutations, free/busy, scheduling, folder creation,
and file deletion are also available; inspect `schema_sdl` for their arguments.

Calendar/file requests use `X-Fastmail-Username` and `X-Fastmail-App-Password`,
or local DAV configuration. DAV resolvers use the credentials injected for that
request. `putCalendarEvent` creates an absent event by
default; supply its current `etag` to replace it conditionally. Scheduling
returns per-recipient statuses to inspect.

The MCP server exposes **2 tools** via a GraphQL interface:

- **`graphql`** — executes any GraphQL query or mutation. Its description carries a slimmed schema for everyday mail: the queries, the `EmailFilter` tree, the common `Email` fields, the connection shape and the PREVIEW→CONFIRM send flow
- **`schema_sdl`** — the full SDL, with an optional `types` list (e.g. `["MutationRoot", "Attachment"]`) returning only those definitions

The split is about round trips. The SDL is ~27KB, most of it the doc comments
that make it worth reading, and it was previously the only way to learn
anything — so reading mail cost a 27KB fetch first, and cost it again whenever
the connection dropped. The common case is now answered where the model is
already looking, and `schema_sdl` is for what the sketch explicitly says it
doesn't cover: attachment payloads, masked email, contacts, identities,
`moveEmail`, `markAsRead`, `markAsSpam`, and the remaining filter and sort
options. `types` keeps that second hop small too. Named types come back whole
but their references don't, so name those as well; an unrecognised name is
reported alongside the list of names that do exist, rather than silently
dropped.

Both the worked examples and the inlined sketch are checked by the test suite —
the examples are executed against the real schema, and every field name in the
sketch must exist in it. A cheat sheet that outlives a rename is worse than no
cheat sheet.

This replaces the previous 18 individual tools with a composable interface. The LLM fetches the schema once, then constructs exactly the queries it needs — fetching multiple resources in a single round-trip, requesting only the fields it wants, and using typed arguments for filtering and pagination.

### Nested resolution

The object graph is fully navigable, so the LLM can get everything it needs in one hit:

```
Mailbox ──emails──▶ Email ──attachments──▶ Attachment ──base64/image/text──▶
   ▲ │                │ │
   │ └─parent/children┘ ├──thread──▶ Thread ──emails──▶ Email …
   └────mailboxes───────┘
```

Every collection is a Relay connection — `mailboxes`, `identities`,
`maskedEmails`, `contacts`, `attachments`, `Mailbox.children`, `Thread.emails` —
so each takes `first` / `last` / `after` / `before` and exposes `totalCount`,
`pageInfo`, `edges` and `nodes`. Cursors are IDs. Use `nodes` for the items;
`edges` only when you want per-item cursors. Default page 25, max 100 — check
`pageInfo.hasNextPage` rather than assuming you got the lot.

Only `emails` pages server-side, via JMAP's anchors. The rest arrive whole from
one call, so their paging is slicing: `totalCount` is free (a length, not
`calculateTotal`) and a cursor holds only as long as its item is still in the
list — if it goes, you get a "restart pagination" error rather than a quietly
different page. `Thread.emails` returns the same `EmailConnection` as the query
does, with `queryState` null because a conversation is not a query.

Value lists stay plain arrays: `from`, `to`, `cc`, `bcc`, `replyTo`, `sender`,
`keywords`, `mailboxIds`, `headers`, `Contact.emails`. They belong to the parent,
arrive with it, and paging them would be ceremony.

Attachment payloads are three separate fields, each doing the least work that
answers it. Metadata (`name`, `size`, `contentType`, `cid`, …) arrives with the
email and downloads nothing at all:

- `base64` — the raw bytes, base64-encoded. Download only, any type.
- `image(maxBytes:)` — resized then encoded, so a model isn't handed a 10MB
  photo. Null for non-images.
- `text` — extracted document text. **The expensive one**: it parses the whole
  file, is priced far above the other two, and nothing else on `Attachment`
  triggers it. Prefer a small page when selecting it.

```graphql
# Bodies and attachment text for a whole folder — one query, 3 API calls.
# Note the smaller page: `text` parses every document, so a big page means a
# lot of work. Nothing stops you asking for more; the cost is just yours.
{
  emails(filter: { inMailbox: "INBOX" }, first: 10) {
    nodes {
      subject
      from { name email }
      textBody
      attachments {
        nodes { name contentType size cid text }
      }
    }
  }
}

# Walk the graph: folder tree → emails → conversation → the folders they live in
{
  mailbox(name: "INBOX") {
    name
    children { nodes { name unreadEmails } }
    emails(first: 5) {
      nodes {
        subject
        thread { total emails { nodes { subject textBody } } }
        mailboxes { name role }
      }
    }
  }
}
```

### Sending attachments

`sendEmail`, `replyToEmail`, `forwardEmail` and `updateDraft` take
`attachments: [AttachmentInput!]`. The server usually runs somewhere other than
the client, so a local path means nothing to it; each attachment carries its
content instead, as exactly one of:

- `base64` — the file's bytes, inline.
- `blobId` — a blob already on the server, attached without re-uploading:
  one returned by `uploadAttachment`, or an existing email's
  `attachments { nodes { blobId } }`.

`name` is required; `contentType` is inferred from the name when omitted.

```graphql
mutation {
  sendEmail(
    action: PREVIEW
    to: "a@example.com"
    subject: "Report"
    body: "Attached."
    attachments: [{ name: "report.pdf", base64: "JVBERi0xLjQK..." }]
  ) { preview confirmationToken }
}
```

PREVIEW lists every attachment's name, type and size, and the confirmation
token covers them: a CONFIRM whose attachments differ in name, type or content
is rejected. Because CONFIRM repeats PREVIEW's arguments, inline base64 travels
twice; `uploadAttachment(name:, base64:)` sends it once and returns a `blobId`
to use in both. A blob given by ID is downloaded at PREVIEW to measure it, since
Fastmail does not offer JMAP's `Blob/get`.

Attachments are capped at 25 MB per message (Fastmail's own limit is 50 MB):
every inline byte passes through a tool argument, a third larger as base64.
`updateDraft`'s `attachments` replaces the draft's attachments; pass an
existing one's `blobId` to keep it. Inline images an HTML body refers to are
kept either way.

### Composable filters

`EmailFilter` mirrors JMAP's filter tree (RFC 8620 §5.5) rather than flattening
it into scalar arguments. Fields on one filter object are AND-ed; `and` / `or` /
`not` nest arbitrarily. The same input type is accepted everywhere emails
appear, including `Mailbox.emails`, where it's AND-ed with the mailbox.

```graphql
# Unread, from either sender, not in Archive, biggest first
{
  emails(
    filter: {
      unread: true
      or:  [{ from: "alice@example.com" }, { from: "bob@example.com" }]
      not: [{ inMailbox: "Archive" }]
    }
    sort: [{ property: SIZE, ascending: false }]
    first: 20
  ) {
    totalCount
    nodes { subject size }
  }
}
```

Mailbox names and roles are resolved to IDs at every depth of the tree in a
single `Mailbox/get`. `collapseThreads: true` returns one email per
conversation, for a threaded view.

The filter mirrors JMAP's `FilterCondition` (RFC 8621 §4.4.1) field for field:
alongside the usual participant/date/size conditions there are
`inMailboxOtherThan`, `hasKeyword` / `notKeyword`, the thread-wide
`allInThreadHaveKeyword` / `someInThreadHaveKeyword` / `noneInThreadHaveKeyword`,
and raw `header` matching (`["List-Id"]` for presence, `["List-Id", "rust-lang"]`
for a value).

Sorting likewise covers the full comparator set — `receivedAt`, `sentAt`, `size`,
`subject`, `from`, `to`, plus the keyword-based `hasKeyword`,
`allInThreadHaveKeyword` and `someInThreadHaveKeyword`, with `collation`. The
keyword comparators require a `keyword` argument and the others reject one;
both are caught before any API call.

### Pagination

Email lists are Relay connections with `edges`/`nodes`, `pageInfo`, `totalCount`,
`position`, and `queryState`.

**Cursors are email IDs**, mapped onto JMAP's `anchor` / `anchorOffset`. This
falls out of what the API already offers, and it's what makes pagination stable:
a positional cursor silently shifts every time mail arrives, so page 2 would
re-show or skip messages. An anchor names a specific message, so the page after
it is the same page whatever else changed. It stays legible for a model
composing the follow-up query too — the cursor is just an ID it has already seen.

```graphql
{
  emails(first: 25, after: "Mabc123") {
    totalCount
    pageInfo { hasNextPage endCursor }
    edges { cursor node { subject } }
  }
}
```

`last` without `before` becomes a negative `position`, which JMAP counts from
the end — "the last N" is one call and never needs a total. `last` with `before`
anchors backwards from the cursor.

The trade for stability is that a cursor can go stale: if its message is deleted
or stops matching the filter, JMAP returns `anchorNotFound`. That surfaces as an
error saying exactly that, and that the fix is to restart pagination. Compare
`queryState` between pages to detect that the result set moved underneath you.

`totalCount` maps to JMAP's `calculateTotal`, which costs the server real work,
so it's only requested when the field is actually selected. The same look-ahead
means a query selecting **only** `totalCount` performs one `Email/query` and
fetches no emails at all:

```graphql
# "How many unread from this sender?" — one call, zero emails transferred
{ emails(filter: { unread: true, from: "alerts@example.com" }) { totalCount } }
```

### Lazy fields and batching

Nothing below a list is fetched eagerly, and every lazy field goes through a
[DataLoader](https://github.com/graphql/dataloader): the resolvers for a list's
elements run concurrently, and their fetches collapse into **one batched API
call** rather than one per element.

| Selection on `emails(first: 25) { nodes { … } }` | JMAP calls |
| ----------------------------------------------- | ---------- |
| `{ totalCount }` alone | 1 (`Email/query`, no emails fetched) |
| `{ subject from { email } }` | 2 (`Email/query` + `Email/get`) |
| `{ subject textBody }` | 3 (+1 batched `Email/get` for all 25 bodies) |
| `{ subject textBody attachments { nodes { name } } }` | 3 (same batch covers both) |
| `{ … attachments { nodes { name cid size } } }` | 3 (metadata downloads nothing) |
| `{ … attachments { nodes { base64 } } }` | 3 + the blob downloads, issued concurrently |
| `{ … mailboxes { name } }` + `mailbox(…)` + a name filter | +1 `Mailbox/get` total, however many ask |

The naive shape — one detail call per email — would be 26. Loader caches are
per request, so referencing the same email or mailbox twice in one query costs
one fetch; nothing is retained between requests where it could go stale.

Because the graph contains cycles (`Email.thread.emails`, `Email.mailboxes.emails`),
nesting is capped at depth 15 — nothing else bounds a cycle. Breadth is **not**
capped. Resolvers declare a cost (a document parse prices far above a download,
nested lists scale with page size) but that cost is guidance for choosing a page
size, surfaced in the field descriptions; it never refuses a query. Being told
"too complex" without being told the threshold just makes a caller guess.

All operations are available as GraphQL queries and mutations: mailboxes, emails, search, threads, identities (with signatures), attachments (with text extraction and image resizing), contacts, masked email management, and send/reply/forward with the preview/confirm safety pattern.

One subscription, `emails`, streams arrivals over the same machinery as `fastmail watch`.

Token can be set via `FASTMAIL_API_TOKEN` env var or config file.

## Debug Logging

Enable debug output with `RUST_LOG`:

```bash
RUST_LOG=debug fastmail list mailboxes
```

## JMAP API

This CLI talks directly to Fastmail's JMAP server — the protocol layer is hand-rolled, there's no JMAP client library in the dependency tree. Capabilities are filtered dynamically based on your API token's permissions — read-only tokens work fine for listing/reading, while send and masked email operations require appropriate capabilities.

For more on JMAP: [jmap.io](https://jmap.io/)

## Verification

```bash
cargo test
cargo clippy --all-targets -- -D warnings
```

`tests/live_fastmail.rs` contains ignored tests for actual Fastmail behavior.
They are opt-in and use your configured credentials. Run the read-only check with:

```bash
cargo test --test live_fastmail live_session_and_mailbox_reads -- --ignored
```

With credentials for a test account, enable the temporary contact/photo/rich-note
and calendar/recurrence/free-busy/file round trips individually:

```bash
FASTMAIL_LIVE_WRITE_TESTS=1 cargo test --test live_fastmail live_contact_media_and_rich_notes -- --ignored
FASTMAIL_LIVE_WRITE_TESTS=1 cargo test --test live_fastmail live_dav_files_recurrence_and_freebusy -- --ignored
```

These create uniquely named resources and attempt cleanup. Scheduling sends a
REQUEST and CANCEL to an explicitly supplied test recipient, so it has additional
inputs:

```bash
FASTMAIL_LIVE_WRITE_TESTS=1 FASTMAIL_LIVE_SCHEDULING=1 \
FASTMAIL_LIVE_ORIGINATOR=mailto:you@fastmail.com \
FASTMAIL_LIVE_RECIPIENT=mailto:calendar-test@example.com \
cargo test --test live_fastmail live_itip_scheduling -- --ignored
```

## License

MIT
