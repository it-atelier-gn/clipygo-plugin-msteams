# clipygo-plugin-msteams

A Microsoft Teams target provider plugin for [clipygo](https://github.com/it-atelier-gn/clipygo).

## What it does

This plugin lets you send clipboard content directly to Microsoft Teams chats and channels. It fetches your recent conversations and all team channels as targets, so you can route content to any of them with a single click. Authentication is handled via OAuth2 (browser login) or username/password (ROPC).

## Prerequisites

An Azure AD app registration with the following **delegated** Graph API permissions:

- `Team.ReadBasic.All` — list joined teams
- `Channel.ReadBasic.All` — list channels per team
- `ChannelMessage.Send` — post to channels
- `Chat.ReadBasic` — list recent chats
- `ChatMessage.Send` — post to chats
- `offline_access` — token refresh

Set the redirect URI to `http://localhost` (mobile and desktop applications) for the OAuth2 flow.

## Configuration

Configure the plugin through clipygo's Settings → Plugins → config UI, or edit the config file directly:

| Field | Required | Description |
|---|---|---|
| `tenant_id` | Yes | Azure AD tenant ID |
| `client_id` | Yes | App registration client ID |
| `auth_method` | No | `oauth2` (default) or `password` |
| `username` | ROPC only | UPN (user@company.com) |
| `password` | ROPC only | User password |

Config file location:

| Platform | Path |
|---|---|
| Windows | `%APPDATA%\clipygo-plugin-msteams\config.json` |
| macOS | `~/Library/Application Support/clipygo-plugin-msteams/config.json` |
| Linux | `~/.config/clipygo-plugin-msteams/config.json` |

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
