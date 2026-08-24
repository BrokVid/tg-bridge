# Tg Bridge

A tiny, fast, self-hosted relay between your own servers and the Telegram Bot API.

Deploy it on any VPS that can reach `api.telegram.org`. Your other machines
(e.g. nodes behind national-level blocking) call the bridge over HTTP with an
HMAC-signed request; the bridge forwards the call to Telegram and returns the
response verbatim. Bot tokens live **only** on the bridge host — clients never
see them.

## Why

Typical case: a project runs on a VPS inside Russia and needs Telegram login
plus admin notifications, but `api.telegram.org` is unreachable from there.
Tg Bridge runs on an unblocked VPS and becomes the single egress point to
Telegram for the whole server fleet.

## Features

- Single static Rust binary (~5–10 MB RAM idle, no runtime dependencies)
- Transparent passthrough of any Bot API method + named templated "actions"
- Per-client HMAC-SHA256 auth with timestamp (replay protection)
- Optional: per-client IP allowlist, method whitelist, rate limiting
- Prometheus metrics (`/metrics`, protected by the same signature)
- Stateless — no database, safe to restart anytime
- TOML config; secrets via environment variables or secret files

## Quick start

```bash
cargo build --release
cp config/tg-bridge.example.toml config/local/tg-bridge.toml
# edit config, export secrets
TGB_CONFIG=config/local/tg-bridge.toml ./target/release/tg-bridge
```

See [docs/DEPLOY.md](docs/DEPLOY.md) for systemd/Docker on Ubuntu 24/26,
[docs/PROTOCOL.md](docs/PROTOCOL.md) for the wire protocol, and
[examples/python_client.py](examples/python_client.py) for a ready-made client.

## Integrating into your project

Wiring a project to the bridge is one client file plus four env vars.
Hand it to your AI agent:

> Read [docs/INTEGRATION.md](docs/INTEGRATION.md) and integrate Tg Bridge
> into this project: admin notifications and handling admin messages.
> Take TGB_URL / TGB_CLIENT / TGB_SECRET / BOT_ALIAS / ADMIN_TG_ID from env
> (the bridge owner provides them).

Drop-in client module: [examples/tgbridge_client.py](examples/tgbridge_client.py).

## Status

v0.1.0 — running in production on the author's own fleet: passthrough,
actions, metrics, rate limiting, integration tests. Roadmap: TLS fronting,
nonce cache against in-window replay, multipart (files), webhook mode.

## License

MIT — see [LICENSE](LICENSE).
