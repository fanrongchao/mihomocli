use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_yaml::Value;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct ClashConfig {
    #[serde(default)]
    pub port: Option<u16>,

    #[serde(rename = "socks-port", default)]
    pub socks_port: Option<u16>,

    #[serde(rename = "redir-port", default)]
    pub redir_port: Option<u16>,

    #[serde(default)]
    pub proxies: Vec<Value>,

    #[serde(rename = "proxy-groups", default)]
    pub proxy_groups: Vec<Value>,

    #[serde(default)]
    pub rules: Vec<String>,

    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl ClashConfig {
    pub fn from_yaml_str(input: &str) -> anyhow::Result<Self> {
        let config: ClashConfig = serde_yaml::from_str(input)?;
        Ok(config)
    }

    pub fn to_yaml_string(&self) -> anyhow::Result<String> {
        let yaml = serde_yaml::to_string(self)?;
        Ok(yaml)
    }

    pub fn proxy_names(&self) -> Vec<String> {
        self.proxies
            .iter()
            .filter_map(|proxy| match proxy {
                Value::Mapping(map) => map
                    .get(&Value::from("name"))
                    .and_then(|value| value.as_str())
                    .map(|s| s.to_string()),
                _ => None,
            })
            .collect()
    }
}
