use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context};
use clap::{Args, Parser, Subcommand};
use mihomo_core::output::{ConfigDeployer, FileDeployer};
use mihomo_core::storage::{self, AppPaths, CustomRule, RuleKind, SubscriptionList};
use mihomo_core::subscription::{Subscription, SubscriptionKind};
use mihomo_core::{merge_configs, Template};
use tokio::fs;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(
    name = "mihomo-cli",
    author,
    version,
    about = "Mihomo subscription merge CLI",
    long_about = "Generate Mihomo/Clash configuration files by combining a template with one or more subscriptions.\n\nUse `mihomo-cli merge --help` for command-specific options and defaults for runtime directories under ~/.config/mihomocli.",
    arg_required_else_help = true
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    #[command(
        about = "Merge subscriptions with a template",
        long_about = "Load subscriptions (from the default list or ad-hoc sources), merge them with a template, and emit a Mihomo-compatible config.",
        after_long_help = r#"
Examples:

  Minimal (template resolves under ~/.config/mihomocli/templates):

    mihomo-cli merge --template default.yaml


  Full example (all options combined):

    mihomo-cli merge \
      --template default.yaml \
      --base-config base-config.yaml \
      --subscriptions-file ~/.config/mihomocli/subscriptions.yaml \
      -s https://example.com/sub.yaml \
      -s ./extras/local.yaml \
      --output ~/.config/mihomocli/output/config.yaml


  Print to stdout instead of writing a file:

    mihomo-cli merge --template default.yaml --stdout -s https://example.com/sub.yaml


Notes:

  - Relative paths for --template are resolved under ~/.config/mihomocli/templates/.

  - Relative paths for --base-config are resolved under ~/.config/mihomocli/.

  - If --subscriptions-file is omitted, the default list at ~/.config/mihomocli/subscriptions.yaml is used.

  - Multiple -s/--subscription entries may be provided (URL or file path).

  - Use --stdout or --output, not both.

  - Use --subscription-ua to override the HTTP User-Agent for fetching subscriptions.

    Defaults to 'clash-verge/v2.4.2' to coax providers into returning Clash YAML with rules.

  - Use --subscription-allow-base64 to enable decoding base64/share-link subscriptions (trojan/vmess/ss).

    Disabled by default; when disabled, only native Clash YAML is accepted from providers.

  - Use --dev-rules to prepend GitHub, Docker, GCR, and cache.nixos.org rules that force traffic through a proxy.

    Change the target proxy/group with --dev-rules-via (defaults to 'Proxy').
"#
    )]
    Merge(MergeArgs),

    /// Show or manage cached state and quick rules
    #[command(subcommand)]
    Manage(Manage),
}

// Note: default clap styles are used to avoid introducing extra dependencies

#[derive(Args)]
struct MergeArgs {
    /// Template YAML file path. Relative paths resolve against the templates directory.
    #[arg(long)]
    template: PathBuf,

    /// Optional base config to inherit fields/rules from (e.g., clash-verge.yaml).
    #[arg(long)]
    base_config: Option<PathBuf>,

    /// Optional subscriptions YAML definition (defaults to ~/.config/mihomocli/subscriptions.yaml).
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

    /// Prepend common developer domains with proxy rules (GitHub, Docker, GCR, cache.nixos.org).
    #[arg(long = "dev-rules", default_value_t = false)]
    dev_rules: bool,

    /// Proxy group/tag used by generated dev rules when --dev-rules is set.
    #[arg(long = "dev-rules-via", default_value = DEFAULT_DEV_RULE_VIA)]
    dev_rules_via: String,

    /// Reuse the cached last subscription URL when no -s/--subscription is provided.
    /// If both are set, explicit subscriptions take precedence.
    #[arg(long = "use-last", default_value_t = false)]
    use_last: bool,

    /// HTTP User-Agent used to fetch subscriptions (some providers vary output by UA).
    /// Defaults to clash-verge UA to obtain Clash YAML with rules when available.
    #[arg(long = "subscription-ua")]
    subscription_ua: Option<String>,

    /// Allow decoding base64/subscription share-link lists when fetching subscriptions.
    /// Disabled by default to prefer native Clash YAML from providers.
    #[arg(long = "subscription-allow-base64", default_value_t = false)]
    subscription_allow_base64: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();

    let cli = Cli::parse();

