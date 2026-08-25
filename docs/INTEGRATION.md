# Интеграция Tg Bridge в проект (инструкция для ИИ-агента)

Ты — агент (Claude Code, Codex, opencode или другой), которого попросили
добавить в проект Telegram-уведомления и/или авторизацию через Telegram,
когда хост проекта не имеет прямого доступа к `api.telegram.org`.
Проект общается с Telegram **только** через мост Tg Bridge по протоколу
ниже. Этот документ самодостаточен: внешние источники не нужны.

Полная спецификация протокола: [PROTOCOL.md](PROTOCOL.md).

---

## Чеклист перед началом

Запроси у владельца моста (или найди в секретах проекта) четыре значения:

| Переменная | Пример | Что это |
|---|---|---|
| `TGB_URL` | `http://31.x.x.x:8080` | адрес моста |
| `TGB_CLIENT` | `myapp` | имя клиента, регистрируется на мосте |
| `TGB_SECRET` | 64 hex-символа | общий HMAC-секрет этого клиента |
| `BOT_ALIAS` | `mybot` | алиас бота на мосте |

Если какого-то значения нет — остановись и спроси у пользователя.
**Не выдумывай секреты и не храни их в коде/репозитории** — только env.

Дополнительно для уведомлений: `ADMIN_TG_ID` (числовой id получателя).
Узнать его можно, отправив боту любое сообщение и прочитав
`result[].message.from.id` из ответа `getUpdates`.

## Шаг 1. Клиентский модуль

Скопируй [`examples/tgbridge_client.py`](../examples/tgbridge_client.py)
в проект (один файл, только stdlib) и оберни в конфиг проекта:

```python
from tgbridge_client import TgBridgeClient

tg = TgBridgeClient(os.environ["TGB_URL"], os.environ["TGB_CLIENT"],
                    os.environ["TGB_SECRET"])
r = tg.call(os.environ["BOT_ALIAS"], "sendMessage",
            {"chat_id": admin_id, "text": "deploy ok"})
```

Не на Python — реализуй по спецификации ниже; она умещается в 30 строк
на любом языке.

## Шаг 2. Как устроен запрос (если пишешь сам)

```
POST {TGB_URL}/v1/t/{BOT_ALIAS}/{method}
Content-Type: application/json
X-TgB-Client:    {TGB_CLIENT}
X-TgB-Timestamp: {unix_time_seconds}
X-TgB-Signature: {hex(HMAC-SHA256(TGB_SECRET, "{timestamp}\n{raw_body}"))}

{raw_body} = тело запроса, отправляй РОВНО те байты, которые подписал
```

Критичные правила (нарушение = 401):

1. Подписывай те же байты, что отправляешь. Сериализуй JSON один раз:
   `body = json.dumps(payload).encode()`, потом `sign(ts, body)` и `send(body)`.
2. Timestamp — UTC в секундах (`int(time.time())`). Окно моста ±60 c:
   если ловишь `tgb: timestamp out of window` — проверь NTP на хосте.
3. Секрет не логировать, не коммитить, не подставлять в URL.

Ответ моста = дословный ответ Telegram Bot API (тот же HTTP-статус и JSON).
Ошибки самого моста имеют префикс `tgb:` в поле `description`.

## Шаг 3. Отправка уведомлений

```python
def notify(text: str, level: str = "info") -> bool:
    emoji = {"info": "ℹ️", "warn": "⚠️", "error": "🚨"}.get(level, "")
    try:
        return tg.call(alias, "sendMessage",
                       {"chat_id": admin_id, "text": f"{emoji} [{project}] {text}"}
                       ).get("ok", False)
    except Exception:
        logger.warning("telegram notify failed", exc_info=True)
        return False   # уведомление никогда не должно ронять основной процесс
```

Правила:

- Уведомления — fire-and-forget с коротким таймаутом (10–15 c), вне
  критического пути. Упало — залогировать и жить дальше.
- Не спамить: при массовых событиях агрегируй в одно сообщение.
- 429 от моста (`tgb: rate limited`) или от Telegram (поле
  `result.parameters.retry_after`) — повторить через указанное число секунд,
  максимум 2–3 попытки. `send_message()` из модуля делает это сама, плюс
  режет тексты длиннее 4096 символов на части (лимит Telegram).
- Если используешь `parse_mode: "HTML"` или `"MarkdownV2"` — пользовательский
  ввод ОБЯЗАТЕЛЬНО прогоняй через `escape_html()` / экранирование разметки,
  иначе это инъекция entities (подделка ссылок/форматирования в сообщении).
  Для логов и уведомлений проще не задавать parse_mode вовсе.

