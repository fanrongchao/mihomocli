use std::collections::HashMap;

use anyhow::{anyhow, Context};
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;
use percent_encoding::percent_decode_str;
use serde_json::Value as JsonValue;
use serde_yaml::{Mapping, Number, Sequence, Value};
use url::Url;

use crate::model::ClashConfig;

/// Attempt to interpret the raw subscription payload as a ClashConfig.
///
/// - First try native YAML deserialization.
/// - Then attempt to decode base64-wrapped data.
/// - Finally, treat the decoded/plain text as a list of share links (trojan/vmess/ss).
pub fn parse_subscription_payload(raw: &str) -> anyhow::Result<ClashConfig> {
    // Fast path: valid YAML Clash configuration.
    if let Ok(config) = serde_yaml::from_str::<ClashConfig>(raw) {
        return Ok(config);
    }

    // Try base64 decoding; keep the best-effort decoded text if it looks valid.
    if let Some(decoded) = try_decode_base64(raw) {
        if let Ok(config) = serde_yaml::from_str::<ClashConfig>(&decoded) {
            return Ok(config);
        }
        if let Some(config) = parse_share_links(&decoded)? {
            return Ok(config);
        }
    }

    // Finally, interpret the original text as share links.
    if let Some(config) = parse_share_links(raw)? {
        return Ok(config);
    }

    Err(anyhow!("subscription payload is neither valid Clash YAML nor supported share links"))
}

fn try_decode_base64(raw: &str) -> Option<String> {
    let filtered: String = raw.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    if filtered.is_empty() {
        return None;
    }

    // Try standard base64 first.
    if let Ok(bytes) = STANDARD.decode(&filtered) {
        if let Ok(text) = String::from_utf8(bytes) {
            if looks_like_share_links(&text) {
                return Some(text);
            }
        }
    }

    // Some subscriptions use URL-safe alphabet without padding.
    if let Ok(bytes) = URL_SAFE_NO_PAD.decode(&filtered) {
        if let Ok(text) = String::from_utf8(bytes) {
            if looks_like_share_links(&text) {
                return Some(text);
            }
        }
    }

    None
}

fn looks_like_share_links(text: &str) -> bool {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            (!line.is_empty()).then_some(line)
        })
        .any(|line| line.contains("://"))
}

fn parse_share_links(input: &str) -> anyhow::Result<Option<ClashConfig>> {
    let mut proxies = Vec::new();

    for line in input.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let value = if line.starts_with("trojan://") {
            parse_trojan(line)?
        } else if line.starts_with("vmess://") {
            parse_vmess(line)?
        } else if line.starts_with("ss://") {
            parse_shadowsocks(line)?
        } else {
            continue;
        };

        if let Some(value) = value {
            proxies.push(value);
        }
    }

    if proxies.is_empty() {
        return Ok(None);
    }

    Ok(Some(ClashConfig {
        proxies,
        ..Default::default()
    }))
}

fn parse_trojan(line: &str) -> anyhow::Result<Option<Value>> {
    let url = Url::parse(line)?;
    let server = url
        .host_str()
        .ok_or_else(|| anyhow!("trojan share link missing host"))?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| anyhow!("trojan share link missing port"))?;
    let password = percent_decode_str(url.username())
        .decode_utf8()
        .context("failed to decode trojan password")?
        .to_string();
    let name = url
        .fragment()
        .map(|frag| percent_decode_str(frag).decode_utf8_lossy().to_string())
        .unwrap_or_else(|| format!("{}:{}", server, port));

    let mut map = Mapping::new();
    insert_string(&mut map, "name", name);
    insert_string(&mut map, "type", "trojan");
    insert_string(&mut map, "server", server);
    insert_u64(&mut map, "port", port as u64);
    insert_string(&mut map, "password", password);

    let query: HashMap<_, _> = url.query_pairs().collect();

    if let Some(sni) = query.get("sni").or_else(|| query.get("peer")) {
        insert_string(&mut map, "sni", sni);
    }

    if let Some(value) = query.get("alpn") {
        let sequence = value
            .split(',')
            .map(|item| Value::from(item.trim()))
            .collect::<Sequence>();
        if !sequence.is_empty() {
            map.insert(Value::from("alpn"), Value::Sequence(sequence));
        }
    }

    if query
        .get("allowInsecure")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
    {
        map.insert(Value::from("skip-cert-verify"), Value::Bool(true));
    }

    if let Some(transport) = query.get("type") {
        let transport = transport.trim();
        if !transport.is_empty() {
            insert_string(&mut map, "network", transport);
            if transport.eq_ignore_ascii_case("ws") {
                let mut ws_opts = Mapping::new();
                if let Some(path) = query.get("path") {
                    insert_string(&mut ws_opts, "path", path);
                }
                if let Some(host) = query.get("host").or_else(|| query.get("hostHeader")) {
                    let mut headers = Mapping::new();
                    insert_string(&mut headers, "Host", host);
                    ws_opts.insert(Value::from("headers"), Value::Mapping(headers));
                }
                if !ws_opts.is_empty() {
                    map.insert(Value::from("ws-opts"), Value::Mapping(ws_opts));
                }
            }
        }
    }

    Ok(Some(Value::Mapping(map)))
}

