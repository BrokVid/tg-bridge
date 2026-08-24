use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::Deserialize;

#[derive(Debug, Clone)]
pub struct Config {
    pub server: Server,
    pub telegram: Telegram,
    pub rate_limit: RateLimit,
    pub metrics: Metrics,
    pub bots: HashMap<String, String>,
    pub clients: HashMap<String, Client>,
    pub actions: HashMap<String, Action>,
}

#[derive(Debug, Clone)]
pub struct Metrics {
    pub enabled: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct MetricsToml {
    #[serde(default = "d_true")]
    enabled: bool,
}

fn d_true() -> bool {
    true
}

/// A named action: client POSTs semantic JSON to /v1/a/{name}; the bridge
/// renders `params` (with {{field}} placeholders) and calls the Bot API.
#[derive(Debug, Clone, Deserialize)]
pub struct Action {
    /// client name allowed to call this action
    pub client: String,
    /// bot alias on the bridge
    pub bot: String,
    /// Bot API method, e.g. sendMessage
    pub method: String,
    /// parameter template; strings may contain {{field}} / {{field|default}}
    #[serde(default)]
    pub params: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize)]
struct ServerToml {
    listen: String,
    #[serde(default = "d_max_body")]
    max_body_bytes: usize,
    #[serde(default = "d_req_timeout")]
    request_timeout_secs: u64,
    #[serde(default = "d_ts_window")]
    timestamp_window_secs: i64,
}

fn d_max_body() -> usize {
    65536
}
fn d_req_timeout() -> u64 {
    35
}
fn d_ts_window() -> i64 {
    60
}

