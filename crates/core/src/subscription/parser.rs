use std::collections::HashMap;

use anyhow::{anyhow, Context};
use std::collections::HashSet;

use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;
use percent_encoding::percent_decode_str;
use serde_json::Value as JsonValue;
use serde_yaml::{Mapping, Number, Sequence, Value};
use url::Url;

use crate::model::ClashConfig;

#[derive(Clone, Copy, Debug, Default)]
pub struct ParseOptions {
    pub allow_base64: bool,
}

/// Attempt to interpret the raw subscription payload as a ClashConfig.
///
/// - First try native YAML deserialization.
/// - Then attempt to decode base64-wrapped data.
/// - Finally, treat the decoded/plain text as a list of share links (trojan/vmess/ss).
#[allow(dead_code)]
pub fn parse_subscription_payload(raw: &str) -> anyhow::Result<ClashConfig> {
    parse_subscription_payload_with_options(raw, ParseOptions { allow_base64: true })
}

pub fn parse_subscription_payload_with_options(
    raw: &str,
    opts: ParseOptions,
) -> anyhow::Result<ClashConfig> {
    // Fast path: valid YAML Clash configuration.
    if let Ok(config) = serde_yaml::from_str::<ClashConfig>(raw) {
        return Ok(config);
    }

    if opts.allow_base64 {
        let mut decoded_candidates = decode_candidates(raw);

        for candidate in decoded_candidates.iter() {
            if let Ok(config) = serde_yaml::from_str::<ClashConfig>(candidate) {
                return Ok(config);
            }
        }

        for candidate in decoded_candidates.drain(..) {
            if let Some(config) = parse_share_links(&candidate)? {
                return Ok(config);
            }
        }
    }

    if let Some(config) = parse_share_links(raw)? {
        return Ok(config);
    }

    Err(anyhow!(
        "subscription payload is neither valid Clash YAML nor supported share links"
    ))
}

fn decode_candidates(raw: &str) -> Vec<String> {
    let filtered: String = raw.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    if filtered.is_empty() {
        return Vec::new();
    }

    let mut out = Vec::new();
    let mut seen = HashSet::new();

    for bytes in [
        STANDARD.decode(&filtered),
        URL_SAFE_NO_PAD.decode(&filtered),
    ]
    .into_iter()
    .flatten()
    {
        if bytes.is_empty() {
            continue;
        }
        if let Ok(text) = String::from_utf8(bytes) {
            if is_mostly_printable(&text) && seen.insert(text.clone()) {
                out.push(text);
            }
        }
    }

    out
}