fn parse_vmess(line: &str) -> anyhow::Result<Option<Value>> {
    let encoded = line.trim_start_matches("vmess://");
    let padded = pad_base64(encoded);
    let decoded = STANDARD
        .decode(padded)
        .context("failed to decode vmess base64 body")?;
    let json = String::from_utf8(decoded).context("vmess body is not valid UTF-8")?;
    let data: JsonValue = serde_json::from_str(&json).context("vmess body is not valid JSON")?;

    let server = data
        .get("add")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| anyhow!("vmess share link missing server"))?;
    let port = data
        .get("port")
        .and_then(|value| match value {
            JsonValue::String(s) => s.parse::<u16>().ok(),
            JsonValue::Number(n) => n.as_u64().and_then(|p| u16::try_from(p).ok()),
            _ => None,
        })
        .ok_or_else(|| anyhow!("vmess share link missing port"))?;
    let uuid = data
        .get("id")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| anyhow!("vmess share link missing uuid"))?;

    let name = data
        .get("ps")
        .and_then(JsonValue::as_str)
        .unwrap_or(server)
        .to_string();

    let mut map = Mapping::new();
    insert_string(&mut map, "name", name);
    insert_string(&mut map, "type", "vmess");
    insert_string(&mut map, "server", server);
    insert_u64(&mut map, "port", port as u64);
    insert_string(&mut map, "uuid", uuid);

    if let Some(alter_id) = data
        .get("aid")
        .and_then(JsonValue::as_str)
        .and_then(|s| s.parse::<u64>().ok())
        .or_else(|| data.get("aid").and_then(JsonValue::as_u64))
    {
        insert_u64(&mut map, "alterId", alter_id);
    }

    if let Some(cipher) = data
        .get("scy")
        .or_else(|| data.get("cipher"))
        .and_then(JsonValue::as_str)
    {
        if !cipher.is_empty() {
            insert_string(&mut map, "cipher", cipher);
        }
    }

    if let Some(net) = data.get("net").and_then(JsonValue::as_str) {
        if !net.is_empty() {
            insert_string(&mut map, "network", net);
            if net.eq_ignore_ascii_case("ws") {
                let mut ws_opts = Mapping::new();
                if let Some(path) = data.get("path").and_then(JsonValue::as_str) {
                    if !path.is_empty() {
                        insert_string(&mut ws_opts, "path", path);
                    }
                }
                if let Some(host) = data.get("host").and_then(JsonValue::as_str) {
                    if !host.is_empty() {
                        let mut headers = Mapping::new();
                        insert_string(&mut headers, "Host", host);
                        ws_opts.insert(Value::from("headers"), Value::Mapping(headers));
                    }
                }
                if !ws_opts.is_empty() {
                    map.insert(Value::from("ws-opts"), Value::Mapping(ws_opts));
                }
            }
        }
    }

    if let Some(tls) = data.get("tls").and_then(JsonValue::as_str) {
        if tls.eq_ignore_ascii_case("tls") || tls == "1" {
            map.insert(Value::from("tls"), Value::Bool(true));
        }
    }

    if let Some(sni) = data.get("sni").and_then(JsonValue::as_str) {
        if !sni.is_empty() {
            insert_string(&mut map, "servername", sni);
        }
    }

    if let Some(fp) = data.get("fp").and_then(JsonValue::as_str) {
        if !fp.is_empty() {
            insert_string(&mut map, "client-fingerprint", fp);
        }
    }

    if let Some(alpn) = data.get("alpn").and_then(JsonValue::as_str) {
        let sequence = alpn
            .split(',')
            .map(|item| Value::from(item.trim()))
            .collect::<Sequence>();
        if !sequence.is_empty() {
            map.insert(Value::from("alpn"), Value::Sequence(sequence));
        }
    }

    if let Some(allow_insecure) = data.get("allowInsecure") {
        if allow_insecure == &JsonValue::Bool(true)
            || allow_insecure == &JsonValue::String("1".into())
        {
            map.insert(Value::from("skip-cert-verify"), Value::Bool(true));
        }
    }

    Ok(Some(Value::Mapping(map)))
}