#[derive(Debug, Clone, Deserialize)]
struct TelegramToml {
    api_base: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct RateLimitToml {
    requests_per_minute: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
struct BotToml {
    token: SecretRef,
}

#[derive(Debug, Clone, Deserialize)]
struct ClientToml {
    name: String,
    secret: SecretRef,
    #[serde(default)]
    allowed_ips: Vec<String>,
    #[serde(default)]
    bots: Vec<String>,
    #[serde(default)]
    methods_allowlist: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum SecretRef {
    Prefixed(String),
    Plain(String),
}

impl SecretRef {
    fn resolve(&self) -> Result<String> {
        let raw = match self {
            SecretRef::Prefixed(s) => s,
            SecretRef::Plain(s) => s,
        };
        if let Some(var) = raw.strip_prefix("env:") {
            return std::env::var(var)
                .map(|v| v.trim().to_owned())
                .with_context(|| format!("env var {var} is not set"));
        }
        if let Some(path) = raw.strip_prefix("file:") {
            let p = Path::new(path);
            let v = std::fs::read_to_string(p)
                .with_context(|| format!("cannot read secret file {path}"))?;
            return Ok(v.trim().to_owned());
        }
        Ok(raw.to_owned())
    }
}

#[derive(Debug, Clone)]
pub struct Server {
    pub listen: String,
    pub max_body_bytes: usize,
    pub request_timeout: Duration,
    pub timestamp_window_secs: i64,
}

#[derive(Debug, Clone)]
pub struct Telegram {
    pub api_base: String,
}

#[derive(Debug, Clone)]
pub struct RateLimit {
    pub requests_per_minute: u32,
}

#[derive(Debug, Clone)]
pub struct Client {
    pub secret: String,
    pub allowed_ips: Vec<ipnet::IpNet>,
    pub bots: Vec<String>,
    pub methods_allowlist: Option<Vec<String>>,
}

// tiny local ipnet to avoid an extra crate
mod ipnet {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    use serde::Deserialize;

    #[derive(Debug, Clone)]
    pub enum IpNet {
        V4(u32, u8), // addr as bits, prefix
        V6(u128, u8),
    }

    impl IpNet {
        pub fn contains(&self, ip: &IpAddr) -> bool {
            match (self, ip) {
                (IpNet::V4(net, prefix), IpAddr::V4(v4)) => {
                    let pfx = *prefix as u32;
                    let mask = if pfx == 0 { 0 } else { u32::MAX << (32 - pfx) };
                    net & mask == u32::from(*v4) & mask
                }
                (IpNet::V6(net, prefix), IpAddr::V6(v6)) => {
                    let pfx = *prefix as u32;
                    let mask = if pfx == 0 { 0 } else { u128::MAX << (128 - pfx) };
                    net & mask == u128::from(*v6) & mask
                }
                _ => false,
            }
        }
    }

    impl<'de> Deserialize<'de> for IpNet {
        fn deserialize<D: serde::Deserializer<'de>>(
            d: D,
        ) -> std::result::Result<Self, D::Error> {
            let s = String::deserialize(d)?;
            let (addr, pfx) = s.split_once('/').ok_or_else(|| {
                serde::de::Error::custom(format!("expected addr/prefix, got {s:?}"))
            })?;
            let pfx: u8 = pfx.parse().map_err(serde::de::Error::custom)?;
            match addr.parse::<Ipv4Addr>() {
                Ok(v4) => {
                    if pfx > 32 {
                        return Err(serde::de::Error::custom("bad v4 prefix"));
                    }
                    Ok(IpNet::V4(u32::from(v4), pfx))
                }
                Err(_) => {
                    let v6: Ipv6Addr = addr
                        .parse()
                        .map_err(|_| serde::de::Error::custom(format!("bad ip {addr:?}")))?;
                    if pfx > 128 {
                        return Err(serde::de::Error::custom("bad v6 prefix"));
                    }
                    Ok(IpNet::V6(u128::from(v6), pfx))
                }
            }
        }
    }
}

#[derive(Debug, Deserialize)]
struct ConfigToml {
    server: ServerToml,
    telegram: Option<TelegramToml>,
    rate_limit: Option<RateLimitToml>,
    metrics: Option<MetricsToml>,
    #[serde(default)]
    bots: HashMap<String, BotToml>,
    #[serde(default)]
    clients: Vec<ClientToml>,
    #[serde(default)]
    actions: Vec<ActionNamedToml>,
}

#[derive(Debug, Deserialize)]
struct ActionNamedToml {
    name: String,
    #[serde(flatten)]
    action: Action,
}

pub fn load(path: &str) -> Result<Config> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read config {path}"))?;
    let t: ConfigToml =
        toml::from_str(&raw).with_context(|| format!("bad config {path}"))?;

    let mut bots = HashMap::new();
    for (alias, spec) in &t.bots {
        bots.insert(alias.clone(), spec.token.resolve()?);
    }
    let mut clients = HashMap::new();
    for spec in &t.clients {
        let name = spec.name.clone();
        let mut nets = Vec::with_capacity(spec.allowed_ips.len());
        for s in &spec.allowed_ips {
            let net = parse_net(s)
                .with_context(|| format!("client {name}: bad allowed_ips entry {s:?}"))?;
            nets.push(net);
        }
        if clients
            .insert(
                name.clone(),
                Client {
                    secret: spec.secret.resolve()?,
                    allowed_ips: nets,
                    bots: spec.bots.clone(),
                    methods_allowlist: if spec.methods_allowlist.is_empty() {
                        None
                    } else {
                        Some(spec.methods_allowlist.clone())
                    },
                },
            )
            .is_some()
        {
            bail!("duplicate client name {name}");
        }
    }

    if bots.is_empty() {
        bail!("no [[bots]] defined");
    }
    if clients.is_empty() {
        bail!("no [[clients]] defined");
    }

    let mut actions = HashMap::new();
    for spec in &t.actions {
        if !clients.contains_key(&spec.action.client) {
            bail!(
                "action {}: unknown client {}",
                spec.name,
                spec.action.client
            );
        }
        if !bots.contains_key(&spec.action.bot) {
            bail!("action {}: unknown bot {}", spec.name, spec.action.bot);
        }
        if actions
            .insert(spec.name.clone(), spec.action.clone())
            .is_some()
        {
            bail!("duplicate action name {}", spec.name);
        }
    }

    Ok(Config {
        server: Server {
            listen: t.server.listen.clone(),
            max_body_bytes: t.server.max_body_bytes,
            request_timeout: Duration::from_secs(t.server.request_timeout_secs),
            timestamp_window_secs: t.server.timestamp_window_secs,
        },
        telegram: Telegram {
            api_base: t
                .telegram
                .as_ref()
                .and_then(|x| x.api_base.clone())
                .unwrap_or_else(|| "https://api.telegram.org".into()),
        },
        rate_limit: RateLimit {
            requests_per_minute: t.rate_limit.as_ref().and_then(|r| r.requests_per_minute).unwrap_or(120),
        },
        metrics: Metrics {
            enabled: t.metrics.as_ref().map(|m| m.enabled).unwrap_or(true),
        },
        bots,
        clients,
        actions,
    })
}

fn parse_net(s: &str) -> Result<ipnet::IpNet> {
    Ok(serde_json::from_value::<ipnet::IpNet>(
        serde_json::Value::String(s.to_owned()),
    )?)
}
