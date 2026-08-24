# AGENTS.md — Tg Bridge

Серверный релей: принимает HMAC-подписанные HTTP-запросы с VPS фермы,
пересылает их в Telegram Bot API, возвращает ответы. Rust, Ubuntu 24/26.

## Контекст проекта
- Дизайн-документы: `docs/ARCHITECTURE.md` (ADR), `docs/PROTOCOL.md` (wire v1).
- Стек зафиксирован в `.agents/memory/decisions.md`.
- Целевая аудитория репо — публичная (GitHub): любой может захостить свой мост.

## Критично
- **Публично**: весь код, docs/, examples/, deploy/. **Приватно (gitignore)**:
  `config/local/`, `.env*`, `docs/private/`. Реальные токены и секреты в git
  не попадают никогда — даже в примерах.
- Токены ботов живут только на мосте; клиент обращается по алиасу.
- Подпись: `HMAC-SHA256(secret, "{timestamp}\n{raw_body}")`, сравнение constant-time.
- Протокол версионируется префиксом пути (`/v1/`).

## Стек (проверено 2026-08-24)
Rust stable 1.98 · tokio 1.53 · axum 0.8.9 · reqwest 0.13 (rustls) ·
hmac 0.13 + sha2 · serde/serde_json 1.x · toml 1.1 · tracing.
Не брать: actix-web (второй фреймворк незачем), sqlx/БД (stateless),
default-tls reqwest (только rustls — меньше зависимостей).

## Стиль
- Минимальные зависимости; каждый новый crate обосновать в ADR.
- Ошибки моста: префикс `tgb:` в description, формат из PROTOCOL.md.
- Логи: tracing, без тел запросов и секретов.
- Проверка готовности: `cargo build --release && cargo clippy && cargo test`.

## Команды
```bash
cargo build --release          # сборка
cargo clippy -- -D warnings    # линт
cargo test                     # тесты
```
