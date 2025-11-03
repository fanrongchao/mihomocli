use std::path::PathBuf;

use async_trait::async_trait;
use tokio::fs;

#[async_trait]
pub trait ConfigDeployer {
    async fn deploy(&self, yaml: &str) -> anyhow::Result<()>;
}

pub struct FileDeployer {
    pub path: PathBuf,
}

#[async_trait]
impl ConfigDeployer for FileDeployer {
    async fn deploy(&self, yaml: &str) -> anyhow::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).await?;
        }
        fs::write(&self.path, yaml).await?;
        Ok(())
    }
}

pub struct HttpDeployer {
    pub endpoint: String,
    pub secret: Option<String>,
}

#[async_trait]
impl ConfigDeployer for HttpDeployer {
    async fn deploy(&self, _yaml: &str) -> anyhow::Result<()> {
        anyhow::bail!("HTTP deployer not implemented yet: {}", self.endpoint);
    }
}
