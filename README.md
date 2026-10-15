# MailFabrica

English | [简体中文](README.zh-CN.md)

A native desktop mail client written in Rust. The interface and workflow follow Foxmail; the engineering approach follows GridFabrica (layered crates, an egui UI, everything is a command, a control channel, MCP).

## Screenshots

The reading pane shows HTML mail as designed, close to what a browser renders. Messages with the same subject collapse into one row in the list:

![Main window: an HTML message and the list grouped by subject](docs/screenshots/main.png)

Open a grouped row to see every message with that subject:

![Subject group: all messages with the same subject](docs/screenshots/group.png)

Replies to HTML mail quote the original with its formatting intact, in the same quote format Gmail uses:

![Reply: the original message quoted with its formatting](docs/screenshots/reply.png)

The screenshots come from demo mode (`--demo`); all accounts and messages in them are made up. They show the Chinese UI; English is also available.

## Features
- **Three-pane layout**: account/folder tree (with unread counts) · message list grouped into Today / Yesterday / This week / This month / Older · reading pane with quick reply.
- **Toolbar**: get mail, write, reply, reply all, forward, delete, mark (read / unread / flagged), move to, junk, search.
- **Unified views**: all inboxes, unread, flagged (across accounts).
- **Compose**: multiple windows, recipient autocomplete (address book), attachments, save draft / continue from a draft, signatures, quoted replies, forwarding with the original attachments.
- **Accounts**: server settings filled in from the address (QQ/Foxmail, 163, 126, yeah, Sina, Aliyun, 139, Gmail, Outlook, iCloud, Yahoo, Zoho) and editable by hand; passwords are kept in the system keychain.
- **Protocols**: IMAP4rev1 (incremental UID sync, MOVE/UIDPLUS, Chinese folder names, the NetEase ID command) + SMTP (465 TLS / 587 STARTTLS, AUTH PLAIN/LOGIN).
- **Local storage**: one directory per account with a JSON index, the original `.eml` files and the address book; readable offline.
- **HTML mail**: laid out with blitz and rendered on the CPU, so CSS layout, buttons and images look as designed; remote images are loaded only from public internet addresses.
- **Group by subject**: messages with the same subject (ignoring prefixes like `Re:`, `Fwd:`, `回复：`) share one row; turn it off from the list menu or in Settings.
- **AI writing assistant**: in a compose window, write a draft or rewrite your text (polish, more formal, friendlier, shorter, longer, translate). The result is a suggestion you accept (replace or append) or dismiss; only your own text is changed, never the signature or the quoted original. On replies and forwards the original message is given to the AI as context.
- **AI quick reply**: drafts a short reply to the open message and fills the quick-reply box under the reading pane.
- **One-click invoice export**: the "Invoices" virtual folder in the sidebar lists invoice emails from all accounts. Export saves the invoice attachments (pdf/ofd/xml/zip) as `date_sender_originalname` and writes `invoices.csv` (date, sender, subject, amount, files, download links found in the body).
- **Chinese and English UI, light and dark themes**.
- **Scriptable**: every action is a command, callable from the control channel, the command line and MCP.

## Running
```sh
# Demo mode (in-memory fake server, no real account needed)
cargo run --release -p mailfabrica -- --demo

# Normal use (data lives in ~/Library/Application Support/MailFabrica; change it with --data or MAILFABRICA_HOME)
cargo run --release -p mailfabrica

# Command line
cargo run -p mailfabrica-cli -- --demo list
cargo run -p mailfabrica-cli -- account add me@qq.com --password-stdin
cargo run -p mailfabrica-cli -- sync && cargo run -p mailfabrica-cli -- list --unread
```
For QQ, 163, 126 and similar providers, first turn on IMAP/SMTP in the webmail settings and use the **authorization code** instead of your login password. For Gmail, use an app password.

## AI assistant setup
The AI features are off until you configure a service in **Settings**:
- **Anthropic** (default): URL `https://api.anthropic.com`, model `claude-opus-5-5`. Only the API key is needed.
- **OpenAI-compatible**: any Chat Completions endpoint (DeepSeek, Qwen/DashScope, Ollama, ...). URL and model are required.

The API key is kept in the system keychain (the same place as mail passwords) and never written to `prefs.json`. The URL must be `https`; plain `http` is allowed only for localhost (for example Ollama). Email content is sent to the configured AI service only when you click an AI action.

Command line: `docs/cli.md`. MCP: `docs/mcp.md`. Control channel: `docs/control-protocol.md`. (These documents are in Chinese.)

## Development
- Architecture and conventions: `AGENTS.md`; releases: `docs/release.md`.
- Local macOS app and DMG: `cargo xtask package` (see `docs/local-build.md`); logo preview: `cargo xtask icon`.
- Before committing: `cargo xtask ci` (fmt, clippy -D warnings, tests, layer check).

## Acknowledgements
Built with the help of [Claude Code](https://www.anthropic.com/claude-code) (Claude by Anthropic).

## License
MIT OR Apache-2.0.

- Updates: use **Check for Updates** in the toolbar menu; download, verify and open the macOS installer. See [help](docs/update-help.md) and [developer guide](docs/updates.md).

- Windows amd64/arm64 and Linux amd64/arm64 portable packages are built and tested natively; see [cross-platform releases](docs/portable-release.md).
