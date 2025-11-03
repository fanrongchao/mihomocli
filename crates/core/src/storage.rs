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
        let config_dir = base.home_dir().join(".config/mihomo-tui");
        let cache_dir = base.home_dir().join(".cache/mihomo-tui/subscriptions");
        Ok(Self { config_dir, cache_dir })
    }

    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    pub fn templates_dir(&self) -> PathBuf {
        self.config_dir.join("templates")
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

pub async fn save_subscription_list(paths: &AppPaths, list: &SubscriptionList) -> anyhow::Result<()> {
    let yaml = serde_yaml::to_string(list)?;
    if let Some(parent) = paths.subscriptions_file().parent() {
        fs::create_dir_all(parent).await?;
    }
    fs::write(paths.subscriptions_file(), yaml).await?;
    Ok(())
}
