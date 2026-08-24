# Tg Bridge

Крошечный быстрый self-hosted релей между вашими серверами и Telegram Bot API.

Разворачивается на любом VPS с доступом к `api.telegram.org`. Остальные машины
(например, ноды за национальными блокировками) дергают мост по HTTP с
HMAC-подписанным запросом; мост пересылает вызов в Telegram и возвращает ответ.
Токены ботов живут **только** на хосте моста — клиенты их не видят.

## Зачем

Типовой сценарий: проект крутится на VPS внутри России, ему нужен вход через
Telegram и уведомления админам, но `api.telegram.org` оттуда недоступен.
Tg Bridge ставится на незаблокированный VPS и становится единой точкой выхода
в Telegram для всей фермы серверов.

## Возможности

- Один статический бинарник Rust (~5–10 МБ RAM в простое, без рантайм-зависимостей)
- Прозрачный passthrough любого метода Bot API + опциональные именованные «действия»
- Per-client авторизация HMAC-SHA256 с меткой времени (защита от replay)
- Опционально: IP allowlist по клиенту, белый список методов, rate limit
- Без базы данных и состояния — безопасен для рестартов
- Конфиг в TOML; секреты через переменные окружения или файлы

## Быстрый старт

```bash
cargo build --release
cp config/tg-bridge.example.toml config/local/tg-bridge.toml
# правим конфиг, экспортируем секреты
TGB_CONFIG=config/local/tg-bridge.toml ./target/release/tg-bridge
```

Подробности: [docs/DEPLOY.md](docs/DEPLOY.md) (systemd/Docker на Ubuntu 24/26),
[docs/PROTOCOL.md](docs/PROTOCOL.md) (протокол),
[examples/python_client.py](examples/python_client.py) (готовый клиент).

## Интеграция в свой проект

Подключение моста к проекту сводится к одному файлу клиента и четырём
переменным окружения. Просто дайте эту задачу ИИ-агенту:

> Прочитай [docs/INTEGRATION.md](docs/INTEGRATION.md) и интегрируй Tg Bridge
> в этот проект: уведомления админу и обработка его сообщений. Значения
> TGB_URL / TGB_CLIENT / TGB_SECRET / BOT_ALIAS / ADMIN_TG_ID возьми из env
> (владелец моста их выдаёт).

Готовый drop-in модуль: [examples/tgbridge_client.py](examples/tgbridge_client.py).

## Статус

v0.1.0 — работает в проде на ферме автора: passthrough, действия, метрики,
rate limiting, интеграционные тесты. Roadmap: TLS-фронт, nonce-кэш,
multipart (файлы), webhook-режим.

## Лицензия

MIT — см. [LICENSE](LICENSE).
