# MailFabrica

English | [简体中文](README.zh-CN.md)

A desktop email client built with Go and Wails v3, with React, TypeScript and Vite on the frontend. The interface takes visual cues from Magpie and uses Foxmail-style compact three-pane mail navigation.

Features include multi-account IMAP/SMTP, incremental UID sync, offline mail, HTML isolation and inline images, rich-text composing in independent windows, attachments, drafts, contacts, subject grouping, invoice export, a mail calendar with lunar dates, macOS EventKit integration, optional AI writing assistance, light/dark themes and English/Chinese UI.

Desktop, CLI, MCP and the loopback control channel share one Go command engine. MailFabrica preserves the previous JSON/EML layout, message keys, folder hashes and system credential service name.

```sh
make deps
make demo                 # isolated demo mailbox; no real accounts or credentials
make check
make package              # macOS: dist/MailFabrica.app
./dist/mailfabrica-cli --demo list
./dist/mailfabrica-cli --demo mcp
```

Go builds and runtime do not require Rust or Cargo. `VERSION` is the build version. See the Chinese documentation for the [architecture](docs/architecture.md), [migration record](docs/go-wails-migration.md), [local builds](docs/local-build.md), [CLI](docs/cli.md), [MCP](docs/mcp.md) and [control protocol](docs/control-protocol.md).

Local macOS bundles use ad-hoc signatures for development. Official signing, notarization and publication are separate release operations. Cross-compilation is reported separately from native operating-system validation.

MIT OR Apache-2.0.