fn is_mostly_printable(text: &str) -> bool {
    let mut printable = 0usize;
    let mut control = 0usize;

    for ch in text.chars() {
        if ch.is_ascii_control() && ch != '\n' && ch != '\r' && ch != '\t' {
            control += 1;
        } else {
            printable += 1;
        }

        if control > 8 {
            return false;
        }
    }

    printable > 0
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
    map.insert(Value::from("udp"), Value::Bool(true));

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
    map.insert(Value::from("udp"), Value::Bool(true));

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
    map.insert(Value::from("udp"), Value::Bool(true));

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
    while !padded.len().is_multiple_of(4) {
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
        assert_eq!(map.get(Value::from("type")).and_then(Value::as_str), Some("trojan"));
        assert_eq!(map.get(Value::from("server")).and_then(Value::as_str), Some("example.com"));
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
        assert_eq!(map.get(Value::from("type")).and_then(Value::as_str), Some("vmess"));
        assert_eq!(map.get(Value::from("server")).and_then(Value::as_str), Some("vmess.example.com"));
        assert_eq!(map.get(Value::from("uuid")).and_then(Value::as_str), Some("123e4567-e89b-12d3-a456-426614174000"));
    }

    #[test]
    fn parse_shadowsocks_link() {
        // ss://base64(method:password)@server:port#tag
        // Base64 encoded: aes-256-gcm:password@ss.example.com:8388
        let ss_link = "ss://YWVzLTI1Ni1nY206cGFzc3dvcmRAc3MuZXhhbXBsZS5jb206ODM4OA==#SS%20Example";
        let config = parse_subscription_payload(ss_link).expect("should parse");
        assert_eq!(config.proxies.len(), 1);
        let proxy = config.proxies.first().expect("proxy");
        let map = proxy.as_mapping().expect("mapping");
        assert_eq!(map.get(Value::from("type")).and_then(Value::as_str), Some("ss"));
        assert_eq!(map.get(Value::from("server")).and_then(Value::as_str), Some("ss.example.com"));
        assert_eq!(map.get(Value::from("cipher")).and_then(Value::as_str), Some("aes-256-gcm"));
        assert_eq!(map.get(Value::from("password")).and_then(Value::as_str), Some("password"));
    }

    #[test]
    fn parse_shadowsocks_plain_format() {
        let ss_link = "ss://aes-256-gcm:password@ss.example.com:8388#PlainFormat";
        let config = parse_subscription_payload(ss_link).expect("should parse");
        assert_eq!(config.proxies.len(), 1);
        let proxy = config.proxies.first().expect("proxy");
        let map = proxy.as_mapping().expect("mapping");
        assert_eq!(map.get(Value::from("server")).and_then(Value::as_str), Some("ss.example.com"));
        assert_eq!(map.get(Value::from("port")).and_then(Value::as_u64).map(|v| v as u16), Some(8388));
    }

    #[test]
    fn parse_mixed_share_links() {
        let mixed = r#"trojan://pass1@example1.com:443#Trojan1
vmess://eyJwcyI6IlZtZXNzMSIsImFkZCI6ImV4YW1wbGUyLmNvbSIsInBvcnQiOiI0NDMiLCJpZCI6IjEyM2U0NTY3LWU4OWItMTJkMy1hNDU2LTQyNjYxNDE3NDAwMCJ9
ss://aes-128-gcm:test@example3.com:8388#SS1"#;

        let config = parse_subscription_payload(mixed).expect("should parse");
        assert_eq!(config.proxies.len(), 3);

        let types: Vec<_> = config
            .proxies
            .iter()
            .filter_map(|p| {
                p.as_mapping()
                    .and_then(|m| m.get(Value::from("type")).and_then(Value::as_str))
            })
            .collect();
        assert!(types.contains(&"trojan"));
        assert!(types.contains(&"vmess"));
        assert!(types.contains(&"ss"));
    }

    #[test]
    fn parse_direct_yaml_config() {
        let yaml = r#"
port: 7890
proxies:
  - name: test-proxy
    type: http
    server: example.com
    port: 8080
rules:
  - "DOMAIN,example.com,DIRECT"
"#;
        let config = parse_subscription_payload(yaml).expect("should parse");
        assert_eq!(config.port, Some(7890));
        assert_eq!(config.proxies.len(), 1);
        assert_eq!(config.rules.len(), 1);
    }

    #[test]
    fn parse_invalid_payload_returns_error() {
        let invalid = "this is not a valid subscription format\nno proxies here";
        let result = parse_subscription_payload(invalid);
        assert!(result.is_err());
    }

    #[test]
    fn parse_empty_and_whitespace_input() {
        // Empty string is actually valid YAML (empty document), so it may parse successfully
        // Let's test what actually happens
        let result = parse_subscription_payload_with_options(
            "",
            ParseOptions {
                allow_base64: false,
            },
        );
        if let Ok(config) = result {
            // If it parses, it should have empty proxies
            assert_eq!(config.proxies.len(), 0);
        } else {
            // Or it returns an error, which is also acceptable
            assert!(result.is_err());
        }

        // Whitespace-only should behave similarly
        let result2 = parse_subscription_payload_with_options(
            "   \n\n  ",
            ParseOptions {
                allow_base64: false,
            },
        );
        if let Ok(config2) = result2 {
            assert_eq!(config2.proxies.len(), 0);
        }
    }

    #[test]
    fn parse_with_allow_base64_disabled() {
        let ss_link = "ss://aes-256-gcm:password@ss.example.com:8388#Test";
        let opts = ParseOptions {
            allow_base64: false,
        };
        // Should still parse plain share links
        let config = parse_subscription_payload_with_options(ss_link, opts).expect("should parse");
        assert_eq!(config.proxies.len(), 1);
    }

    #[test]
    fn test_is_mostly_printable() {
        assert!(is_mostly_printable("normal text\n"));
        assert!(is_mostly_printable("trojan://test@example.com:443"));
        // String with more than 8 control characters (excluding newline, \r, \t)
        // \x09 is tab (allowed), so we need more control chars
        assert!(!is_mostly_printable(
            "\x01\x02\x03\x04\x05\x06\x07\x08\x0b\x0c\x0d\x0e\x0f"
        ));
        // Mostly control characters should fail
        assert!(!is_mostly_printable("\x01\x02\x03\x04\x05\x06\x07\x08\x0b"));
    }

    #[test]
    fn test_pad_base64() {
        assert_eq!(pad_base64("YWJj"), "YWJj");
        assert_eq!(pad_base64("YWJjZA"), "YWJjZA==");
        assert_eq!(pad_base64("YWJjZGU"), "YWJjZGU=");
    }
}
