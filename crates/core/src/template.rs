use std::path::Path;

use anyhow::anyhow;
use serde_yaml::{Mapping, Value};
use tokio::fs;

use crate::model::ClashConfig;

#[derive(Debug, Clone)]
pub struct Template {
    raw: Mapping,
    config: ClashConfig,
}

impl Template {
    pub async fn load(path: &Path) -> anyhow::Result<Self> {
        let content = fs::read_to_string(path).await?;
        Self::from_yaml_str(&content)
    }

    pub fn from_yaml_str(yaml: &str) -> anyhow::Result<Self> {
        let value: Value = serde_yaml::from_str(yaml)?;
        let mapping = value
            .as_mapping()
            .cloned()
            .ok_or_else(|| anyhow!("template YAML must be a mapping"))?;
        let config: ClashConfig = serde_yaml::from_value(value)?;
        Ok(Self { raw: mapping, config })
    }

    pub fn into_config(self) -> ClashConfig {
        self.config
    }

    pub fn config(&self) -> &ClashConfig {
        &self.config
    }

    pub fn raw(&self) -> &Mapping {
        &self.raw
    }

    pub fn apply_merge(&mut self, merge: Mapping) -> anyhow::Result<()> {
        self.raw = merge_mappings(merge, self.raw.clone());
        let updated_value = Value::Mapping(self.raw.clone());
        self.config = serde_yaml::from_value(updated_value)?;
        Ok(())
    }
}

fn deep_merge(target: &mut Value, patch: &Value) {
    match (target, patch) {
        (Value::Mapping(target_map), Value::Mapping(patch_map)) => {
            for (key, value) in patch_map {
                let entry = target_map.entry(key.clone()).or_insert(Value::Null);
                deep_merge(entry, value);
            }
        }
        (target, patch) => *target = patch.clone(),
    }
}

fn merge_mappings(merge: Mapping, mut config: Mapping) -> Mapping {
    let mut config_value = Value::Mapping(config);
    let merge_value = Value::Mapping(lowercase_keys(merge));
    deep_merge(&mut config_value, &merge_value);
    config = config_value
        .as_mapping()
        .cloned()
        .unwrap_or_else(Mapping::new);
    config
}

fn lowercase_keys(mapping: Mapping) -> Mapping {
    let mut lowered = Mapping::new();
    for (key, value) in mapping.into_iter() {
        if let Some(key_str) = key.as_str() {
            lowered.insert(Value::from(key_str.to_ascii_lowercase()), value);
        }
    }
    lowered
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_behaves_like_clash_verge_rev() {
        let merge_yaml = r#"
prepend-rules:
  - prepend
append-rules:
  - append
prepend-proxies:
  - 9999
append-proxies:
  - 1111
rules:
  - replace
proxy-groups:
  - 123781923810
"#;

        let config_yaml = r#"
rules:
  - aaaaa
script1: test
"#;

        let merge_mapping: Mapping = serde_yaml::from_str(merge_yaml).unwrap();
        let config_mapping: Mapping = serde_yaml::from_str(config_yaml).unwrap();

        let merged = merge_mappings(merge_mapping, config_mapping);
        // Ensure original fields survive
        assert!(merged.contains_key(&Value::from("script1")));
        // Ensure lowercase conversion happens
        assert!(merged.contains_key(&Value::from("prepend-rules")));
    }

    #[test]
    fn template_apply_merge_updates_config() {
        let base_yaml = r#"
port: 7890
rules:
  - RULE-1
"#;
        let merge_yaml = r#"
rules:
  - RULE-2
"#;

        let mut template = Template::from_yaml_str(base_yaml).unwrap();
        let merge_mapping: Mapping = serde_yaml::from_str(merge_yaml).unwrap();
        template.apply_merge(merge_mapping).unwrap();

        let config = template.config();
        assert_eq!(config.rules, vec!["RULE-2".to_string()]);
        assert_eq!(config.port, Some(7890));
    }
}