    match cli.command {
        Commands::Merge(args) => run_merge(args).await?,
        Commands::Manage(cmd) => run_manage(cmd).await?,
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
    let mut app_cfg = storage::load_app_config(&paths).await?;

    // Mimic clash-verge UA so some providers return Clash YAML (with rules)
    let ua = args
        .subscription_ua
        .clone()
        .unwrap_or_else(|| "clash-verge/v2.4.2".to_string());
    let client = reqwest::Client::builder().user_agent(&ua).build()?;

    // Configure core parser behavior (align with UA behavior):
    // by default, do NOT attempt base64 decoding; allow only if explicitly requested.
    mihomo_core::subscription::set_parse_options(mihomo_core::subscription::ParseOptions {
        allow_base64: args.subscription_allow_base64,
    });

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
    let mut used_url: Option<String> = None;

    for subscription in subscription_list.items.iter_mut() {
        match subscription.load_config(&client, &paths).await {
            Ok(Some(config)) => configs.push(config),
            Ok(None) => {}
            Err(err) => {
                tracing::error!(id = %subscription.id, error = %err, "failed to load subscription");
            }
        }
        if let Some(url) = subscription.url.clone() {
            used_url = Some(url);
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
        if let Some(url) = subscription.url.clone() {
            used_url = Some(url);
        }
    }

    // If requested and no explicit sources, reuse cached last subscription URL
    if configs.is_empty() && args.subscriptions.is_empty() && subscription_list.items.is_empty() {
        if args.use_last {
            if let Some(last_url) = app_cfg.last_subscription_url.clone() {
                tracing::info!(last_url = %last_url, "using cached last subscription URL");
                let mut subscription = subscription_from_input(0, &last_url);
                match subscription.load_config(&client, &paths).await {
                    Ok(Some(config)) => {
                        configs.push(config);
                        used_url = Some(last_url);
                    }
                    Ok(None) => {}
                    Err(err) => {
                        return Err(anyhow!(
                            "failed to load cached subscription {}: {}",
                            last_url,
                            err
                        ));
                    }
                }
            } else {
                return Err(anyhow!(
                    "--use-last set but no cached last subscription URL found. Merge once with -s/--subscription first."
                ));
            }
        } else {
            return Err(anyhow!(
                "no subscription provided. Pass -s/--subscription or use --use-last to reuse the cached last URL."
            ));
        }
    }

    let mut merged = merge_configs(template, configs);
    if let Some(base) = base_config.as_ref() {
        merged = mihomo_core::merge::apply_base_config(merged, base);
    }

    if args.dev_rules {
        let mut dev_rules = build_dev_rules(&args.dev_rules_via);
        dev_rules.extend(merged.rules.into_iter());
        merged.rules = dev_rules;
    }

    // Prepend custom quick rules (take precedence)
    if !app_cfg.custom_rules.is_empty() {
        let mut quick = Vec::with_capacity(app_cfg.custom_rules.len());
        for r in &app_cfg.custom_rules {
            let tag = match r.kind {
                RuleKind::Domain => "DOMAIN",
                RuleKind::DomainSuffix => "DOMAIN-SUFFIX",
                RuleKind::DomainKeyword => "DOMAIN-KEYWORD",
            };
            quick.push(format!("{},{},{}", tag, r.domain, r.via));
        }
        let mut new_rules = quick;
        new_rules.extend(merged.rules.into_iter());
        merged.rules = new_rules;
    }

    let yaml = merged.to_yaml_string()?;

    if args.stdout {
        println!("{}", yaml);
    } else {
        let output_path = args
            .output
            .clone()
            .unwrap_or_else(|| paths.output_config_path());
        ensure_parent(&output_path).await?;
        let deployer = FileDeployer {
            path: output_path.clone(),
        };
        deployer.deploy(&yaml).await.with_context(|| {
            format!("failed to write merged config to {}", output_path.display())
        })?;
        println!("merged config written to {}", output_path.display());
    }

    if args.subscriptions_file.is_none() {
        storage::save_subscription_list(&paths, &subscription_list).await?;
    } else if let Some(custom) = args.subscriptions_file.as_ref() {
        save_subscriptions_to_path(custom, &subscription_list).await?;
    }

    // Update caches after successful merge
    if let Some(url) = used_url.take() {
        app_cfg.last_subscription_url = Some(url);
        storage::save_app_config(&paths, &app_cfg).await?;
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

const DEFAULT_DEV_RULE_VIA: &str = "Proxy";

fn build_dev_rules(via: &str) -> Vec<String> {
    const DEV_RULE_TARGETS: &[(&str, &str)] = &[
        ("DOMAIN-SUFFIX", "github.com"),
        ("DOMAIN-SUFFIX", "githubusercontent.com"),
        ("DOMAIN-SUFFIX", "githubassets.com"),
        ("DOMAIN-SUFFIX", "githubstatic.com"),
        ("DOMAIN-SUFFIX", "docker.com"),
        ("DOMAIN-SUFFIX", "docker.io"),
        ("DOMAIN-SUFFIX", "dockerusercontent.com"),
        ("DOMAIN-SUFFIX", "registry-1.docker.io"),
        ("DOMAIN-SUFFIX", "gcr.io"),
        ("DOMAIN-SUFFIX", "k8s.gcr.io"),
        ("DOMAIN-SUFFIX", "pkg.dev"),
        ("DOMAIN", "cache.nixos.org"),
    ];

    DEV_RULE_TARGETS
        .iter()
        .map(|(kind, target)| format!("{kind},{target},{via}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dev_rules_use_selected_via() {
        let via = "MyProxy";
        let rules = build_dev_rules(via);
        assert!(rules
            .iter()
            .all(|rule| rule.ends_with(&format!(",{}", via))));
        assert!(rules
            .iter()
            .any(|rule| rule.starts_with("DOMAIN-SUFFIX,github.com,")));
    }
}

fn default_base_config_path(paths: &AppPaths) -> Option<PathBuf> {
    let candidate = paths.app_config_path().with_file_name("base-config.yaml");
    if candidate.exists() {
        Some(candidate)
    } else {
        None
    }
}

async fn ensure_parent(path: &Path) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await?;
    }
    Ok(())
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

// Management commands (cache and custom rules)

#[derive(Subcommand)]
enum Manage {
    /// Show or clear the cached last subscription URL
    #[command(subcommand)]
    Cache(CacheCmd),

    /// Manage quick custom rules that force domains via a specific proxy
    #[command(subcommand)]
    Custom(CustomCmd),
}

#[derive(Subcommand)]
enum CacheCmd {
    /// Show the cached last subscription URL
    Show,
    /// Clear the cached last subscription URL
    Clear,
}

#[derive(Subcommand)]
enum CustomCmd {
    /// Add a custom rule
    Add(CustomAddArgs),
    /// List custom rules
    List,
    /// Remove custom rules matching domain (and optionally via)
    Remove(CustomRemoveArgs),
}

#[derive(Args)]
struct CustomAddArgs {
    /// Domain to match (e.g., cache.nixos.org)
    #[arg(long)]
    domain: String,
    /// Proxy or group name to route via
    #[arg(long)]
    via: String,
    /// Match kind: domain|suffix|keyword (default: suffix)
    #[arg(long, default_value = "suffix")]
    kind: String,
}

#[derive(Args)]
struct CustomRemoveArgs {
    /// Domain to remove
    #[arg(long)]
    domain: String,
    /// Optional proxy/group name to narrow removal
    #[arg(long)]
    via: Option<String>,
}

async fn run_manage(cmd: Manage) -> anyhow::Result<()> {
    let paths = AppPaths::new()?;
    paths.ensure_runtime_dirs().await?;
    match cmd {
        Manage::Cache(c) => manage_cache(&paths, c).await,
        Manage::Custom(c) => manage_custom(&paths, c).await,
    }
}

async fn manage_cache(paths: &AppPaths, cmd: CacheCmd) -> anyhow::Result<()> {
    let mut cfg = storage::load_app_config(paths).await?;
    match cmd {
        CacheCmd::Show => {
            if let Some(url) = cfg.last_subscription_url.as_ref() {
                println!("last-subscription-url: {}", url);
            } else {
                println!("last-subscription-url: <none>");
            }
        }
        CacheCmd::Clear => {
            cfg.last_subscription_url = None;
            storage::save_app_config(paths, &cfg).await?;
            println!("cleared last-subscription-url");
        }
    }
    Ok(())
}

async fn manage_custom(paths: &AppPaths, cmd: CustomCmd) -> anyhow::Result<()> {
    let mut cfg = storage::load_app_config(paths).await?;
    match cmd {
        CustomCmd::Add(args) => {
            let kind = match args.kind.to_ascii_lowercase().as_str() {
                "domain" => RuleKind::Domain,
                "keyword" => RuleKind::DomainKeyword,
                _ => RuleKind::DomainSuffix,
            };
            let rule = CustomRule {
                domain: args.domain,
                kind,
                via: args.via,
            };
            if !cfg.custom_rules.contains(&rule) {
                cfg.custom_rules.push(rule);
                storage::save_app_config(paths, &cfg).await?;
                println!("custom rule added");
            } else {
                println!("custom rule already exists");
            }
        }
        CustomCmd::List => {
            if cfg.custom_rules.is_empty() {
                println!("<no custom rules>");
            } else {
                for r in &cfg.custom_rules {
                    let kind = match r.kind {
                        RuleKind::Domain => "DOMAIN",
                        RuleKind::DomainSuffix => "DOMAIN-SUFFIX",
                        RuleKind::DomainKeyword => "DOMAIN-KEYWORD",
                    };
                    println!("{},{},{}", kind, r.domain, r.via);
                }
            }
        }
        CustomCmd::Remove(args) => {
            let before = cfg.custom_rules.len();
            cfg.custom_rules.retain(|r| {
                if r.domain != args.domain {
                    return true;
                }
                if let Some(v) = args.via.as_ref() {
                    // keep if via doesn't match
                    return &r.via != v;
                }
                // drop all with this domain
                false
            });
            let after = cfg.custom_rules.len();
            storage::save_app_config(paths, &cfg).await?;
            println!("removed {} rule(s)", before.saturating_sub(after));
        }
    }
    Ok(())
}
