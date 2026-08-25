# Tg Bridge

[README in English](README.md)

[![CI](https://github.com/BrokVid/tg-bridge/actions/workflows/ci.yml/badge.svg)](https://github.com/BrokVid/tg-bridge/actions/workflows/ci.yml)
[![Release](https://github.com/BrokVid/tg-bridge/actions/workflows/release.yml/badge.svg)](https://github.com/BrokVid/tg-bridge/actions/workflows/release.yml)

Крошечный быстрый self-hosted релей между вашими серверами и Telegram Bot API.

Разворачивается на любом VPS с доступом к `api.telegram.org`. Остальные машины
(например, ноды за сетевыми блокировками) дергают мост по HTTP с
HMAC-подписанным запросом; мост пересылает вызов в Telegram и возвращает ответ
дословно. Токены ботов живут **только** на хосте моста — клиенты их не видят.

## Зачем

Типовой сценарий: приложение крутится там, где `api.telegram.org` недоступен
или ходить на него с каждой ноды нежелательно. Tg Bridge становится единственной
контролируемой точкой выхода в Telegram для всей фермы серверов:

- **Изоляция токенов** — компрометация клиента не раскрывает токен бота;
  ротация токена не требует правок на клиентах.
- **Per-client контроль доступа** — HMAC, окно времени, защита от replay,
  IP allowlist, белый список методов, rate limit.
- **Без состояния** — нет базы данных, безопасные рестарты, ~5–10 МБ RAM.
- **Уведомления и боты** — слать сообщения из cron, CI, мониторинга;
  интерактивные боты через long polling или webhook-релей.

## Возможности

- Прозрачный passthrough любого метода Bot API: JSON **и** multipart
  (отправка файлов до ~50 МБ)
- Именованные «действия»: клиент шлёт семантический JSON (`title`, `text`,
  `level`), мост сам собирает сконфигурированный вызов Telegram
- Webhook-релей: Telegram пушит апдейты на `/webhook/{alias}`, мост доставляет
  их клиенту с HMAC-подписью
- Защита от replay (in-memory nonce-кэш), constant-time сравнение подписей
- Метрики Prometheus на `/metrics` под той же аутентификацией
- Один статический бинарник Rust, конфиг TOML, секреты через env или файлы

## Быстрый старт

Установка одной командой на хосте моста (Ubuntu/Debian с systemd):

```bash
curl -fsSL https://raw.githubusercontent.com/BrokVid/tg-bridge/main/install.sh | sudo bash
```

Мастер задаст несколько вопросов (токен бота, адрес, имя клиента), сгенерирует
секреты, пропишет конфиг и hardened systemd-юнит, напечатает четыре значения
для клиентских машин.

Или собрать вручную:

```bash
cargo build --release
cp config/tg-bridge.example.toml /etc/tg-bridge/tg-bridge.toml
TGB_CONFIG=/etc/tg-bridge/tg-bridge.toml ./target/release/tg-bridge
```

Статические бинарники прикреплены к каждому
[релизу на GitHub](https://github.com/BrokVid/tg-bridge/releases)
(собираются GitHub Actions).

## Интеграция в проект

Drop-in модуль на Python: [examples/tgbridge_client.py](examples/tgbridge_client.py).
Работающий демо-бот: [examples/demo_bot.py](examples/demo_bot.py).

```python
from tgbridge_client import TgBridgeClient

tg = TgBridgeClient(tgb_url, tgb_client, tgb_secret)
tg.call(bot_alias, "sendMessage", {"chat_id": admin_id, "text": "deploy ok"})
```

Дайте ИИ-агенту [docs/INTEGRATION.md](docs/INTEGRATION.md) и четыре значения env
(`TGB_URL`, `TGB_CLIENT`, `TGB_SECRET`, `BOT_ALIAS`) — этого достаточно, чтобы
подключить уведомления к любому проекту.

## Документация

| Документ | Содержимое |
|---|---|
| [docs/PROTOCOL.md](docs/PROTOCOL.md) | Протокол v1: эндпоинты, заголовки, формат ошибок, лимиты, справочник конфига |
| [docs/DEPLOY.md](docs/DEPLOY.md) | Развёртывание на Ubuntu 24/26: systemd, Docker, TLS-фронт |
| [docs/INTEGRATION.md](docs/INTEGRATION.md) | Самодостаточный чеклист для человека и ИИ-агента по подключению моста |
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | Проектные решения (ADR), компоненты, модель безопасности |

## Статус

v1.0.0 — работает в проде. Протокол `/v1/` заморожен: несовместимые изменения
пойдут в `/v2/`, совместимость сохраняется минимум год.

## Лицензия

MIT — см. [LICENSE](LICENSE).
