# clipygo-plugin-msteams

[![Build](https://github.com/it-atelier-gn/clipygo-plugin-msteams/actions/workflows/ci.yml/badge.svg)](https://github.com/it-atelier-gn/clipygo-plugin-msteams/actions)
[![Rust](https://img.shields.io/badge/rust-1.80%2B-orange?logo=rust)](https://www.rust-lang.org/)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

A [clipygo](https://github.com/it-atelier-gn/clipygo) plugin that sends clipboard content to Microsoft Teams chats and channels. It fetches your recent conversations and team channels as targets, so you can route content with a single click.

## Prerequisites

An Azure AD app registration with these delegated Graph API permissions:

- `Team.ReadBasic.All`, `Channel.ReadBasic.All`, `ChannelMessage.Send`
- `Chat.ReadBasic`, `ChatMessage.Send`
- `offline_access`

Set the redirect URI to `http://localhost` (Web) for the OAuth2 flow.

## Authentication

| Method | Description |
|---|---|
| **OAuth2** (default) | Opens a browser for interactive login |
| **Device Code** | Enter a code at microsoft.com/devicelogin — no browser needed on the machine |
| **Password (ROPC)** | Username/password, requires tenant to allow ROPC |

All methods cache a refresh token so re-authentication is only needed when it expires.

## Configuration

Configure through clipygo's Settings → Plugins → Configure (includes step-by-step setup instructions).

Config file location:

| Platform | Path |
|---|---|
| Windows | `%APPDATA%\clipygo-plugin-msteams\config.json` |
| macOS | `~/Library/Application Support/clipygo-plugin-msteams/config.json` |
| Linux | `~/.config/clipygo-plugin-msteams\config.json` |

## Building

```sh
cargo build --release
```

## Installing

Either download a pre-built binary from [Releases](https://github.com/it-atelier-gn/clipygo-plugin-msteams/releases), or install directly from the plugin registry in clipygo's Settings.

To register manually: Settings → Plugins → add the path to the binary as the command.

## License

MIT © 2026 Georg Nelles
