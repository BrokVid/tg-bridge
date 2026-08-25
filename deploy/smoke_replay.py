"""Smoke test: replay protection on the live bridge.
Sends byte-identical getMe twice; second must be 401 tgb: replay detected.
Prints only outcomes, never secrets."""
import hashlib
import hmac
import json
import time
import urllib.error
import urllib.request

env = {}
with open("/etc/tg-demo/env") as f:
    for line in f:
        line = line.strip()
        if line and not line.startswith("#") and "=" in line:
            k, v = line.split("=", 1)
            env[k] = v

base = env["TGB_URL"].rstrip("/")
client = env["TGB_CLIENT"]
secret = env["TGB_SECRET"].encode()
alias = env.get("BOT_ALIAS", "mybot")
print("bridge:", base, "| client:", client, "| alias:", alias)


def call(body: bytes, ts: int):
    sig = hmac.new(secret, f"{ts}\n".encode() + body, hashlib.sha256).hexdigest()
    req = urllib.request.Request(
        f"{base}/v1/t/{alias}/getMe",
        data=body,
        method="POST",
        headers={
            "Content-Type": "application/json",
            "X-TgB-Client": client,
            "X-TgB-Timestamp": str(ts),
            "X-TgB-Signature": sig,
        },
    )
    try:
        with urllib.request.urlopen(req, timeout=15) as r:
            return r.status, json.load(r)
    except urllib.error.HTTPError as e:
        return e.code, json.loads(e.read().decode(errors="replace"))


body = b'{"smoke":"replay-test"}'
ts = int(time.time())

s1, j1 = call(body, ts)
print("1st identical request:", s1, "ok=", j1.get("ok"))

s2, j2 = call(body, ts)
desc = j2.get("description")
print("2nd identical request:", s2, desc)
assert s2 == 401 and desc == "tgb: replay detected", "REPLAY NOT DETECTED"

ts3 = int(time.time())
s3, j3 = call(b'{"smoke":"replay-test-fresh"}', ts3)
print("fresh-signed request:", s3, "ok=", j3.get("ok"))
assert s3 == 200 and j3.get("ok") is True

print("SMOKE OK")
