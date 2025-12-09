use std::collections::HashSet;

use serde_yaml::Value;

use crate::model::ClashConfig;

const DEFAULT_SELECTOR_NAME: &str = "🚀 节点选择";

pub fn merge_configs(template: ClashConfig, subs: Vec<ClashConfig>) -> ClashConfig {
    let mut out = template;
    let mut all_proxy_names = Vec::new();
    let mut seen_proxy_names = HashSet::new();

    collect_proxy_names(&out.proxies, &mut all_proxy_names, &mut seen_proxy_names);

    for mut sub in subs {
        collect_proxy_names(&sub.proxies, &mut all_proxy_names, &mut seen_proxy_names);

        out.proxies.extend(sub.proxies.drain(..));
        out.rules.extend(sub.rules.drain(..));
        out.proxy_groups = merge_proxy_groups(out.proxy_groups, sub.proxy_groups);

        for (key, value) in sub.extra.into_iter() {
            out.extra.entry(key).or_insert(value);
        }
    }

    populate_default_selector(&mut out.proxy_groups, &all_proxy_names);

    out
}

pub fn apply_base_config(mut merged: ClashConfig, base: &ClashConfig) -> ClashConfig {
    // Ports: prefer base-config values. If base uses mixed-port (extra), drop legacy ports.
    if let Some(port) = base.port {
        merged.port = Some(port);
    }
    if let Some(socks) = base.socks_port {
        merged.socks_port = Some(socks);
    }
    if let Some(redir) = base.redir_port {
        merged.redir_port = Some(redir);
    }
    if base.extra.contains_key("mixed-port") {
        merged.port = None;
        merged.socks_port = None;
        merged.redir_port = None;
    }

    // Extra: keep base-config values when keys overlap; only add merged keys that base lacks.
    let mut extra = base.extra.clone();
    for (key, value) in merged.extra.into_iter() {
        extra.entry(key).or_insert(value);
    }
    merged.extra = extra;

    if !base.rules.is_empty() {
        merged.rules = base.rules.clone();
    }

    if !base.proxy_groups.is_empty() {
        let names = merged.proxy_names();
        let mut rebuilt = Vec::with_capacity(base.proxy_groups.len());
        for group in &base.proxy_groups {
            rebuilt.push(rebuild_group(group, &names));
        }
        merged.proxy_groups = rebuilt;
    }

    merged
}

fn merge_proxy_groups(mut base: Vec<Value>, incoming: Vec<Value>) -> Vec<Value> {
    for group in incoming.into_iter() {
        match proxy_group_name(&group) {
            Some(name) => {
                if let Some(existing) = base
                    .iter_mut()
                    .find(|candidate| proxy_group_name(candidate).as_deref() == Some(&name))
                {
                    merge_proxy_group(existing, &group);
                } else {
                    base.push(group);
                }
            }
            None => base.push(group),
        }
    }

    base
}

fn merge_proxy_group(base: &mut Value, incoming: &Value) {
    let base_map = match base.as_mapping_mut() {
        Some(map) => map,
        None => return,
    };

    let incoming_map = match incoming.as_mapping() {
        Some(map) => map,
        None => return,
    };

    let proxies_key = Value::from("proxies");

    let base_proxies = base_map
        .entry(proxies_key.clone())
        .or_insert_with(|| Value::Sequence(Vec::new()));

    if !matches!(base_proxies, Value::Sequence(_)) {
        *base_proxies = Value::Sequence(Vec::new());
    }

    let base_list = match base_proxies.as_sequence_mut() {
        Some(list) => list,
        None => return,
    };

    if let Some(incoming_list) = incoming_map
        .get(&proxies_key)
        .and_then(|value| value.as_sequence())
    {
        let mut existing: HashSet<String> = base_list
            .iter()
            .filter_map(|value| value.as_str().map(|s| s.to_string()))
            .collect();

        for proxy_name in incoming_list.iter().filter_map(|value| value.as_str()) {
            if existing.insert(proxy_name.to_string()) {
                base_list.push(Value::from(proxy_name));
            }
        }
    }
}

fn populate_default_selector(groups: &mut [Value], proxy_names: &[String]) {
    for group in groups.iter_mut() {
        let Some(name) = proxy_group_name(group) else {
            continue;
        };

        if name == DEFAULT_SELECTOR_NAME {
            if let Some(mapping) = group.as_mapping_mut() {
                let proxies_key = Value::from("proxies");
                let sequence = mapping
                    .entry(proxies_key)
                    .or_insert_with(|| Value::Sequence(Vec::new()));

                if let Some(list) = sequence.as_sequence_mut() {
                    list.clear();
                    list.extend(proxy_names.iter().cloned().map(Value::from));
                }
            }
        }
    }
}

