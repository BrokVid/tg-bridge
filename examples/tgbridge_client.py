#!/usr/bin/env python3
"""TgBridgeClient — drop-in клиент моста Tg Bridge (stdlib only, Python 3.10+).

Скопируйте этот файл в свой проект. Инструкция по интеграции:
docs/INTEGRATION.md в репозитории tg-bridge.

Переменные окружения:
    TGB_URL      адрес моста, напр. http://31.x.x.x:8080
    TGB_CLIENT   имя клиента на мосте
    TGB_SECRET   HMAC-секрет клиента (64 hex)
    BOT_ALIAS    алиас бота, напр. salut

Проверка из командной строки:
    python3 tgbridge_client.py          # выполнит getMe и напечатает ответ
"""

import hashlib
import hmac
import json
import os
import time
import urllib.request

HTTP_TIMEOUT = 35.0  # >= long-poll timeout (25s) + запас


class TgBridgeError(RuntimeError):
    """Ошибка моста или Telegram (description содержит tgb: для ошибок моста)."""


class TgBridgeClient:
    def __init__(self, base_url: str, client_name: str, secret: str):
        self.base_url = base_url.rstrip("/")
        self.client_name = client_name
        self.secret = secret.encode()

    def _sign(self, timestamp: int, body: bytes) -> str:
        msg = f"{timestamp}\n".encode() + body
        return hmac.new(self.secret, msg, hashlib.sha256).hexdigest()

    def call_action(self, action: str, payload: dict,
                    req_timeout: float = 15.0) -> dict:
        """Вызывает именованное действие моста POST /v1/a/{action}."""
        body = json.dumps(payload, ensure_ascii=False).encode()
        ts = int(time.time())
        req = urllib.request.Request(
            f"{self.base_url}/v1/a/{action}",
            data=body,
            method="POST",
            headers={
                "Content-Type": "application/json",
                "X-TgB-Client": self.client_name,
                "X-TgB-Timestamp": str(ts),
                "X-TgB-Signature": self._sign(ts, body),
            },
        )
        try:
            with urllib.request.urlopen(req, timeout=req_timeout) as resp:
                return json.load(resp)
        except urllib.error.HTTPError as e:
            detail = e.read().decode(errors="replace")
            raise TgBridgeError(f"HTTP {e.code}: {detail}") from None

    @staticmethod
    def from_env() -> "TgBridgeClient":
        return TgBridgeClient(os.environ["TGB_URL"], os.environ["TGB_CLIENT"],
                              os.environ["TGB_SECRET"])


def notify(text: str, level: str = "info", project: str = "",
           admin_chat_id: int | None = None) -> bool:
    """Быстрая отправка уведомления админу; никогда не бросает исключений."""
    if admin_chat_id is None:
        admin_chat_id = int(os.environ.get("ADMIN_TG_ID", "0"))
    if not admin_chat_id:
        return False
    emoji = {"info": "\u2139\ufe0f", "warn": "\u26a0\ufe0f", "error": "\U0001f6a8"}.get(level, "")
    label = f" [{project}]" if project else ""
    try:
        c = TgBridgeClient(os.environ["TGB_URL"], os.environ["TGB_CLIENT"],
                           os.environ["TGB_SECRET"])
        r = c.call(os.environ.get("BOT_ALIAS", "salut"), "sendMessage",
                   {"chat_id": admin_chat_id, "text": f"{emoji}{label} {text}"})
        return bool(r.get("ok"))
    except Exception:  # noqa: BLE001 — уведомление не должно ронять проект
        return False


if __name__ == "__main__":
    client = TgBridgeClient(os.environ["TGB_URL"], os.environ["TGB_CLIENT"],
                            os.environ["TGB_SECRET"])
    print(client.call(os.environ["BOT_ALIAS"], "getMe", {}))
