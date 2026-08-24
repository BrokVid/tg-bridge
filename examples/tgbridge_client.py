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
TEXT_LIMIT = 4096    # лимит Telegram на текст сообщения (после entities)


class TgBridgeError(RuntimeError):
    """Ошибка моста или Telegram (description содержит tgb: для ошибок моста)."""


def escape_html(text: str) -> str:
    """Экранирование для parse_mode="HTML". Обязательно для пользовательского
    ввода, иначе <, >, & ломают разметку (вектор инъекции entities)."""
    return text.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


def split_text(text: str, limit: int = TEXT_LIMIT):
    """Режет длинный текст на куски <= limit символов (по строкам, затем жёстко)."""
    if len(text) <= limit:
        return [text]
    chunks, cur = [], ""
    for line in text.splitlines(keepends=True):
        while len(line) > limit:
            if cur:
                chunks.append(cur)
                cur = ""
            chunks.append(line[:limit])
            line = line[limit:]
        if len(cur) + len(line) > limit:
            chunks.append(cur)
            cur = line
        else:
            cur += line
    if cur:
        chunks.append(cur)
    return chunks


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

    def call(self, bot_alias: str, method: str, payload: dict,
             req_timeout: float = HTTP_TIMEOUT) -> dict:
        body = json.dumps(payload, ensure_ascii=False).encode()
        ts = int(time.time())
        req = urllib.request.Request(
            f"{self.base_url}/v1/t/{bot_alias}/{method}",
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

    def send_message(self, chat_id, text: str,
                     max_retries: int = 3, **extra) -> bool:
        """Отправка с чанкингом длинных текстов и ретраями по retry_after.
        Возвращает message_id последнего куска или None при неудаче."""
        last_id = None
        for chunk in split_text(text):
            payload = {"chat_id": chat_id, "text": chunk, **extra}
            delivered = False
            for attempt in range(max_retries):
                try:
                    r = self.call(os.environ.get("BOT_ALIAS", "salut"),
                                  "sendMessage", payload)
                    if r.get("ok"):
                        last_id = r["result"]["message_id"]
                        delivered = True
                        break
                    params = (r.get("result") or {}).get("parameters") or {}
                    time.sleep(min(params.get("retry_after", 2 ** attempt), 30))
                except TgBridgeError as e:
                    if "429" not in str(e):
                        break
                    time.sleep(min(2 ** attempt, 30))
            if not delivered:
                return None
        return last_id

    def edit_message(self, chat_id, message_id: int, text: str, **extra) -> dict:
        """Правит отправленное сообщение (editMessageText). Лимит тот же:
        1-4096 символов; инлайн-сообщения правятся через inline_message_id."""
        return self.call(os.environ.get("BOT_ALIAS", "salut"), "editMessageText",
                         {"chat_id": chat_id, "message_id": message_id,
                          "text": text, **extra})

    def delete_message(self, chat_id, message_id: int) -> dict:
        """Удаляет сообщение (можно и чужое, если бот админ в группе)."""
        return self.call(os.environ.get("BOT_ALIAS", "salut"), "deleteMessage",
                         {"chat_id": chat_id, "message_id": message_id})


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
        c = TgBridgeClient.from_env()
        return c.send_message(admin_chat_id, f"{emoji}{label} {text}") is not None
    except Exception:  # noqa: BLE001 — уведомление не должно ронять проект
        return False


if __name__ == "__main__":
    client = TgBridgeClient(os.environ["TGB_URL"], os.environ["TGB_CLIENT"],
                            os.environ["TGB_SECRET"])
    print(client.call(os.environ["BOT_ALIAS"], "getMe", {}))
