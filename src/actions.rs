//! Named action templates: render `{{field}}` / `{{field|default}}`
//! placeholders in configured params from the client's JSON payload.

use serde_json::{Map, Value};

/// Renders every string value in `params`, substituting placeholders from
/// `payload`. Returns Err(list of missing field names) when a placeholder
/// has no payload value and no default.
pub fn render_params(
    params: &Value,
    payload: &Map<String, Value>,
) -> Result<Value, Vec<String>> {
    let mut missing = Vec::new();
    let rendered = render_value(params, payload, &mut missing);
    if missing.is_empty() {
        Ok(rendered)
    } else {
        missing.sort();
        missing.dedup();
        Err(missing)
    }
}

fn render_value(v: &Value, payload: &Map<String, Value>, missing: &mut Vec<String>) -> Value {
    match v {
        Value::String(s) => Value::String(render_str(s, payload, missing)),
        Value::Array(a) => Value::Array(
            a.iter()
                .map(|item| render_value(item, payload, missing))
                .collect(),
        ),
        Value::Object(o) => Value::Object(
            o.iter()
                .map(|(k, val)| (k.clone(), render_value(val, payload, missing)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn render_str(s: &str, payload: &Map<String, Value>, missing: &mut Vec<String>) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < s.len() {
        if bytes[i] == b'{' && i + 1 < s.len() && bytes[i + 1] == b'{' {
            if let Some(end) = s[i + 2..].find("}}") {
                let inner = &s[i + 2..i + 2 + end];
                out.push_str(&substitute(inner.trim(), payload, missing));
                i += 2 + end + 2;
                continue;
            }
        }
        // advance one full UTF-8 character
        let ch_len = utf8_len(bytes[i]);
        out.push_str(&s[i..i + ch_len]);
        i += ch_len;
    }
    out
}

fn utf8_len(b: u8) -> usize {
    match b {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    }
}

/// `field` or `field|default` -> payload value or default; unknown without
/// default records the field as missing.
fn substitute(expr: &str, payload: &Map<String, Value>, missing: &mut Vec<String>) -> String {
    let (field, default) = match expr.split_once('|') {
        Some((f, d)) => (f.trim(), Some(d.to_owned())),
        None => (expr, None),
    };
    match payload.get(field) {
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
        None => match default {
            Some(d) => d,
            None => {
                missing.push(field.to_owned());
                String::new()
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn map(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn substitutes_fields_and_defaults() {
        let params = json!({
            "chat_id": -100123,
            "text": "[{{level|info}}] {{title}}: {{text}}",
            "nested": {"s": "{{a}}", "n": 5}
        });
        let p = map(json!({"title": "deploy", "text": "ok", "a": "x"}));
        let r = render_params(&params, &p).unwrap();
        assert_eq!(r["text"], json!("[info] deploy: ok"));
        assert_eq!(r["chat_id"], json!(-100123));
        assert_eq!(r["nested"]["s"], json!("x"));
        assert_eq!(r["nested"]["n"], json!(5));
    }

    #[test]
    fn missing_field_reported() {
        let params = json!({"text": "{{title}} {{missing}}"});
        let p = map(json!({"title": "t"}));
        let err = render_params(&params, &p).unwrap_err();
        assert_eq!(err, vec!["missing".to_string()]);
    }

    #[test]
    fn numbers_render_as_json() {
        let params = json!({"text": "code {{code}}"});
        let p = map(json!({"code": 429}));
        let r = render_params(&params, &p).unwrap();
        assert_eq!(r["text"], json!("code 429"));
    }
}
