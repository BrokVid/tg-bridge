#!/usr/bin/env python3
"""Референсный клиент Tg Bridge. Только стандартная библиотека Python 3.

Использование:
    client = TgBridgeClient("http://bridge:8080", "salut66", secret)
    resp = client.call("salut", "sendMessage", {
        "chat_id": 123456789,
        "text": "deploy finished",
    })
    print(resp)
"""

import hashlib
import hmac
import json
import time
import urllib.request


class TgBridgeError(RuntimeError):
    pass


class TgBridgeClient:
    def __init__(self, base_url: str, client_name: str, secret: str):
        self.base_url = base_url.rstrip("/")
        self.client_name = client_name
        self.secret = secret.encode()

    def _sign(self, timestamp: int, body: bytes) -> str:
        msg = f"{timestamp}\n".encode() + body
        return hmac.new(self.secret, msg, hashlib.sha256).hexdigest()

    def call(self, bot_alias: str, method: str, payload: dict) -> dict:
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
        with urllib.request.urlopen(req, timeout=35) as resp:
            return json.load(resp)


if __name__ == "__main__":
    import os

    c = TgBridgeClient(
        os.environ["TGB_URL"],
        os.environ["TGB_CLIENT"],
        os.environ["TGB_SECRET"],
    )
    print(c.call(os.environ["TGB_BOT"], "getMe", {}))
