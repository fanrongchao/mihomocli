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
    if let Some(port) = base.port {
        merged.port = Some(port);
    }
    if let Some(socks) = base.socks_port {
        merged.socks_port = Some(socks);
    }
    if let Some(redir) = base.redir_port {
        merged.redir_port = Some(redir);
    }

    let mut extra = base.extra.clone();
    for (key, value) in merged.extra.into_iter() {
        extra.insert(key, value);
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
}
