# MailFabrica

English | [简体中文](README.zh-CN.md)

A desktop email client built with **Go and Wails v3**, with React, TypeScript and Vite on the frontend. MailFabrica takes visual cues from Magpie and uses Foxmail-style compact, three-pane navigation: accounts, message list and reading pane.

[Download](https://github.com/OpenFabrica/mailfabrica/releases/latest) · [Changelog](CHANGELOG.md) · [MCP setup](docs/mcp.md) · [Architecture](docs/architecture.md)

## Features

### Accounts and synchronization

- Multiple IMAP/SMTP accounts with TLS and STARTTLS.
- Provider presets for QQ, NetEase, Gmail, iCloud and other services, with editable hosts, ports and encryption settings.
- Custom-domain enterprise accounts, including NetEase enterprise client identification and backup-server handling.
- Per-account background synchronization, manual refresh and configurable polling intervals.
- Incremental UID synchronization, remote message flags, UIDVALIDITY checks and offline access to cached mail.
- Provider icons, full account addresses, folder trees and unread counts; create custom folders.

### Inbox and reading

- Unified inbox, unread, flagged and invoice views; local search, time ordering and date groups.
- Compact two-line rows, multiple selection, read/unread flags, follow-up flags, moving, deletion, junk actions and marking a folder as read.
- Subject-based conversation grouping, an overview and dedicated previous/next navigation within a group.
- HTML and plain-text reading, tables, inline CID images and attachment saving.
- MIME encoded-word decoding for subjects and address names, including existing cached messages.
- Fixed message headers and quick-reply controls with an independently scrolling body; menus dismiss on outside clicks.
- Validated HTTP, HTTPS and mailto links open through system applications, including static unsubscribe links.

### Composing and replies

- Independent compose windows, new messages, reply, reply-all, forwarding, draft saving and reopening.
- Multiple To/Cc/Bcc recipients, address chips, bulk pasting and contact autocomplete.
- Font sizes, bold, italic, underline, strikethrough, text colors, links, lists, quotations and format clearing.
- Attachments, account signatures, plain/rich-text switching and sent copies.
- Original messages appear as separate removable quotes, preserving HTML formatting and inline images.
- Unsaved-message protection when closing compose windows or quitting, with a guard while sending.

### Contacts and correspondence history

- Add, list and remove contacts.
- Prefix matching by name or email across saved contacts, accounts and cached correspondence, with weighted Trie ranking and address deduplication.
- Open correspondence history from the reading pane's **Mail history** button, sender/recipient names or the contact list.
- Search across all accounts by default, with an account filter, month groups, incoming/outgoing markers and source folders.
- Deduplicate copies by Message-ID, load additional pages, refresh, open the original message or write to the contact.

### AI assistance

- Anthropic and OpenAI-compatible endpoints, with configurable server URL, model and API key.
- Draft, polish, change tone, shorten, expand and translate message text.
- Generate quick-reply suggestions, then accept or discard them.
- Summarize a grouped conversation into a timeline, conclusions and action items, with source counts and visible errors.
- AI runs only on explicit user actions and does not automatically send mail. Relevant message text is sent to the configured service.

### Calendar and invoices

- A month view combining mail and events, with lunar dates, today navigation and month switching.
- Account-colored dots for mail; calendar-colored bars and backgrounds for events; hover details for titles, times and sources.
- Separate account/calendar filters and macOS EventKit integration for system events.
- Date-filtered invoice selection, attachment export and a UTF-8 CSV invoice list.

### Desktop experience and privacy

- English and Simplified Chinese, light/dark/system themes, and saved pane widths, collapsed accounts and preferences.
- Optional menu-bar/system-tray residency: close the main window while synchronization continues, then restore it from the icon. On macOS, closing also hides the Dock icon; reopening restores it.
- Inbox unread totals on the tray/menu-bar icon, hidden at zero, with open, sync, settings and quit actions.
- Script-disabled email iframes, active-content removal and a content security policy. Email JavaScript is not executed.
- A remote-image setting, isolated email content and bounded attachment/external-input sizes.
- macOS Keychain and Windows Credential Manager integration. Linux currently defaults to a permission-restricted local credentials file; this file is not encrypted storage.

### CLI, MCP and automation

- Desktop, CLI, MCP and the loopback JSON-line control channel share the same Go command engine, currently exposing 78 commands.
- CLI commands for accounts, synchronization, folders, listing, searching, reading, sending and direct command execution.
- stdio MCP tools for accounts, folders, mail search/read/send, status, command discovery and the general `mail_execute` entry point.
- Run MCP independently or connect it to the desktop session with `--control` to share state.
- Correspondence history, conversation summaries, AI, calendars and invoice export are also available through commands.

## Platforms and installation

Download the appropriate architecture from [Releases](https://github.com/OpenFabrica/mailfabrica/releases/latest). Each release includes `SHA256SUMS`.

| Platform | Architectures and package | Requirements and updates |
| --- | --- | --- |
| macOS | arm64 DMG; Developer ID signed and notarized | Built for macOS 13+. Drag into Applications. Installed Go versions support verified download and replacement after normal exit. |
| Windows | amd64 / arm64 ZIP; unsigned | Requires WebView2. Extract and run `mailfabrica.exe`; downloaded and verified updates are installed manually. |
| Linux | amd64 / arm64 tar.gz; unsigned | Release builds use GTK3 / WebKitGTK 4.1. Extract and run `mailfabrica`; replace files manually when updating. |

System-calendar access is currently macOS-only and requires permission. Linux tray visibility depends on the desktop environment's StatusNotifierItem support.

**For the first migration from 0.1.5 or earlier, quit the old client and install the new package manually, retaining the data directory.** The two generations use different update-helper handoff protocols. See [updates](docs/updates.md).

## CLI and MCP

On macOS, the CLI is inside `MailFabrica.app/Contents/MacOS/mailfabrica-cli`. Windows/Linux packages place it beside the desktop executable. These examples assume the CLI is on `PATH`:

```sh
mailfabrica-cli --demo list
mailfabrica-cli --demo commands --json
mailfabrica-cli --demo exec contacts.timeline '{"email":"sender4@example.com"}'
mailfabrica-cli --demo mcp
```

To control a desktop session, start it with `mailfabrica --control 7981` and configure your MCP client:

```json
{
  "mcpServers": {
    "mailfabrica": {
      "command": "/absolute/path/to/mailfabrica-cli",
      "args": ["mcp", "--control", "7981"]
    }
  }
}
```

Replace `command` with the installed path. When the desktop is using a data directory, connect through its control port instead of starting a second headless writer for that directory. See the [MCP guide](docs/mcp.md) for protocol details and tool parameters.

## Development and checks

Install Go as specified in `go.mod`, Node.js 22 and the platform's native build tools. macOS requires Xcode Command Line Tools. Linux release builds use `GOFLAGS=-tags=gtk3` with GTK3/WebKitGTK 4.1; the default build uses GTK4/WebKitGTK 6.

```sh
make deps
make demo                 # isolated demo; no real accounts or credentials
make check                # frontend build/format, go vet, tests and race checks
make build                # desktop executable and CLI
make package              # local package; macOS defaults to development ad-hoc signing
python3 scripts/smoke.py   # isolated mail send/read, invoice and calendar flow
```

For frontend hot reload:

```sh
./dist/mailfabrica --demo --web 7982
npm run dev --prefix frontend -- --port 5184
```

Open `http://127.0.0.1:5184`; the frontend proxy connects to the same Go engine. Native file dialogs, system calendars and desktop integration need Wails testing.

The source tree contains the Go + Wails implementation, with versions managed by `VERSION`. Native CI checks and isolated demos cover macOS, Windows amd64/arm64 and Linux amd64/arm64; these checks do not cover every OS version, mail provider or user desktop environment.

## Data compatibility and documentation

Account JSON, folder indexes, original `.eml` files, contacts, message keys and FNV-1a folder names retain the existing format. Relevant preferences from old `ui.json` files are read without replacing the original file. The macOS default directory is `~/Library/Application Support/MailFabrica`; use `--data` or `MAILFABRICA_HOME` to select another directory. Preserve the associated credential storage when backing up.

Most developer documentation is in Chinese:

- [Architecture](docs/architecture.md) and [migration](docs/go-wails-migration.md)
- [Account settings](docs/account-settings.md) and [NetEase enterprise mail](docs/enterprise-mail.md)
- [Reading](docs/mail-reading.md), [HTML content](docs/html-preview.md) and [conversations](docs/conversations.md)
- [Correspondence history](docs/contact-timeline.md), [invoice and calendar commands](docs/cli.md)
- [MCP](docs/mcp.md), [control protocol](docs/control-protocol.md) and [tray residency](docs/menu-bar.md)
- [Builds and signing](docs/local-build.md), [portable builds](docs/portable-release.md) and [updates](docs/updates.md)

## License

MIT OR Apache-2.0. Magpie is a UI reference; its gateways, plugins and telemetry are not included.
