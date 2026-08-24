# Решения проекта (ADR-журнал)

Формат: дата, решение, почему. Публичная часть дублируется в docs/ARCHITECTURE.md;
здесь можно оставлять личные заметки (файл коммитится — секреты не писать).

## 2026-08-24

- **Rust + axum 0.8.9 + tokio 1.53**: минимальный CPU/RAM, один статический
  бинарник для Ubuntu 24/26. Альтернативы (Go/FastAPI) отвергнуты по RSS и
  профилю нагрузки pure-I/O proxy.
- **HTTP REST + HMAC-SHA256 вместо gRPC/WebSocket**: клиенты — обычные VPS с
  Python/curl; частота вызовов мала; простота отладки решает.
- **Bot tokens только на мосте**, клиент обращается по алиасу бота.
- **Два режима**: passthrough `/v1/t/{alias}/{method}` и именованные действия
  `/v1/a/{name}` из конфига.
- **Stateless**: без БД; rate limit in-memory; рестарт безопасен.
- **reqwest 0.13 c rustls** (не default-tls) — меньше системных зависимостей.
- **Публично/приватно**: весь репо публичен; `config/local/`, `.env*`,
  `docs/private/` в gitignore.

## Открытые вопросы

- Загрузка файлов в Bot API (multipart passthrough) — нужна ли на v1?
- Webhook-приёмник (Telegram -> мост -> клиент) как фаза 2?
- Nonce-кэш против replay внутри окна timestamp: делать сразу или по факту?
- TLS между клиентами и мостом: MVP ходит по открытому порту с HMAC +
  allowed_ips; TLS (caddy/nginx) добавить при выводе в прод.

## Реализовано (MVP, 2026-08-24)

- Проверено локально: cargo build/clippy/test зелёные; smoke-тест:
  unsigned -> 401, плохая подпись -> `tgb: bad signature`, подписанный
  запрос с фейковым токеном доходит до api.telegram.org и возвращает ответ
  дословно.
- reqwest 0.13: фича называется `rustls` (не `rustls-tls`).
- hmac 0.13: трейт `KeyInit` нужно импортировать явно.
- Клиенты в конфиге — массив `[[clients]]` с полем `name`.
- Тестовый клиент `examples/botik.py`: шлёт события, лонг-поллит getUpdates,
  отвечает только ADMIN_TG_ID (/ping /status /help, остальное echo).

## Задеплоено (MVP, 2026-08-24)

- **Мост на de-01** (31.77.168.37, Ubuntu 26.04): rustup stable 1.98 в
  `~/.cargo`, исходники `~/tgbridge-src`, бинарник `/opt/tg-bridge/tg-bridge`,
  конфиг `/opt/tg-bridge/tg-bridge.toml`, секреты `/opt/tg-bridge/env` (0600,
  owner tg-bridge). Юнит `/etc/systemd/system/tg-bridge.service`. Слушает
  0.0.0.0:8080. UFW: 8080/tcp только с 176.109.110.202 (ru-01).
  Swap: /swapfile 2G (без fstab, слетит после ребута — ок, нужен был для сборки).
- **Ботик на ru-01**: `/opt/tg-botik/botik.py`, env `/etc/tg-botik/env`,
  юнит tg-botik.service, user tg-botik. Лонг-поллит getUpdates (25s) через мост,
  отвечает только ADMIN_TG_ID (/ping /status /help, остальное echo),
  heartbeat раз в час.
- Клиент `salut66`: allowed_ips ru-01/32, методы getMe/sendMessage/getUpdates/
  answerCallbackQuery/setMyCommands.
- Проверено сквозняком: getMe/sendMessage/getUpdates 200 в логах de-01,
  сообщения админа доходят и обрабатываются.
- RAM моста ~5 МБ (de-01 free после запуска не изменился за пределами шума).

Где какие секреты: токен бота + HMAC-секрет — только /opt/tg-bridge/env на
de-01; HMAC-секрет + ADMIN_TG_ID — только /etc/tg-botik/env на ru-01.

## Интеграция в проекты (2026-08-24)

- `docs/INTEGRATION.md` — самодостаточная инструкция для ИИ-агента:
  чеклист env, правила подписи (байты = подписанные байты, UTC-секунды),
  паттерны notify/лонг-полла, обязательная проверка, раздел для агента
  на хосте моста (добавление клиента + SIGHUP reload).
- `examples/tgbridge_client.py` — drop-in модуль (stdlib, 3.10+), класс
  TgBridgeClient + функция notify(); проверен вживую с ru-01 против
  боевого моста.
- Сценарий использования: дать агенту проекта ссылку на INTEGRATION.md
  и 5 значений env — больше ничего не нужно.

## Релиз v0.1.0 (2026-08-24)

- Реализовано из ранее описанного: действия `[[actions]]` (глобальный массив,
  привязка к клиенту, шаблоны `{{field}}` / `{{field|default}}`, ответ
  `{ok, telegram_ok, result}`) и `/metrics` (Prometheus text, защищён HMAC,
  агрегаты, отключается `[metrics] enabled=false`).
- lib/bin сплит (`src/lib.rs` + тонкий main) ради интеграционных тестов:
  `tests/bridge_it.rs`, 11 тестов против фейкового Telegram-сервера
  (auth, rate limit, passthrough, actions, metrics). Итого 20 тестов.
- CI: `.github/workflows/ci.yml` (build --locked / clippy -D warnings / test).
- git init, коммит `977e5f0`, тег `v0.1.0`. Публичный GitHub — по готовности.
- de-01 передеплоен: новый бинарник + в боевом конфиге добавлено действие
  notify_admins (chat_id = админ). Проверено сквозняком с ru-01 через
  call_action: сообщение пришло в чат.
- В tgbridge_client.py добавлены call_action() и from_env().

Грабли деплоя: here-doc и вложенные кавычки через ssh/cmd не пробрасывать —
файлы скриптов заливать scp'ом и выполнять sudo bash. serde flatten +
toml для [[actions]] работает (toml 1.1).

## Хардненинг (2026-08-24)

- Сверено с Bot API 10.2 (14.07.2026): sendMessage/editMessageText 1–4096,
  caption 0–1024, answerCallbackQuery 0–200, setMyCommands ≤100,
  getUpdates limit 1–100 + offset=update_id+1 + несовместим с webhook.
- Мост: валидация сегментов пути ([A-Za-z0-9_-], ≤64) до авторизации;
  капа на длину подписи (128) перед hex-decode; abs_diff вместо вычитания
  (нет переполнения i64::MIN); лимиты длины Telegram для действий — 400
  до отправки; serde_json recursion limit против глубокой вложенности;
  предупреждение в лог при шаблонизации chat_id из клиентских полей.
- Клиенты: send_message() с чанкингом >4096 и ретраями по retry_after;
  escape_html() для parse_mode; botik шлёт кусками по 4000 и фильтрует
  allowed_updates=["message"].
- Тестов стало 29 (9 unit + 20 integration). Проверено вживую:
  get%2Fextra → 400 до auth; ботик перезапущен и работает.

## Следующий шаг

Каркас готов и задеплоен. Дальше по мере надобности: TLS-фронт, nonce-кэш,
multipart (файлы), webhook-режим, интеграция salut66.ru.
