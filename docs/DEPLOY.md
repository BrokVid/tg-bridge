# Деплой на Ubuntu 24.04 / 26.04

## 0. Где размещать

Мост должен иметь доступ к `api.telegram.org`. Рекомендуемая схема —
приватная сеть между мостом и клиентами (Tailscale/WireGuard): порт моста
не открывается в интернет вообще.

## 1. Сборка

Вариант А — собрать локально и скопировать бинарник (на сервере ничего
собирать не нужно):

```bash
cargo build --release --target x86_64-unknown-linux-musl
scp target/x86_64-unknown-linux-musl/release/tg-bridge bridge:/usr/local/bin/
```

Вариант Б — Docker (`deploy/Dockerfile`), образ ~15 МБ.

## 2. Конфигурация

```bash
sudo useradd --system --home /var/lib/tg-bridge --shell /usr/sbin/nologin tg-bridge
sudo mkdir -p /etc/tg-bridge /var/lib/tg-bridge
sudo cp config/tg-bridge.example.toml /etc/tg-bridge/tg-bridge.toml
sudo chown -R tg-bridge:tg-bridge /etc/tg-bridge /var/lib/tg-bridge
```

Секреты — в `/etc/tg-bridge/env` (права 0600, владелец tg-bridge):

```sh
TGB_CLIENT_SALUT66_SECRET=<openssl rand -hex 32>
TGB_BOT_SALUT_TOKEN=123456:ABC...
```

## 3. systemd

`deploy/systemd/tg-bridge.service.example` → `/etc/systemd/system/tg-bridge.service`, затем:

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now tg-bridge
curl -s http://127.0.0.1:8080/healthz
```

Смена конфига требует рестарта (мост stateless, даунтайм < секунды):
`sudo systemctl restart tg-bridge`.

## 4. Клиенты

На каждой машине-клиенте достаточно curl/Python из `examples/python_client.py`.
Клиенту нужны только: адрес моста, имя клиента и его секрет.

## 5. Обновление

Бинарник stateless: заменить файл, `systemctl restart tg-bridge`. Даунтайм —
меньше секунды; клиенты с ретраями потерь не заметят.
