#!/usr/bin/env python3
"""Tg Bridge test client ("demo bot").

Runs on any host without direct Telegram access (e.g. behind RU blocking).
Sends demo events to the admin chat through the bridge and answers admin's
messages via long-polling getUpdates.

Stdlib only, Python 3.8+.

Environment (usually an env file read by the systemd unit):
    TGB_URL       http://bridge-host:8080
    TGB_CLIENT    client name registered on the bridge (e.g. myapp)
    TGB_SECRET    shared HMAC secret for this client
    BOT_ALIAS     bot alias on the bridge (e.g. mybot)
    ADMIN_TG_ID   telegram user id allowed to talk to the bot
"""

import hashlib
import hmac
import json
import os
import signal
import sys
import time
import urllib.request

POLL_TIMEOUT = 25        # seconds, getUpdates long-poll
HTTP_TIMEOUT = POLL_TIMEOUT + 10
HEARTBEAT_SECS = int(os.environ.get("DEMO_HEARTBEAT_SECS", "3600"))


class BridgeClient:
    def __init__(self, base_url: str, client_name: str, secret: str):
        self.base_url = base_url.rstrip("/")
        self.client_name = client_name
        self.secret = secret.encode()

    def call(self, bot_alias: str, method: str, payload: dict, req_timeout: float = HTTP_TIMEOUT):
        body = json.dumps(payload, ensure_ascii=False).encode()
        ts = int(time.time())
        sig = hmac.new(
            self.secret,
            f"{ts}\n".encode() + body,
            hashlib.sha256,
        ).hexdigest()
        req = urllib.request.Request(
            f"{self.base_url}/v1/t/{bot_alias}/{method}",
            data=body,
            method="POST",
            headers={
                "Content-Type": "application/json",
                "X-TgB-Client": self.client_name,
                "X-TgB-Timestamp": str(ts),
                "X-TgB-Signature": sig,
            },
        )
        with urllib.request.urlopen(req, timeout=req_timeout) as resp:
            return json.load(resp)


def log(msg: str) -> None:
    print(time.strftime("%Y-%m-%d %H:%M:%S"), msg, flush=True)


def main() -> int:
    cfg = {k: os.environ.get(k, "") for k in
           ("TGB_URL", "TGB_CLIENT", "TGB_SECRET", "BOT_ALIAS", "ADMIN_TG_ID")}
    missing = [k for k, v in cfg.items() if not v]
    if missing:
        log(f"FATAL: missing env vars: {', '.join(missing)}")
        return 2

    admin_id = int(cfg["ADMIN_TG_ID"])
    br = BridgeClient(cfg["TGB_URL"], cfg["TGB_CLIENT"], cfg["TGB_SECRET"])
    alias = cfg["BOT_ALIAS"]

    running = {"flag": True}
    signal.signal(signal.SIGTERM, lambda *_: running.update(flag=False))

    me = br.call(alias, "getMe", {})
    if not me.get("ok"):
        log(f"FATAL: getMe failed: {me}")
        return 3
    bot_username = me["result"].get("username", "?")
    log(f"bridge ok, bot @{bot_username}")

    state = {"offset": 0, "events_sent": 0, "started": time.time()}

    def send(text: str) -> bool:
        ok = True
        for i in range(0, len(text), 4000):  # Telegram: 1-4096 chars per message
            chunk = text[i:i + 4000]
            try:
                r = br.call(alias, "sendMessage",
                            {"chat_id": admin_id, "text": chunk})
                state["events_sent"] += 1
                ok = ok and bool(r.get("ok"))
            except Exception as e:  # noqa: BLE001
                log(f"send failed: {e}")
                ok = False
        return ok

    def handle(text: str) -> str | None:
        cmd = text.strip().split(maxsplit=1)[0].lower() if text.strip() else ""
        if cmd == "/ping":
            return "pong"
        if cmd == "/status":
            up = int(time.time() - state["started"])
            return (f"uptime {up}s, events sent: {state['events_sent']}, "
                    f"bot: @{bot_username}")
        if cmd == "/help":
            return "commands: /ping /status /help; anything else gets echoed"
        return f"echo: {text}"

    send(f"demo bot started on this host, talking via bridge to @{bot_username}")

    next_heartbeat = time.time() + HEARTBEAT_SECS
    while running["flag"]:
        try:
            if time.time() >= next_heartbeat:
                send("heartbeat: alive")
                next_heartbeat = time.time() + HEARTBEAT_SECS

            r = br.call(alias, "getUpdates",
                        {"offset": state["offset"], "timeout": POLL_TIMEOUT,
                         "allowed_updates": ["message"]},
                        req_timeout=HTTP_TIMEOUT)
            if not r.get("ok"):
                log(f"getUpdates error: {r.get('description')}")
                time.sleep(5)
                continue

            for upd in r.get("result", []):
                state["offset"] = upd["update_id"] + 1
                msg = upd.get("message") or {}
                sender = (msg.get("from") or {}).get("id")
                text = msg.get("text", "")
                if not text:
                    continue
                if sender != admin_id:
                    log(f"ignored message from unauthorized user {sender}")
                    continue
                log(f"message from admin: {text!r}")
                reply = handle(text)
                if reply:
                    send(reply)
        except KeyboardInterrupt:
            break
        except Exception as e:  # noqa: BLE001
            log(f"loop error: {e}")
            time.sleep(5)

    send("demo bot stopped")
    return 0


if __name__ == "__main__":
    sys.exit(main())
