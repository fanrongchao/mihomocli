use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context};
use clap::{Args, Parser, Subcommand};
use mihomo_core::output::{ConfigDeployer, FileDeployer};
use mihomo_core::storage::{self, AppPaths, SubscriptionList};
use mihomo_core::subscription::{Subscription, SubscriptionKind};
use mihomo_core::{merge_configs, Template};
use tokio::fs;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "mihomo-cli", author, version, about = "Mihomo subscription merge CLI", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Merge(MergeArgs),
}

#[derive(Args)]
struct MergeArgs {
    /// Template YAML file path. Relative paths resolve against the templates directory.
    #[arg(long)]
    template: PathBuf,

    /// Optional base config to inherit fields/rules from (e.g., clash-verge.yaml).
    #[arg(long)]
    base_config: Option<PathBuf>,

    /// Optional subscriptions YAML definition (defaults to ~/.config/mihomo-tui/subscriptions.yaml).
    #[arg(long)]
    subscriptions_file: Option<PathBuf>,

    /// Additional subscription sources (URL or file path). May be repeated.
    #[arg(long = "subscription", short = 's')]
    subscriptions: Vec<String>,

    /// Output config file path. Defaults to spec output if omitted.
    #[arg(long)]
    output: Option<PathBuf>,

    /// Write merged config to stdout instead of a file.
    #[arg(long)]
    stdout: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();

    let cli = Cli::parse();

    match cli.command {
        Commands::Merge(args) => run_merge(args).await?,
    }

    Ok(())
}

fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .try_init();
}

async fn run_merge(args: MergeArgs) -> anyhow::Result<()> {
    let paths = AppPaths::new()?;
    paths.ensure_runtime_dirs().await?;

    let client = reqwest::Client::builder()
        .user_agent("mihomo-cli/0.1")
        .build()?;

    ensure_mihomo_resources(&client, &paths).await?;

    let template_path = resolve_template_path(&paths, &args.template);
    let template = Template::load(&template_path)
        .await
        .with_context(|| format!("failed to load template from {}", template_path.display()))?
        .into_config();

    let base_config_path = args
        .base_config
        .as_ref()
        .map(|p| resolve_base_path(&paths, p))
        .or_else(|| default_base_config_path(&paths));

    let base_config = if let Some(path) = base_config_path {
        Some(
            Template::load(&path)
                .await
                .with_context(|| format!("failed to load base config from {}", path.display()))?
                .into_config(),
        )
    } else {
        None
    };

    let mut subscription_list = if let Some(path) = args.subscriptions_file.as_ref() {
        load_subscriptions_from_path(path).await?
    } else {
        storage::load_subscription_list(&paths).await?
    };

    let mut configs = Vec::new();

    for subscription in subscription_list.items.iter_mut() {
        match subscription.load_config(&client, &paths).await {
            Ok(Some(config)) => configs.push(config),
            Ok(None) => {}
            Err(err) => {
                tracing::error!(id = %subscription.id, error = %err, "failed to load subscription");
            }
        }
    }

    for (idx, source) in args.subscriptions.iter().enumerate() {
        let mut subscription = subscription_from_input(idx, source);
        match subscription.load_config(&client, &paths).await {
            Ok(Some(config)) => configs.push(config),
            Ok(None) => {}
            Err(err) => {
                tracing::error!(source = source, error = %err, "failed to load ad-hoc subscription");
            }
        }
    }

    let mut merged = merge_configs(template, configs);
    if let Some(base) = base_config.as_ref() {
        merged = mihomo_core::merge::apply_base_config(merged, base);
    }
    let yaml = merged.to_yaml_string()?;

    if args.stdout {
        println!("{}", yaml);
    } else {
        let output_path = args
            .output
            .clone()
            .unwrap_or_else(|| paths.output_config_path());
        let deployer = FileDeployer {
            path: output_path.clone(),
        };
        deployer
            .deploy(&yaml)
            .await
            .with_context(|| format!("failed to write merged config to {}", output_path.display()))?;
        println!("merged config written to {}", output_path.display());
    }

    if args.subscriptions_file.is_none() {
        storage::save_subscription_list(&paths, &subscription_list).await?;
    } else if let Some(custom) = args.subscriptions_file.as_ref() {
        save_subscriptions_to_path(custom, &subscription_list).await?;
    }

    Ok(())
}

