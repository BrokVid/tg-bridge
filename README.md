# Tg Bridge

[README на русском](README.ru.md)

[![CI](https://github.com/BrokVid/tg-bridge/actions/workflows/ci.yml/badge.svg)](https://github.com/BrokVid/tg-bridge/actions/workflows/ci.yml)
[![Release](https://github.com/BrokVid/tg-bridge/actions/workflows/release.yml/badge.svg)](https://github.com/BrokVid/tg-bridge/actions/workflows/release.yml)
![Crates.io style license](https://img.shields.io/badge/license-MIT-blue)

A tiny, fast self-hosted relay between your servers and the Telegram Bot API.

Deploy it on any VPS that can reach `api.telegram.org`. Your other machines
(for example, nodes behind network restrictions) call the bridge over HTTP
with an HMAC-signed request; the bridge forwards the call to Telegram and
returns the response verbatim. Bot tokens live **only** on the bridge host —
clients never see them.

## Why

Typical setup: your application runs on a server where `api.telegram.org` is
unreachable or undesirable to hit directly from every node. Tg Bridge becomes
the single controlled exit point to Telegram for your whole fleet:

- **Token isolation** — a compromised client never leaks the bot token;
  rotating a token requires no changes on clients.
- **Per-client access control** — HMAC authentication, timestamp window,
  replay protection, IP allowlists, method allowlists, rate limiting.
- **Stateless** — no database, safe restarts, ~5–10 MB of RAM.
- **Notifications and bots** — send messages from cron jobs, CI, monitoring
  or run interactive bots via long polling or the webhook relay.

## Features

- Transparent passthrough of any Bot API method (JSON **and** multipart file
  uploads up to ~50 MB)
- Named *actions*: clients post semantic JSON (`title`, `text`, `level`),
  the bridge renders preconfigured Telegram calls
- Webhook relay: Telegram pushes updates to `/webhook/{alias}`, the bridge
  delivers them HMAC-signed to your client endpoint
- Replay protection (in-memory nonce cache), constant-time signature checks
- Prometheus metrics at `/metrics`, protected by the same auth
- Single static Rust binary, TOML config, secrets via env vars or files

## Quick start

One-line install on the bridge host (Ubuntu/Debian with systemd):

```bash
curl -fsSL https://raw.githubusercontent.com/BrokVid/tg-bridge/main/install.sh | sudo bash
```

The wizard asks a few questions (bot token, listen address, client name),
generates secrets, writes the config, installs a hardened systemd unit and
prints the four values your client machines need.

Or build manually:

```bash
cargo build --release
cp config/tg-bridge.example.toml /etc/tg-bridge/tg-bridge.toml
TGB_CONFIG=/etc/tg-bridge/tg-bridge.toml ./target/release/tg-bridge
```

Prebuilt static binaries are attached to every
[GitHub release](https://github.com/BrokVid/tg-bridge/releases)
(built by GitHub Actions).

## Client integration

Drop-in Python module: [examples/tgbridge_client.py](examples/tgbridge_client.py).
A working demo bot: [examples/demo_bot.py](examples/demo_bot.py).

```python
from tgbridge_client import TgBridgeClient

tg = TgBridgeClient(tgb_url, tgb_client, tgb_secret)
tg.call(bot_alias, "sendMessage", {"chat_id": admin_id, "text": "deploy ok"})
```

Hand [docs/INTEGRATION.md](docs/INTEGRATION.md) to an AI agent together with
four env values (`TGB_URL`, `TGB_CLIENT`, `TGB_SECRET`, `BOT_ALIAS`) — that's
all it needs to wire notifications into any project.

## Documentation

| Document | Contents |
|---|---|
| [docs/PROTOCOL.md](docs/PROTOCOL.md) | Wire protocol v1: endpoints, auth headers, error format, limits, config reference |
| [docs/DEPLOY.md](docs/DEPLOY.md) | Deployment on Ubuntu 24/26: systemd, Docker, TLS fronting |
| [docs/INTEGRATION.md](docs/INTEGRATION.md) | Self-contained checklist for humans and AI agents to integrate the bridge into a project |
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | Design decisions (ADRs), components, security model |

## Status

v1.0.0 — running in production. Protocol `/v1/` is frozen: breaking changes
will go to `/v2/`, compatibility kept for at least a year.

## License

MIT — see [LICENSE](LICENSE).
