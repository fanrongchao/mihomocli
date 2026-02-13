use std::path::{Path, PathBuf};

use anyhow::anyhow;
use directories::BaseDirs;
use serde::{Deserialize, Serialize};
use tokio::fs;

use crate::subscription::Subscription;

#[derive(Debug, Clone)]
pub struct AppPaths {
    config_dir: PathBuf,
    cache_dir: PathBuf,
}

impl AppPaths {
    pub fn new() -> anyhow::Result<Self> {
        let base = BaseDirs::new().ok_or_else(|| anyhow!("failed to resolve base directories"))?;
        let config_dir = base.home_dir().join(".config/mihomocli");
        let cache_dir = base.home_dir().join(".cache/mihomocli/subscriptions");
        Ok(Self {
            config_dir,
            cache_dir,
        })
    }

    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    pub fn templates_dir(&self) -> PathBuf {
        self.config_dir.join("templates")
    }

    pub fn default_template_path(&self) -> PathBuf {
        self.templates_dir().join("cvr_template.yaml")
    }

    pub fn resources_dir(&self) -> PathBuf {
        self.config_dir.join("resources")
    }

    pub fn app_config_path(&self) -> PathBuf {
        self.config_dir.join("app.yaml")
    }

    pub fn subscriptions_file(&self) -> PathBuf {
        self.config_dir.join("subscriptions.yaml")
    }

    pub fn output_config_path(&self) -> PathBuf {
        self.config_dir.join("output/config.yaml")
    }

    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    pub fn cache_file(&self, id: &str) -> PathBuf {
        self.cache_dir.join(format!("{id}.yaml"))
    }

    pub fn cache_meta_file(&self, id: &str) -> PathBuf {
        self.cache_dir.join(format!("{id}.meta.json"))
    }

    pub async fn ensure_runtime_dirs(&self) -> anyhow::Result<()> {
        fs::create_dir_all(self.config_dir()).await?;
        fs::create_dir_all(self.templates_dir()).await?;
        fs::create_dir_all(self.resources_dir()).await?;
        if let Some(parent) = self.output_config_path().parent() {
            fs::create_dir_all(parent).await?;
        }
        fs::create_dir_all(self.cache_dir()).await?;
        Ok(())
    }

