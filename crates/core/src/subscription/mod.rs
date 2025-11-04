use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{anyhow, Context};
use chrono::{DateTime, Utc};
use reqwest::header::{HeaderMap, ETAG, IF_MODIFIED_SINCE, IF_NONE_MATCH, LAST_MODIFIED};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use tokio::fs;
use tracing::Instrument;

mod parser;
pub use parser::ParseOptions;

use crate::model::ClashConfig;
use crate::storage::AppPaths;
use parser::parse_subscription_payload_with_options;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subscription {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub path: Option<PathBuf>,
    #[serde(default)]
    pub last_updated: Option<DateTime<Utc>>,
    #[serde(default)]
    pub etag: Option<String>,
    #[serde(default)]
    pub last_modified: Option<String>,
    #[serde(default)]
    pub kind: SubscriptionKind,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SubscriptionKind {
    Clash,
    Merge,
    Script,
}

impl Default for SubscriptionKind {
    fn default() -> Self {
        SubscriptionKind::Clash
    }
}

impl Subscription {
    pub fn ensure_id(&mut self) {
        if self.id.is_empty() {
            self.id = self
                .url
                .clone()
                .or_else(|| self.path.as_ref().map(|p| p.display().to_string()))
                .unwrap_or_else(|| format!("{}", uuid::Uuid::new_v4()));
        }
    }

    pub async fn load_config(
        &mut self,
        client: &Client,
        paths: &AppPaths,
    ) -> anyhow::Result<Option<ClashConfig>> {
        if !self.enabled {
            return Ok(None);
        }

        self.ensure_id();

        match self.kind {
            SubscriptionKind::Clash => {}
            SubscriptionKind::Merge | SubscriptionKind::Script => {
                return Err(anyhow!(
                    "subscription kind {:?} is not supported for merging yet",
                    self.kind
                ))
            }
        }

        match (&self.url, &self.path) {
            (Some(url), _) => {
                let span = tracing::info_span!("fetch_subscription", id = %self.id, url);
                let fetch_result = fetch_remote(
                    client,
                    paths,
                    &self.id,
                    url,
                    self.etag.clone(),
                    self.last_modified.clone(),
                )
                .instrument(span)
                .await?;

                if let Some(new_etag) = fetch_result.etag.clone() {
                    self.etag = Some(new_etag);
                }
                if let Some(new_last_modified) = fetch_result.last_modified.clone() {
                    self.last_modified = Some(new_last_modified);
                }
                self.last_updated = Some(Utc::now());

                let config = parse_subscription_payload_with_options(
                    &fetch_result.yaml,
                    current_parse_options(),
                )?;
                Ok(Some(config))
            }
            (None, Some(path)) => {
                let span =
                    tracing::info_span!("read_subscription", id = %self.id, path = %path.display());
                let yaml = fs::read_to_string(path)
                    .instrument(span)
                    .await
                    .with_context(|| {
                        format!("failed to read subscription file {}", path.display())
                    })?;
                self.last_updated = Some(Utc::now());
                let config =
                    parse_subscription_payload_with_options(&yaml, current_parse_options())?;
                Ok(Some(config))
            }
            _ => Err(anyhow!("subscription {} missing url or path", self.id)),
        }
    }
}

static PARSE_OPTIONS: std::sync::OnceLock<ParseOptions> = std::sync::OnceLock::new();

/// Configure how subscription payloads are parsed (e.g., allow/disallow base64 list decoding).
/// Call once during program initialization.
pub fn set_parse_options(opts: ParseOptions) {
    let _ = PARSE_OPTIONS.set(opts);
}

fn current_parse_options() -> ParseOptions {
    *PARSE_OPTIONS
        .get()
        .unwrap_or(&ParseOptions { allow_base64: true })
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct SubscriptionCacheMeta {
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

struct FetchResult {
    yaml: String,
    etag: Option<String>,
    last_modified: Option<String>,
}

async fn fetch_remote(
    client: &Client,
    paths: &AppPaths,
    id: &str,
    url: &str,
    etag: Option<String>,
    last_modified: Option<String>,
) -> anyhow::Result<FetchResult> {
    let cache_file = paths.cache_file(id);
    let meta_file = paths.cache_meta_file(id);

    let cached_meta = match fs::read_to_string(&meta_file).await {
        Ok(raw) => serde_json::from_str::<SubscriptionCacheMeta>(&raw).unwrap_or_default(),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => SubscriptionCacheMeta::default(),
        Err(err) => return Err(err.into()),
    };

    let mut request = client.get(url);

    if let Some(header_etag) = etag.or_else(|| cached_meta.etag.clone()) {
        request = request.header(IF_NONE_MATCH, header_etag);
    }

    if let Some(header_last_modified) = last_modified.or_else(|| cached_meta.last_modified.clone())
    {
        request = request.header(IF_MODIFIED_SINCE, header_last_modified);
    }

    let response = match request.timeout(Duration::from_secs(30)).send().await {
        Ok(resp) => resp,
        Err(err) => {
            if let Some(cached) = read_cached_yaml(&cache_file).await? {
                tracing::warn!(id = id, error = %err, "network error, using cached subscription");
                return Ok(FetchResult {
                    yaml: cached,
                    etag: cached_meta.etag,
                    last_modified: cached_meta.last_modified,
                });
            }
            return Err(err.into());
        }
    };

    match response.status() {
        StatusCode::OK => {
            let headers = response.headers().clone();
            let yaml = response.text().await?;
            write_cache_files(&cache_file, &meta_file, &yaml, &headers).await?;
            let etag = header_to_string(headers.get(ETAG)).or(cached_meta.etag);
            let last_modified =
                header_to_string(headers.get(LAST_MODIFIED)).or(cached_meta.last_modified);

            Ok(FetchResult {
                yaml,
                etag,
                last_modified,
            })
        }
        StatusCode::NOT_MODIFIED => {
            let yaml = read_cached_yaml(&cache_file)
                .await?
                .ok_or_else(|| anyhow!("remote responded 304 but cache missing for {}", id))?;
            Ok(FetchResult {
                yaml,
                etag: cached_meta.etag,
                last_modified: cached_meta.last_modified,
            })
        }
        status if status.is_success() => {
            let headers = response.headers().clone();
            let yaml = response.text().await?;
            write_cache_files(&cache_file, &meta_file, &yaml, &headers).await?;
            Ok(FetchResult {
                yaml,
                etag: header_to_string(headers.get(ETAG)).or(cached_meta.etag),
                last_modified: header_to_string(headers.get(LAST_MODIFIED))
                    .or(cached_meta.last_modified),
            })
        }
        status => {
            if let Some(cached) = read_cached_yaml(&cache_file).await? {
                tracing::warn!(id = id, status = ?status, "unexpected status, falling back to cache");
                Ok(FetchResult {
                    yaml: cached,
                    etag: cached_meta.etag,
                    last_modified: cached_meta.last_modified,
                })
            } else {
                Err(anyhow!("failed to fetch subscription {}: {}", id, status))
            }
        }
    }
}

async fn read_cached_yaml(path: &Path) -> anyhow::Result<Option<String>> {
    match fs::read_to_string(path).await {
        Ok(content) => Ok(Some(content)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err.into()),
    }
}

async fn write_cache_files(
    cache_file: &Path,
    meta_file: &Path,
    yaml: &str,
    headers: &HeaderMap,
) -> anyhow::Result<()> {
    if let Some(parent) = cache_file.parent() {
        fs::create_dir_all(parent).await?;
    }
    fs::write(cache_file, yaml).await?;

    let meta = SubscriptionCacheMeta {
        etag: header_to_string(headers.get(ETAG)),
        last_modified: header_to_string(headers.get(LAST_MODIFIED)),
    };

    if let Some(parent) = meta_file.parent() {
        fs::create_dir_all(parent).await?;
    }
    fs::write(meta_file, serde_json::to_string(&meta)?).await?;
    Ok(())
}

fn header_to_string(value: Option<&reqwest::header::HeaderValue>) -> Option<String> {
    value
        .and_then(|val| val.to_str().ok())
        .map(|s| s.to_string())
}