## Шаг 4. Приём сообщений (интерактивный бот)

По умолчанию используй лонг-поллинг `getUpdates` в отдельном
потоке/процессе/юните (вебхук-релей тоже есть — см. PROTOCOL.md,
«POST /webhook/{alias}», но он требует HTTPS-endpoint на стороне клиента):

```python
offset = 0
while True:
    r = tg.call(alias, "getUpdates",
                {"offset": offset, "timeout": 25}, req_timeout=35)
    for upd in r.get("result", []):
        offset = upd["update_id"] + 1          # обязательно до обработки
        msg = upd.get("message") or {}
        if (msg.get("from") or {}).get("id") != int(ADMIN_TG_ID):
            continue                            # чужих молча игнорируем
        text = msg.get("text", "")
        # ... твоя обработка ...
```

- Таймаут HTTP >= 35 c при `timeout=25` в payload.
- `offset` сохраняй (хотя бы в память), иначе сообщения будут приходить заново.
- Авторизация получателя — сравнение `from.id`; id отрицательный для групп.

## Шаг 5. Проверка (обязательна)

```bash
# 1. мост доступен
curl -s $TGB_URL/healthz            # {"ok":true,...}

# 2. подписанный вызов проходит (запусти модуль как скрипт)
python3 tgbridge_client.py          # печатает ответ getMe

# 3. сообщение реально доставлено
# отправь тестовое уведомление и проверь, что оно пришло в чат
```

Если п.2 даёт `401 unknown client` / `bad signature` — перепроверь
`TGB_CLIENT`/`TGB_SECRET` у владельца моста. `403 method not allowed` —
попроси добавить метод в `methods_allowlist`.

---

## Для агента на стороне моста (добавление нового клиента)

Работаешь на хосте моста и нужно зарегистрировать новый проект?

1. Сгенерируй секрет: `openssl rand -hex 32`.
2. В `/opt/tg-bridge/tg-bridge.toml` добавь:

   ```toml
   [[clients]]
   name = "<имя-проекта>"
   secret = "env:TGB_CLIENT_<ИМЯ>_SECRET"
   allowed_ips = ["<IP клиента>/32"]      # опционально, но рекомендуется
   bots = ["mybot"]
   methods_allowlist = ["getMe", "sendMessage", "getUpdates"]
   ```

3. В `/opt/tg-bridge/env` добавь строку `TGB_CLIENT_<ИМЯ>_SECRET=<секрет>`.
4. `sudo systemctl restart tg-bridge` — мост stateless, даунтайм < секунды.
5. Передай клиенту `TGB_URL`, `TGB_CLIENT`, `TGB_SECRET`, `BOT_ALIAS`
   по защищённому каналу — не в общем чате и не в git.

---

## Группы и чаты

Бот работает с любыми чатами — личка, группа, супергруппа, канал:

1. **Добавь бота в группу** (ссылка-приглашение, права не обязательны).
2. **Узнай chat_id группы**: напиши любое сообщение в группе, вызови
   `getUpdates` через клиент и возьми `result[].message.chat.id`
   (у групп он отрицательный, у супергрупп/каналов вида `-100…`).
3. Дальше всё то же самое: `send_message(-100123456789, "...")`.

Нюансы из доков Bot API:

- **Privacy mode**: по умолчанию бот в группах видит только команды
  (`/cmd@botname`), ответы на свои сообщения и упоминания. Чтобы получать
  все сообщения группы — отключи privacy mode у @BotFather
  (`/setprivacy` → Disable) или сделай бота админом группы.
- `deleteMessage` чужих сообщений и закрепление требуют прав админа у бота.
- В группах действует антифлуд Telegram (медленный режим) — уважай
  `retry_after`; `send_message()` делает это автоматически.

## Редактирование отправленных сообщений

Любой метод Bot API проходит через мост, включая редактирование:

```python
mid = c.send_message(chat_id, "статус: выполняется")     # -> message_id
c.edit_message(chat_id, mid, "статус: готово ✅")
c.delete_message(chat_id, mid)                            # если не нужно хранить
```

Типовой паттерн для долгих задач — одно сообщение-«статус», которое бот
обновляет по ходу работы вместо спама новыми сообщениями.
Ограничения Telegram: текст после правки ≤4096 символов; сообщение можно
править в течение 48 часов; медиа правится через `editMessageCaption`.