    pub fn resource_file<S: AsRef<str>>(&self, name: S) -> PathBuf {
        self.resources_dir().join(name.as_ref())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SubscriptionList {
    pub current: Option<String>,
    #[serde(default)]
    pub items: Vec<Subscription>,
}

impl SubscriptionList {
    pub fn enabled(&self) -> impl Iterator<Item = &Subscription> {
        self.items.iter().filter(|sub| sub.enabled)
    }
}

pub async fn load_subscription_list(paths: &AppPaths) -> anyhow::Result<SubscriptionList> {
    match fs::read_to_string(paths.subscriptions_file()).await {
        Ok(contents) => {
            let list: SubscriptionList = serde_yaml::from_str(&contents)?;
            Ok(list)
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            let list = SubscriptionList::default();
            save_subscription_list(paths, &list).await?;
            Ok(list)
        }
        Err(err) => Err(err.into()),
    }
}

pub async fn save_subscription_list(
    paths: &AppPaths,
    list: &SubscriptionList,
) -> anyhow::Result<()> {
    let yaml = serde_yaml::to_string(list)?;
    if let Some(parent) = paths.subscriptions_file().parent() {
        fs::create_dir_all(parent).await?;
    }
    fs::write(paths.subscriptions_file(), yaml).await?;
    Ok(())
}

// App configuration (simple key-value plus custom rules)

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppConfig {
    #[serde(default)]
    pub last_subscription_url: Option<String>,

    #[serde(default)]
    pub custom_rules: Vec<CustomRule>,

    /// Manually-managed server sources (references to local files containing share links).
    ///
    /// This is intentionally a file reference so secrets (trojan passwords, etc.) do not need to
    /// live inside app.yaml.
    #[serde(default)]
    pub manual_servers: Vec<ManualServerRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManualServerRef {
    pub name: String,
    pub file: PathBuf,
    /// Optional proxy-groups to append the injected proxy names into (e.g., a provider selector).
    #[serde(default)]
    pub attach_groups: Vec<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RuleKind {
    Domain,
    DomainSuffix,
    DomainKeyword,
}

fn default_rule_kind() -> RuleKind {
    RuleKind::DomainSuffix
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CustomRule {
    pub domain: String,
    #[serde(default = "default_rule_kind")]
    pub kind: RuleKind,
    pub via: String,
}

pub async fn load_app_config(paths: &AppPaths) -> anyhow::Result<AppConfig> {
    match fs::read_to_string(paths.app_config_path()).await {
        Ok(raw) => Ok(serde_yaml::from_str(&raw)?),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(AppConfig::default()),
        Err(err) => Err(err.into()),
    }
}

pub async fn save_app_config(paths: &AppPaths, cfg: &AppConfig) -> anyhow::Result<()> {
    if let Some(parent) = paths.app_config_path().parent() {
        fs::create_dir_all(parent).await?;
    }
    let yaml = serde_yaml::to_string(cfg)?;
    fs::write(paths.app_config_path(), yaml).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_paths(temp_dir: &TempDir) -> AppPaths {
        let config_dir = temp_dir.path().join("config");
        let cache_dir = temp_dir.path().join("cache");
        AppPaths {
            config_dir,
            cache_dir,
        }
    }

    #[tokio::test]
    async fn test_app_paths_creation() {
        let temp_dir = TempDir::new().unwrap();
        let paths = create_test_paths(&temp_dir);

        assert_eq!(
            paths.templates_dir(),
            temp_dir.path().join("config/templates")
        );
        assert_eq!(
            paths.resources_dir(),
            temp_dir.path().join("config/resources")
        );
        assert_eq!(
            paths.app_config_path(),
            temp_dir.path().join("config/app.yaml")
        );
        assert_eq!(
            paths.subscriptions_file(),
            temp_dir.path().join("config/subscriptions.yaml")
        );
        assert_eq!(
            paths.output_config_path(),
            temp_dir.path().join("config/output/config.yaml")
        );
        assert_eq!(
            paths.cache_file("test-id"),
            temp_dir.path().join("cache/test-id.yaml")
        );
        assert_eq!(
            paths.cache_meta_file("test-id"),
            temp_dir.path().join("cache/test-id.meta.json")
        );
    }

    #[tokio::test]
    async fn test_ensure_runtime_dirs() {
        let temp_dir = TempDir::new().unwrap();
        let paths = create_test_paths(&temp_dir);

        paths.ensure_runtime_dirs().await.unwrap();

        assert!(paths.config_dir().exists());
        assert!(paths.templates_dir().exists());
        assert!(paths.resources_dir().exists());
        assert!(paths.cache_dir().exists());
        assert!(paths.output_config_path().parent().unwrap().exists());
    }

    #[tokio::test]
    async fn test_load_save_subscription_list() {
        let temp_dir = TempDir::new().unwrap();
        let paths = create_test_paths(&temp_dir);
        paths.ensure_runtime_dirs().await.unwrap();

        // Test loading non-existent file (should create default)
        let list = load_subscription_list(&paths).await.unwrap();
        assert_eq!(list.items.len(), 0);
        assert_eq!(list.current, None);

        // Test saving and loading
        let new_list = SubscriptionList {
            current: Some("test-id".to_string()),
            items: vec![Subscription {
                id: "test-id".to_string(),
                name: "Test Subscription".to_string(),
                url: Some("https://example.com/sub".to_string()),
                path: None,
                last_updated: None,
                etag: None,
                last_modified: None,
                kind: crate::subscription::SubscriptionKind::Clash,
                enabled: true,
            }],
        };

        save_subscription_list(&paths, &new_list).await.unwrap();

        let loaded = load_subscription_list(&paths).await.unwrap();
        assert_eq!(loaded.current, Some("test-id".to_string()));
        assert_eq!(loaded.items.len(), 1);
        assert_eq!(loaded.items[0].name, "Test Subscription");
    }

    #[tokio::test]
    async fn test_subscription_list_enabled_filter() {
        let list = SubscriptionList {
            current: None,
            items: vec![
                Subscription {
                    id: "enabled1".to_string(),
                    name: "Enabled 1".to_string(),
                    url: Some("https://example.com/1".to_string()),
                    path: None,
                    last_updated: None,
                    etag: None,
                    last_modified: None,
                    kind: crate::subscription::SubscriptionKind::Clash,
                    enabled: true,
                },
                Subscription {
                    id: "disabled".to_string(),
                    name: "Disabled".to_string(),
                    url: Some("https://example.com/2".to_string()),
                    path: None,
                    last_updated: None,
                    etag: None,
                    last_modified: None,
                    kind: crate::subscription::SubscriptionKind::Clash,
                    enabled: false,
                },
                Subscription {
                    id: "enabled2".to_string(),
                    name: "Enabled 2".to_string(),
                    url: Some("https://example.com/3".to_string()),
                    path: None,
                    last_updated: None,
                    etag: None,
                    last_modified: None,
                    kind: crate::subscription::SubscriptionKind::Clash,
                    enabled: true,
                },
            ],
        };

        let enabled: Vec<_> = list.enabled().collect();
        assert_eq!(enabled.len(), 2);
        assert_eq!(enabled[0].id, "enabled1");
        assert_eq!(enabled[1].id, "enabled2");
    }

    #[tokio::test]
    async fn test_load_save_app_config() {
        let temp_dir = TempDir::new().unwrap();
        let paths = create_test_paths(&temp_dir);
        paths.ensure_runtime_dirs().await.unwrap();

        // Test loading non-existent file (should return default)
        let config = load_app_config(&paths).await.unwrap();
        assert_eq!(config.last_subscription_url, None);
        assert_eq!(config.custom_rules.len(), 0);
        assert_eq!(config.manual_servers.len(), 0);

        // Test saving and loading with data
        let new_config = AppConfig {
            last_subscription_url: Some("https://example.com/sub".to_string()),
            custom_rules: vec![
                CustomRule {
                    domain: "example.com".to_string(),
                    kind: RuleKind::Domain,
                    via: "PROXY".to_string(),
                },
                CustomRule {
                    domain: "google.com".to_string(),
                    kind: RuleKind::DomainSuffix,
                    via: "DIRECT".to_string(),
                },
            ],
            manual_servers: vec![ManualServerRef {
                name: "jp-vultr".to_string(),
                file: PathBuf::from("/run/secrets/manual_share_links"),
                attach_groups: vec!["BosLife".to_string()],
                enabled: true,
            }],
        };

        save_app_config(&paths, &new_config).await.unwrap();

        let loaded = load_app_config(&paths).await.unwrap();
        assert_eq!(
            loaded.last_subscription_url,
            Some("https://example.com/sub".to_string())
        );
        assert_eq!(loaded.custom_rules.len(), 2);
        assert_eq!(loaded.custom_rules[0].domain, "example.com");
        assert_eq!(loaded.custom_rules[0].kind, RuleKind::Domain);
        assert_eq!(loaded.custom_rules[1].kind, RuleKind::DomainSuffix);
        assert_eq!(loaded.manual_servers.len(), 1);
        assert_eq!(loaded.manual_servers[0].name, "jp-vultr");
    }

    #[tokio::test]
    async fn test_manual_server_default_enabled() {
        let yaml = r#"
name: jp
file: /run/secrets/jp
"#;
        let s: ManualServerRef = serde_yaml::from_str(yaml).unwrap();
        assert!(s.enabled);
    }

    #[tokio::test]
    async fn test_custom_rule_default_kind() {
        let yaml = r#"
domain: example.com
via: PROXY
"#;
        let rule: CustomRule = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(rule.kind, RuleKind::DomainSuffix); // Default
    }

    #[tokio::test]
    async fn test_custom_rule_serialization() {
        let rule = CustomRule {
            domain: "test.com".to_string(),
            kind: RuleKind::DomainKeyword,
            via: "REJECT".to_string(),
        };

        let yaml = serde_yaml::to_string(&rule).unwrap();
        let deserialized: CustomRule = serde_yaml::from_str(&yaml).unwrap();

        assert_eq!(deserialized.domain, "test.com");
        assert_eq!(deserialized.kind, RuleKind::DomainKeyword);
        assert_eq!(deserialized.via, "REJECT");
    }

    #[tokio::test]
    async fn test_rule_kind_serde() {
        // Test kebab-case serialization
        let yaml_domain = serde_yaml::to_string(&RuleKind::Domain).unwrap();
        assert!(yaml_domain.contains("domain"));

        let yaml_suffix = serde_yaml::to_string(&RuleKind::DomainSuffix).unwrap();
        assert!(yaml_suffix.contains("domain-suffix"));

        let yaml_keyword = serde_yaml::to_string(&RuleKind::DomainKeyword).unwrap();
        assert!(yaml_keyword.contains("domain-keyword"));
    }
}