fn proxy_group_name(value: &Value) -> Option<String> {
    match value {
        Value::Mapping(map) => map
            .get(&Value::from("name"))
            .and_then(|value| value.as_str())
            .map(|s| s.to_string()),
        _ => None,
    }
}

fn collect_proxy_names(values: &[Value], dest: &mut Vec<String>, seen: &mut HashSet<String>) {
    for value in values {
        if let Value::Mapping(map) = value {
            if let Some(name) = map
                .get(&Value::from("name"))
                .and_then(|value| value.as_str())
                .map(|s| s.to_string())
            {
                if seen.insert(name.clone()) {
                    dest.push(name);
                }
            }
        }
    }
}

fn rebuild_group(group: &Value, proxy_names: &[String]) -> Value {
    let Some(map) = group.as_mapping() else {
        return group.clone();
    };

    let mut rebuilt = map.clone();
    let proxies_key = Value::from("proxies");
    let new_list = proxy_names
        .iter()
        .cloned()
        .map(Value::from)
        .collect::<Vec<_>>();

    rebuilt.insert(proxies_key, Value::Sequence(new_list));

    Value::Mapping(rebuilt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ClashConfig;

    fn proxy(name: &str) -> Value {
        serde_yaml::from_str(&format!(
            "{{ name: \"{}\", type: \"http\", server: \"example.com\", port: 443 }}",
            name
        ))
        .unwrap()
    }

    fn selector_group(name: &str, proxies: &[&str]) -> Value {
        let proxies_yaml = proxies
            .iter()
            .map(|p| format!("- {}", p))
            .collect::<Vec<_>>()
            .join("\n");
        serde_yaml::from_str(&format!(
            "name: \"{name}\"\ntype: select\nproxies:\n{proxies}\n",
            proxies = proxies_yaml
        ))
        .unwrap()
    }

    #[test]
    fn test_merge_ports_template_wins() {
        let template = ClashConfig {
            port: Some(7890),
            ..Default::default()
        };

        let mut sub = ClashConfig::default();
        sub.port = Some(8888);

        let merged = merge_configs(template, vec![sub]);
        assert_eq!(merged.port, Some(7890));
    }

    #[test]
    fn test_merge_proxies_append() {
        let mut template = ClashConfig::default();
        template.proxies.push(proxy("A"));

        let mut sub = ClashConfig::default();
        sub.proxies.push(proxy("B"));

        let merged = merge_configs(template, vec![sub]);
        let names = merged.proxy_names();
        assert_eq!(names, vec!["A".to_string(), "B".to_string()]);
    }

    #[test]
    fn test_merge_proxy_groups_by_name() {
        let mut template = ClashConfig::default();
        template
            .proxy_groups
            .push(selector_group(DEFAULT_SELECTOR_NAME, &[]));

        let mut sub = ClashConfig::default();
        sub.proxy_groups
            .push(selector_group(DEFAULT_SELECTOR_NAME, &["B"]));
        sub.proxies.push(proxy("B"));

        let merged = merge_configs(template, vec![sub]);
        assert!(merged.proxy_groups.iter().any(|group| match group {
            Value::Mapping(map) => {
                map.get(&Value::from("proxies"))
                    .and_then(|value| value.as_sequence())
                    .map(|seq| seq.iter().any(|value| value.as_str() == Some("B")))
                    .unwrap_or(false)
            }
            _ => false,
        }));
    }

    #[test]
    fn test_merge_rules_append() {
        let mut template = ClashConfig::default();
        template.rules = vec!["RULE,TEMPLATE".to_string()];

        let mut sub = ClashConfig::default();
        sub.rules = vec!["RULE,SUB".to_string()];

        let merged = merge_configs(template, vec![sub]);
        assert_eq!(merged.rules, vec!["RULE,TEMPLATE", "RULE,SUB"]);
    }

    #[test]
    fn test_apply_base_config_overrides_rules_and_groups() {
        let mut base = ClashConfig::default();
        base.port = Some(8000);
        base.rules = vec!["BASE_RULE".to_string()];
        base.proxy_groups = vec![selector_group("BaseGroup", &[])];
        base.extra.insert("profile".into(), Value::from("store"));

        let mut merged = ClashConfig::default();
        merged.proxies.push(proxy("X"));
        merged.rules = vec!["MERGED_RULE".to_string()];

        let result = apply_base_config(merged, &base);
        assert_eq!(result.port, Some(8000));
        assert_eq!(result.rules, vec!["BASE_RULE".to_string()]);
        assert_eq!(result.proxy_groups.len(), 1);
        let group = result.proxy_groups[0].as_mapping().unwrap();
        let proxies = group
            .get(&Value::from("proxies"))
            .and_then(|v| v.as_sequence())
            .unwrap();
        assert_eq!(proxies.len(), 1);
        assert_eq!(proxies[0].as_str(), Some("X"));
        assert_eq!(
            result.extra.get("profile").and_then(Value::as_str),
            Some("store")
        );
    }

    #[test]
    fn test_merge_empty_configs() {
        let template = ClashConfig::default();
        let merged = merge_configs(template, vec![]);
        assert_eq!(merged.proxies.len(), 0);
        assert_eq!(merged.rules.len(), 0);
    }

    #[test]
    fn test_merge_multiple_subscriptions() {
        let template = ClashConfig::default();

        let mut sub1 = ClashConfig::default();
        sub1.proxies.push(proxy("A"));
        sub1.rules.push("RULE,A".to_string());

        let mut sub2 = ClashConfig::default();
        sub2.proxies.push(proxy("B"));
        sub2.rules.push("RULE,B".to_string());

        let merged = merge_configs(template, vec![sub1, sub2]);
        assert_eq!(merged.proxies.len(), 2);
        assert_eq!(merged.rules.len(), 2);
        assert_eq!(merged.proxy_names(), vec!["A", "B"]);
    }

    #[test]
    fn test_populate_default_selector_with_empty_proxies() {
        let mut template = ClashConfig::default();
        template
            .proxy_groups
            .push(selector_group(DEFAULT_SELECTOR_NAME, &[]));

        let merged = merge_configs(template, vec![]);
        // Should still create the group, but with empty proxies
        assert_eq!(merged.proxy_groups.len(), 1);
    }

    #[test]
    fn test_apply_base_config_mixed_port() {
        let mut base = ClashConfig::default();
        base.extra.insert("mixed-port".into(), Value::from(7890));

        let mut merged = ClashConfig::default();
        merged.port = Some(8080);
        merged.socks_port = Some(8081);

        let result = apply_base_config(merged, &base);
        // Legacy ports should be cleared when mixed-port is present
        assert_eq!(result.port, None);
        assert_eq!(result.socks_port, None);
        assert_eq!(
            result.extra.get("mixed-port").and_then(Value::as_u64),
            Some(7890)
        );
    }

    #[test]
    fn test_merge_duplicate_proxy_names() {
        let mut template = ClashConfig::default();
        template.proxies.push(proxy("A"));

        let mut sub = ClashConfig::default();
        sub.proxies.push(proxy("A")); // Duplicate name

        let merged = merge_configs(template, vec![sub]);
        // Both proxies should be present (dedup is only for group filling)
        assert_eq!(merged.proxies.len(), 2);
        // But proxy_names should still list both
        assert_eq!(merged.proxy_names().len(), 2);
    }

    #[test]
    fn test_extra_fields_merge() {
        let mut template = ClashConfig::default();
        template
            .extra
            .insert("template-key".into(), Value::from("template-value"));

        let mut sub = ClashConfig::default();
        sub.extra.insert("sub-key".into(), Value::from("sub-value"));
        sub.extra
            .insert("template-key".into(), Value::from("sub-value-override"));

        let merged = merge_configs(template, vec![sub]);
        // Template value should win (or_insert semantics)
        assert_eq!(
            merged.extra.get("template-key").and_then(Value::as_str),
            Some("template-value")
        );
        // Sub's unique key should be present
        assert_eq!(
            merged.extra.get("sub-key").and_then(Value::as_str),
            Some("sub-value")
        );
    }

    #[test]
    fn test_apply_base_config_preserves_proxies() {
        let base = ClashConfig::default();

        let mut merged = ClashConfig::default();
        merged.proxies.push(proxy("A"));
        merged.proxies.push(proxy("B"));

        let result = apply_base_config(merged, &base);
        // Proxies should be preserved
        assert_eq!(result.proxies.len(), 2);
    }
}