fn parse_shadowsocks(line: &str) -> anyhow::Result<Option<Value>> {
    let trimmed = line.trim_start_matches("ss://");
    let (main, tag) = match trimmed.split_once('#') {
        Some((body, tag)) => (body, Some(tag)),
        None => (trimmed, None),
    };

    let (body, plugin) = match main.split_once('?') {
        Some((b, q)) => (b, Some(q)),
        None => (main, None),
    };

    let credentials = if body.contains('@') {
        body.to_string()
    } else {
        let padded = pad_base64(body);
        let decoded = STANDARD
            .decode(padded)
            .context("failed to decode shadowsocks base64 body")?;
        String::from_utf8(decoded).context("shadowsocks credentials are not UTF-8")?
    };

    let (method_password, server_part) = credentials
        .rsplit_once('@')
        .ok_or_else(|| anyhow!("shadowsocks share link missing host"))?;
    let (method, password) = method_password
        .split_once(':')
        .ok_or_else(|| anyhow!("shadowsocks share link missing cipher or password"))?;
    let (server, port) = server_part
        .split_once(':')
        .ok_or_else(|| anyhow!("shadowsocks share link missing port"))?;
    let port: u16 = port.parse()?;

    let mut map = Mapping::new();
    let name = tag
        .map(|t| percent_decode_str(t).decode_utf8_lossy().to_string())
        .unwrap_or_else(|| format!("{}:{}", server, port));

    insert_string(&mut map, "name", name);
    insert_string(&mut map, "type", "ss");
    insert_string(&mut map, "server", server);
    insert_u64(&mut map, "port", port as u64);
    insert_string(&mut map, "cipher", method);
    insert_string(&mut map, "password", password);

    if let Some(plugin) = plugin {
        if let Some((_, opts)) = plugin.split_once("plugin=") {
            insert_string(&mut map, "plugin", opts);
        }
    }

    Ok(Some(Value::Mapping(map)))
}

fn insert_string<S: AsRef<str>>(map: &mut Mapping, key: &str, value: S) {
    map.insert(Value::from(key), Value::from(value.as_ref()));
}

fn insert_u64(map: &mut Mapping, key: &str, value: u64) {
    map.insert(Value::from(key), Value::Number(Number::from(value)));
}

fn pad_base64(input: &str) -> String {
    let mut padded = input.trim().to_string();
    while padded.len() % 4 != 0 {
        padded.push('=');
    }
    padded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_base64_trojan_subscription() {
        let raw_links = "trojan://password@example.com:443?allowInsecure=1&sni=example.com#Example";
        let encoded = STANDARD.encode(raw_links);

        let config = parse_subscription_payload(&encoded).expect("should parse");
        assert_eq!(config.proxies.len(), 1);
        let proxy = config.proxies.first().expect("proxy");
        let map = proxy.as_mapping().expect("mapping");
        assert_eq!(map.get(&Value::from("type")).and_then(Value::as_str), Some("trojan"));
        assert_eq!(map.get(&Value::from("server")).and_then(Value::as_str), Some("example.com"));
    }

    #[test]
    fn parse_vmess_subscription() {
        let json = serde_json::json!({
            "ps": "Test Node",
            "add": "vmess.example.com",
            "port": "443",
            "id": "123e4567-e89b-12d3-a456-426614174000",
            "aid": "0",
            "net": "ws",
            "path": "/ws",
            "host": "ws.example.com",
            "tls": "tls",
            "sni": "sni.example.com"
        });
        let encoded = format!("vmess://{}", STANDARD.encode(json.to_string()));
        let config = parse_subscription_payload(&encoded).expect("should parse");
        assert_eq!(config.proxies.len(), 1);
        let proxy = config.proxies.first().expect("proxy");
        let map = proxy.as_mapping().expect("mapping");
        assert_eq!(map.get(&Value::from("type")).and_then(Value::as_str), Some("vmess"));
        assert_eq!(map.get(&Value::from("server")).and_then(Value::as_str), Some("vmess.example.com"));
        assert_eq!(map.get(&Value::from("uuid")).and_then(Value::as_str), Some("123e4567-e89b-12d3-a456-426614174000"));
    }
}