fn resolve_template_path(paths: &AppPaths, provided: &Path) -> PathBuf {
    if provided.is_absolute() {
        provided.to_path_buf()
    } else {
        let candidate = paths.templates_dir().join(provided);
        if candidate.exists() {
            candidate
        } else {
            provided.to_path_buf()
        }
    }
}

fn resolve_base_path(paths: &AppPaths, provided: &Path) -> PathBuf {
    if provided.is_absolute() {
        provided.to_path_buf()
    } else {
        let candidate = paths.config_dir().join(provided);
        if candidate.exists() {
            candidate
        } else {
            provided.to_path_buf()
        }
    }
}

fn default_base_config_path(paths: &AppPaths) -> Option<PathBuf> {
    let candidate = paths.config_dir().join("base-config.yaml");
    if candidate.exists() {
        Some(candidate)
    } else {
        None
    }
}

fn subscription_from_input(index: usize, input: &str) -> Subscription {
    let mut subscription = Subscription {
        id: String::new(),
        name: format!("cli-{}", index),
        url: None,
        path: None,
        last_updated: None,
        etag: None,
        last_modified: None,
        kind: SubscriptionKind::Clash,
        enabled: true,
    };

    if is_url(input) {
        subscription.url = Some(input.to_string());
        subscription.name = url_name(input).unwrap_or(subscription.name.clone());
    } else {
        subscription.path = Some(PathBuf::from(input));
        subscription.name = Path::new(input)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .map(|s| s.to_string())
            .unwrap_or(subscription.name.clone());
    }

    subscription.ensure_id();
    subscription
}

fn is_url(input: &str) -> bool {
    input.starts_with("http://") || input.starts_with("https://")
}

fn url_name(input: &str) -> Option<String> {
    let start = input.find("//")? + 2;
    let rest = &input[start..];
    let end = rest.find('/')?;
    Some(rest[..end].to_string())
}

async fn load_subscriptions_from_path(path: &Path) -> anyhow::Result<SubscriptionList> {
    match fs::read_to_string(path).await {
        Ok(contents) => Ok(serde_yaml::from_str(&contents)?),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(SubscriptionList::default()),
        Err(err) => Err(err.into()),
    }
}

async fn save_subscriptions_to_path(path: &Path, list: &SubscriptionList) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await?;
    }
    fs::write(path, serde_yaml::to_string(list)?).await?;
    Ok(())
}

const RESOURCE_SOURCES: &[(&str, &str)] = &[
    (
        "Country.mmdb",
        "https://github.com/MetaCubeX/meta-rules-dat/releases/download/latest/country.mmdb",
    ),
    (
        "geoip.dat",
        "https://github.com/MetaCubeX/meta-rules-dat/releases/download/latest/geoip.dat",
    ),
    (
        "geosite.dat",
        "https://github.com/MetaCubeX/meta-rules-dat/releases/download/latest/geosite.dat",
    ),
];

async fn ensure_mihomo_resources(client: &reqwest::Client, paths: &AppPaths) -> anyhow::Result<()> {
    for (name, url) in RESOURCE_SOURCES.iter() {
        let target = paths.resource_file(name);

        if fs::try_exists(&target).await.unwrap_or(false) {
            continue;
        }

        info!(resource = %name, "downloading resource");
        let response = client.get(*url).send().await?;
        if !response.status().is_success() {
            warn!(resource = %name, status = ?response.status(), "failed to download resource");
            return Err(anyhow!("failed to download {name} from {url}"));
        }

        let bytes = response.bytes().await?;
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).await?;
        }
        fs::write(&target, &bytes).await?;
    }

    Ok(())
}
