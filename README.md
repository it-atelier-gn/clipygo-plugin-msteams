# clipygo-plugin-msteams

[![Build](https://github.com/it-atelier-gn/clipygo-plugin-msteams/actions/workflows/ci.yml/badge.svg)](https://github.com/it-atelier-gn/clipygo-plugin-msteams/actions)
[![Rust](https://img.shields.io/badge/rust-1.80%2B-orange?logo=rust)](https://www.rust-lang.org/)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

A Microsoft Teams target provider plugin for [clipygo](https://github.com/it-atelier-gn/clipygo).

## What it does

This plugin lets you send clipboard content directly to Microsoft Teams chats and channels. It fetches your recent conversations and all team channels as targets, so you can route content to any of them with a single click.

## Authentication

Three authentication methods are available:

| Method | Description |
|---|---|
| **OAuth2** (default) | Opens a browser for interactive login. Recommended for desktop use. |
| **Device Code** | Displays a code to enter at [microsoft.com/devicelogin](https://microsoft.com/devicelogin). No browser needed on the machine running the plugin. |
| **Password (ROPC)** | Username/password, no browser needed. Requires the tenant to allow ROPC. |

All methods cache a refresh token so re-authentication is only needed when the token expires or credentials change.

## Prerequisites

An Azure AD app registration with the following **delegated** Graph API permissions:

- `Team.ReadBasic.All` — list joined teams
- `Channel.ReadBasic.All` — list channels per team
- `ChannelMessage.Send` — post to channels
- `Chat.ReadBasic` — list recent chats
- `ChatMessage.Send` — post to chats
- `offline_access` — token refresh

Set the redirect URI to `http://localhost` (Web) for the OAuth2 flow.

## Configuration

Configure the plugin through clipygo's Settings → Plugins → ⚙ config UI (which includes step-by-step setup instructions), or edit the config file directly:

| Field | Required | Description |
|---|---|---|
| `tenant_id` | Yes | Azure AD tenant ID |
| `client_id` | Yes | App registration client ID |
| `auth_method` | No | `oauth2` (default), `device_code`, or `password` |
| `username` | Password only | UPN (user@company.com) |
| `password` | Password only | User password |

The username/password fields are only shown in the config UI when password auth is selected.

Config file location:

| Platform | Path |
|---|---|
| Windows | `%APPDATA%\clipygo-plugin-msteams\config.json` |
| macOS | `~/Library/Application Support/clipygo-plugin-msteams/config.json` |
| Linux | `~/.config/clipygo-plugin-msteams/config.json` |

## Project structure

```
src/
├── main.rs       # Entry point — stdin/stdout JSON protocol loop
├── protocol.rs   # Request/response types
├── config.rs     # Config struct, load/save
├── auth.rs       # Token management, OAuth2/device code/password flows
├── graph.rs      # Graph API types, chat/channel fetching, message posting
└── handler.rs    # Command dispatch + tests
```

## Building

```sh
cargo build --release
```

The binary is at `target/release/clipygo-plugin-msteams` (or `.exe` on Windows).

## Releases

Pre-built binaries for Windows, Linux, and macOS are published automatically via GitHub Actions on every `v*` tag.

| Platform | Artifact |
|---|---|
| Windows x64 | `clipygo-plugin-msteams-windows-x64.exe` |
| Linux x64 | `clipygo-plugin-msteams-linux-x64` |
| macOS ARM64 | `clipygo-plugin-msteams-macos-arm64` |

SHA256 checksums are published alongside each binary.

## Registering in clipygo

In clipygo Settings → Plugins, add the path to the downloaded binary as the command. Or install it directly from the [clipygo plugin registry](https://github.com/it-atelier-gn/clipygo-plugins).
