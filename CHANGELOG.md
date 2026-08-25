# Changelog

Все заметные изменения проекта документируются в этом файле.
Формат основан на [Keep a Changelog](https://keepachangelog.com/ru/1.1.0/).

## [1.0.0] - 2026-08-25

Первый стабильный релиз. Протокол `/v1/` замораживается с этой версии:
несовместимые изменения пойдут в `/v2/`, совместимость `/v1/` сохраняется
минимум год.

### Added

- Passthrough `POST /v1/t/{alias}/{method}`: JSON и multipart/form-data
  (отправка файлов; лимит `max_upload_bytes`, по умолчанию ~50 МБ).
- Действия `POST /v1/a/{name}`: шаблоны `{{field}}` / `{{field|default}}`,
  нормализованный ответ `{ok, telegram_ok, result}`.
- Webhook-релей `POST /webhook/{alias}` (ADR-6): Telegram -> мост -> клиент,
  доставка подписана HMAC секретом клиента, без очередей (ретраит Telegram).
- Аутентификация: HMAC-SHA256 по `{timestamp}\n{raw_body}`, constant-time
  сравнение, окно timestamp ±60 c, per-client IP allowlist.
- Replay-защита: in-memory кэш `(client, signature)`, TTL = 2xокно + 5 c,
  включена по умолчанию (`[server] replay_protection = false` отключает).
- Per-client `allow_passthrough` (запрет сырого passthrough при разрешённых
  actions) и `methods_allowlist`.
- Лимиты: `max_body_bytes`, rate limit per client, лимиты длин Telegram
  (Bot API 10.2) для действий до отправки в апстрим.
- Метрики `GET /metrics` (Prometheus text, защищены HMAC).
- Клиентский модуль `examples/tgbridge_client.py`: notify, call_action,
  send_message с чанкингом и ретраями по retry_after.
- Интеграционные тесты против фейкового Telegram (44 теста суммарно),
  CI (build/clippy/test), примеры деплоя (systemd, Docker, caddy/nginx TLS).

## [0.1.0] - 2026-08-24

MVP: passthrough, действия, метрики, rate limit, интеграционные тесты.
Обкатан на собственной инфраструктуре автора.
