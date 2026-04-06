use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context};
use clap::{Args, Parser, Subcommand, ValueEnum};
use mihomo_core::output::{ConfigDeployer, FileDeployer};
use mihomo_core::storage::{
    self, AppPaths, CustomRule, ManagedTailscaleCompat, ManualServerRef, RuleKind, SubscriptionList,
};
use mihomo_core::subscription::{Subscription, SubscriptionKind};
use mihomo_core::{merge_configs, Template};
use serde::Deserialize;
use serde_yaml::Value;
use tokio::fs;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

const SAFE_FAKE_IP_RANGE: &str = "172.19.0.1/16";
const TAILSCALE_BASE_FAKE_IP_BYPASS: [&str; 2] = ["+.tailscale.com", "+.ts.net"];
const TAILSCALE_ROUTE_EXCLUDES: [&str; 2] = ["100.64.0.0/10", "fd7a:115c:a1e0::/48"];
const TAILSCALE_BASE_DIRECT_RULES: [&str; 2] = [
    "DOMAIN-SUFFIX,tailscale.com,DIRECT",
    "DOMAIN-SUFFIX,ts.net,DIRECT",
];

#[derive(Parser)]
#[command(
    name = "mihomo-cli",
    author,
    version,
    about = "Mihomo subscription merge CLI",
    long_about = "Generate Mihomo/Clash configuration files by combining a template with one or more subscriptions.\n\nUse `mihomo-cli merge --help` for command-specific options and defaults for runtime directories under ~/.config/mihomocli.",
    arg_required_else_help = true,
    after_long_help = r#"
Quick Start Examples

  Merge (minimal, uses bundled CVR-aligned template):
    mihomo-cli merge

  Merge single subscription URL and write to default output:
    mihomo-cli merge -s https://example.com/sub.yaml

  Merge multiple sources (URL + local file):
    mihomo-cli merge -s https://example.com/sub.yaml -s ./extra.yaml

  Use the last successful subscription URL explicitly:
    mihomo-cli merge --use-last

  Print merged YAML to stdout (no file write):
    mihomo-cli merge --stdout -s https://example.com/sub.yaml

  Use a base-config to align ports/rules/groups with clash-verge-rev:
    mihomo-cli merge --base-config ~/.config/mihomocli/base-config.yaml -s https://example.com/sub.yaml

  Auto-detect local Clash Verge config and sync the generated result back:
    mihomo-cli merge --use-last --sync-to-clash-verge

  Override subscription HTTP User-Agent:
    mihomo-cli merge -s https://example.com/sub.yaml --subscription-ua "my-client/1.0"

  Allow base64/share-link formats (trojan/vmess/ss):
    mihomo-cli merge -s https://example.com/base64.txt --subscription-allow-base64

  Dev rules (enabled by default). Change target group or disable:
    mihomo-cli merge -s https://example.com/sub.yaml --dev-rules-via proxy
    mihomo-cli merge -s https://example.com/sub.yaml --no-dev-rules
    mihomo-cli merge -s https://example.com/sub.yaml --dev-rules-show

  Override external controller fields in output config:
    mihomo-cli merge -s https://example.com/sub.yaml \
      --external-controller-url 127.0.0.1 \
      --external-controller-port 9090 \
      --external-controller-secret secret

  Manage: Cache, Quick Custom Rules, and Check

  Show / clear cached last subscription URL:
    mihomo-cli manage cache show
    mihomo-cli manage cache clear

  Add a quick custom rule (prepend to rules):
    # Force domain suffix via a proxy/group named "proxy"
    mihomo-cli manage custom add --domain cache.nixos.org --kind suffix --via proxy
    # Route a domain directly without proxy
    mihomo-cli manage custom add --domain example.com --kind domain --via direct

  List / remove quick custom rules:
    mihomo-cli manage custom list
    mihomo-cli manage custom remove --domain cache.nixos.org
    mihomo-cli manage custom remove --domain cache.nixos.org --via proxy

  Check whether a domain should go via proxy or direct:
    mihomo-cli manage check --domain github.com    # proxy
    mihomo-cli manage check --domain example.com   # direct (unless overridden by custom rules)

  List all built-in dev domains considered proxy-worthy:
    mihomo-cli manage dev-list
    mihomo-cli manage dev-list --format yaml
    mihomo-cli manage dev-list --format json

Other Utilities

  Initialize config folders and seed the default template:
    mihomo-cli init

  Validate output config with mihomo -t (paths auto-detected):
    mihomo-cli test

Notes

  - Default directories live under ~/.config/mihomocli and ~/.cache/mihomocli.
  - The CLI downloads geo resources on demand into ~/.config/mihomocli/resources/.
  - Template lookup resolves relative paths under ~/.config/mihomocli/templates/.
  - If Clash Verge is installed locally, base-config and sync targets can be auto-detected.
"#
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

  Minimal (uses the bundled CVR-aligned template):

    mihomo-cli merge


  Full example (explicit template and options):

    mihomo-cli merge \
      --template custom.yaml \
      --base-config base-config.yaml \
      --subscriptions-file ~/.config/mihomocli/subscriptions.yaml \
      -s https://example.com/sub.yaml \
      -s ./extras/local.yaml \
      --output ~/.config/mihomocli/output/config.yaml


  Print to stdout instead of writing a file:

    mihomo-cli merge --stdout -s https://example.com/sub.yaml


  Use the detected Clash Verge config as base and sync the generated result back:

    mihomo-cli merge --use-last --sync-to-clash-verge


Notes:

  - Relative paths for --template are resolved under ~/.config/mihomocli/templates/.

  - Relative paths for --base-config are resolved under ~/.config/mihomocli/.

  - If --base-config is omitted, the CLI first checks ~/.config/mihomocli/base-config.yaml,
    then auto-detects Clash Verge's exported config (preferring clash-verge.yaml, then config.yaml).

  - If --subscriptions-file is omitted, the default list at ~/.config/mihomocli/subscriptions.yaml is used.

  - Multiple -s/--subscription entries may be provided (URL or file path).

  - Use --stdout or --output, not both.

  - Use --subscription-ua to override the HTTP User-Agent for fetching subscriptions.

    Defaults to 'clash-verge/v2.4.2' to coax providers into returning Clash YAML with rules.

  - Use --subscription-allow-base64 to enable decoding base64/share-link subscriptions (trojan/vmess/ss).

    Disabled by default; when disabled, only native Clash YAML is accepted from providers.

  - Dev rules are enabled by default, prepending proxy-routing for developer/AI endpoints.

    Change the target proxy/group with --dev-rules-via (defaults to 'Proxy'). Disable with --no-dev-rules.

  - Use --dev-rules-show to print the generated list (without changing output unless --dev-rules is enabled).

  - When --sync-to-clash-verge is set, the CLI first writes the normal output file and then
    backs up Clash Verge's current config.yaml before replacing it with the generated result.

  - Fake-IP bypass: To exempt domains from fake-ip (avoid DNS hijack),
    use --fake-ip-bypass <PATTERN>. This appends to dns.fake-ip-filter and
    ensures fake-ip-filter-mode: blacklist. Examples:

      mihomo-cli merge \
        -s https://example.com/sub.yaml \
        --fake-ip-bypass '+.example.com' \
        --fake-ip-bypass 'hs.example.com'

  Tailscale compatibility: keep fake-ip and tun from hijacking tailnet traffic:

    mihomo-cli merge --tailscale-compatible

  Tailscale compatibility with a custom tailnet suffix:

    mihomo-cli merge --tailscale-compatible --tailscale-tailnet-suffix example.com

  Also sync the Tailscale-safe dns/profile source files back into Clash Verge:

    mihomo-cli merge --tailscale-compatible --tailscale-tailnet-suffix example.com --sync-to-clash-verge-sources
"#
    )]
    Merge(MergeArgs),

    #[command(
        name = "refresh-clash-verge",
        about = "Refresh Clash Verge from the active local subscription",
        long_about = "Resolve the active Clash Verge remote subscription from profiles.yaml (unless a URL is passed explicitly), then run the same merge/sync flow used for local desktop refreshes. This keeps the refresh logic inside mihomo-cli instead of shell wrappers, which also makes future macOS/Windows adaptations live in code."
    )]
    RefreshClashVerge(RefreshClashVergeArgs),

    #[command(
        about = "Inspect or mutate the local Mihomo runtime via synced config files and controller reloads",
        long_about = "Operate on the locally detected Clash Verge / Mihomo runtime without relying on GUI toggles. Runtime actions update the runtime YAML first, keep Clash Verge's Merge profile aligned where possible, and then ask the controller to reload so file state and process state stay together."
    )]
    Runtime(RuntimeArgs),

    /// Show or manage cached state and quick rules
    #[command(subcommand)]
    Manage(Manage),

    /// Run mihomo to test the generated config (-t)
    #[command(about = "Validate output config with mihomo -t")]
    Test(TestArgs),

    /// Initialize config directories and default template
    #[command(about = "Create ~/.config/mihomocli structure and seed template")]
    Init,

    #[command(
        about = "Inspect local Mihomo, Clash Verge, system proxy, and Tailscale state",
        long_about = "Best-effort local diagnostics for the common desktop setup. Reports file-backed Clash/Mihomo config state, whether macOS system proxies appear enabled, Tailscale CLI health when available, and live controller connection hints when the controller API is reachable."
    )]
    Doctor(DoctorArgs),
}

// Note: default clap styles are used to avoid introducing extra dependencies

#[derive(Args)]
struct MergeArgs {
    /// Template YAML file path. Defaults to the auto-installed CVR-aligned template.
    #[arg(long)]
    template: Option<PathBuf>,

    /// Optional base config to inherit fields/rules from (e.g., clash-verge.yaml).
    #[arg(long)]
    base_config: Option<PathBuf>,

    /// Optional subscriptions YAML definition (defaults to ~/.config/mihomocli/subscriptions.yaml).
    #[arg(long)]
    subscriptions_file: Option<PathBuf>,

    /// Additional subscription sources (URL or file path). May be repeated.
    #[arg(long = "subscription", short = 's')]
    subscriptions: Vec<String>,

    /// Output config file path. Defaults to ~/.config/mihomocli/output/clash-verge.yaml if omitted.
    #[arg(long)]
    output: Option<PathBuf>,

    /// Final Clash mode for the generated config.
    #[arg(long = "mode", value_enum, default_value_t = ConfigMode::Rule)]
    mode: ConfigMode,

    /// Sniffer preset for transparent/TUN traffic.
    #[arg(long = "sniffer-preset", value_enum, default_value_t = SnifferPreset::Tun)]
    sniffer_preset: SnifferPreset,

    /// Also copy the generated YAML into the detected Clash Verge config.yaml.
    #[arg(long = "sync-to-clash-verge", default_value_t = false)]
    sync_to_clash_verge: bool,

    /// Also sync Tailscale-safe dns/profile settings into Clash Verge source files.
    #[arg(long = "sync-to-clash-verge-sources", default_value_t = false)]
    sync_to_clash_verge_sources: bool,

    /// Write merged config to stdout instead of a file.
    #[arg(long)]
    stdout: bool,

    /// Prepend common developer domains with proxy rules (GitHub, Docker, GCR, cache.nixos.org).
    #[arg(long = "dev-rules", default_value_t = true)]
    dev_rules: bool,

    /// Proxy group/tag used by generated dev rules when --dev-rules is set.
    #[arg(long = "dev-rules-via", default_value = DEFAULT_DEV_RULE_VIA)]
    dev_rules_via: String,

    /// Show the dev rule list that would be added (without modifying the result unless --dev-rules is set).
    #[arg(long = "dev-rules-show", default_value_t = false)]
    dev_rules_show: bool,

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

    /// Host/IP for external-controller (e.g., 0.0.0.0)
    #[arg(long = "external-controller-url")]
    external_controller_url: Option<String>,

    /// Port for external-controller (e.g., 9090)
    #[arg(long = "external-controller-port")]
    external_controller_port: Option<u16>,

    /// Secret used by external controller API
    #[arg(long = "external-controller-secret")]
    external_controller_secret: Option<String>,

    /// Append entries to dns.fake-ip-filter (to avoid DNS hijacking under fake-ip mode)
    /// Example: --fake-ip-filter-add "+.example.com" --fake-ip-filter-add "hs.example.com"
    #[arg(long = "fake-ip-filter-add")]
    fake_ip_filter_add: Vec<String>,

    /// Set dns.fake-ip-filter-mode: blacklist|whitelist (only applies in fake-ip mode)
    #[arg(long = "fake-ip-filter-mode")]
    fake_ip_filter_mode: Option<String>,

    /// Add CIDRs to tun.route-exclude-address (repeatable).
    /// Useful for Kubernetes Pod/Service CIDRs to avoid tun DNS/service hijacking.
    /// Defaults already include 10.42.0.0/16 and 10.43.0.0/16.
    #[arg(long = "k8s-cidr-exclude")]
    k8s_cidr_exclude: Vec<String>,

    /// Add arbitrary CIDRs to tun.route-exclude-address (repeatable).
    /// Use this to keep specific remote IPs/subnets out of mihomo TUN routing.
    #[arg(long = "route-exclude-address-add")]
    route_exclude_address_add: Vec<String>,

    /// Bypass fake-ip for specific domains/patterns (shorthand for adding to dns.fake-ip-filter in blacklist mode)
    /// Example: --fake-ip-bypass '+.example.com' --fake-ip-bypass 'hs.example.com'
    #[arg(long = "fake-ip-bypass")]
    fake_ip_bypass: Vec<String>,

    /// Do not write output; print a concise summary of the merged result
    #[arg(long = "dry-run", default_value_t = false)]
    dry_run: bool,

    /// Keep fake-ip and tun compatible with Tailscale by avoiding fake-ip overlap,
    /// bypassing Tailscale domains, and excluding tailnet CIDRs from tun routing.
    #[arg(long = "tailscale-compatible", default_value_t = false)]
    tailscale_compatible: bool,

    /// Tailnet DNS suffixes whose `tail.<suffix>` names should bypass fake-ip
    /// and be forced DIRECT under --tailscale-compatible. Repeatable.
    #[arg(long = "tailscale-tailnet-suffix")]
    tailscale_tailnet_suffixes: Vec<String>,

    /// Additional exact domains or domain suffixes that should bypass fake-ip
    /// and be forced DIRECT under --tailscale-compatible. Repeatable.
    /// Examples: --tailscale-direct-domain derp.example.com --tailscale-direct-domain +.corp.example.com
    #[arg(long = "tailscale-direct-domain")]
    tailscale_direct_domains: Vec<String>,
}

#[derive(Args)]
struct RefreshClashVergeArgs {
    /// Explicit subscription URL. If omitted, the current Clash Verge remote subscription is used.
    subscription: Option<String>,

    /// Final Clash mode for the generated config.
    #[arg(long = "mode")]
    mode: Option<ConfigMode>,

    /// Sniffer preset for transparent/TUN traffic.
    #[arg(long = "sniffer-preset")]
    sniffer_preset: Option<SnifferPreset>,

    /// Disable the Tailscale compatibility patch set.
    #[arg(long = "no-tailscale-compatible", default_value_t = false)]
    no_tailscale_compatible: bool,

    /// Tailnet DNS suffixes whose `tail.<suffix>` names should bypass fake-ip and be forced DIRECT.
    #[arg(long = "tailscale-tailnet-suffix")]
    tailscale_tailnet_suffixes: Vec<String>,

    /// Additional domains or suffixes that should bypass fake-ip and be forced DIRECT.
    #[arg(long = "tailscale-direct-domain")]
    tailscale_direct_domains: Vec<String>,

    /// Additional CIDRs that should stay out of the Mihomo TUN.
    #[arg(long = "route-exclude-address-add")]
    route_exclude_address_add: Vec<String>,

    /// Preview the resolved merge without writing files.
    #[arg(long = "dry-run", default_value_t = false)]
    dry_run: bool,
}

#[derive(Args)]
struct RuntimeArgs {
    #[command(subcommand)]
    command: RuntimeCommand,
}

#[derive(Subcommand)]
enum RuntimeCommand {
    /// Show detected runtime config/controller state
    Status(DoctorArgs),
    /// Reload the running Mihomo process from the detected runtime file
    Reload,
    /// Update the detected runtime files to a specific Clash mode and reload
    Mode(RuntimeModeArgs),
}

#[derive(Args)]
struct RuntimeModeArgs {
    /// Target Clash mode for the local runtime config.
    #[arg(value_enum)]
    mode: ConfigMode,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
enum ConfigMode {
    Rule,
    Global,
    Direct,
}

impl ConfigMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Rule => "rule",
            Self::Global => "global",
            Self::Direct => "direct",
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
enum SnifferPreset {
    Off,
    Tun,
}

#[derive(Args)]
struct DoctorArgs {
    /// Include a short sample of current live controller connections when available.
    #[arg(long = "show-connections", default_value_t = true)]
    show_connections: bool,

    /// Domains to highlight in live connections.
    #[arg(long = "focus-domain")]
    focus_domains: Vec<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();

    let cli = Cli::parse();

    match cli.command {
        Commands::Merge(args) => run_merge(args).await?,
        Commands::RefreshClashVerge(args) => run_refresh_clash_verge(args).await?,
        Commands::Runtime(args) => run_runtime(args).await?,
        Commands::Manage(cmd) => run_manage(cmd).await?,
        Commands::Test(args) => run_test(args).await?,
        Commands::Init => run_init().await?,
        Commands::Doctor(args) => run_doctor(args).await?,
    }

    Ok(())
}

async fn run_init() -> anyhow::Result<()> {
    let paths = AppPaths::new()?;
    // Create runtime directories (config, templates, resources, output, cache)
    paths.ensure_runtime_dirs().await?;
    // Install bundled default template if missing
    ensure_default_template(&paths).await?;

    println!(
        "Initialized at: {}\n  - templates: {}\n  - resources: {}\n  - output: {}\n  - cache: {}",
        paths.config_dir().display(),
        paths.templates_dir().display(),
        paths.resources_dir().display(),
        paths
            .output_config_path()
            .parent()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "<unknown>".into()),
        paths.cache_dir().display()
    );

    Ok(())
}

#[derive(Debug, Deserialize)]
struct ClashVergeProfiles {
    current: Option<String>,
    #[serde(default)]
    items: Vec<ClashVergeProfileItem>,
}

#[derive(Debug, Deserialize)]
struct ClashVergeProfileItem {
    uid: Option<String>,
    url: Option<String>,
}

async fn run_refresh_clash_verge(args: RefreshClashVergeArgs) -> anyhow::Result<()> {
    let paths = AppPaths::new()?;
    paths.ensure_runtime_dirs().await?;
    let app_cfg = storage::load_app_config(&paths).await?;

    let subscription = if let Some(subscription) = args.subscription.clone() {
        subscription
    } else {
        detect_current_clash_verge_subscription_url(&paths).await?
    };

    let mode = args
        .mode
        .or_else(|| config_mode_from_env("MIHOMOCLI_MODE"))
        .unwrap_or(ConfigMode::Rule);
    let sniffer_preset = args
        .sniffer_preset
        .or_else(|| sniffer_preset_from_env("MIHOMOCLI_SNIFFER_PRESET"))
        .unwrap_or(SnifferPreset::Tun);

    let managed_tailnet_suffixes = app_cfg
        .managed_tailscale_compat
        .as_ref()
        .map(derive_tailnet_suffixes_from_managed)
        .unwrap_or_default();
    let managed_direct_domains = app_cfg
        .managed_tailscale_compat
        .as_ref()
        .map(derive_direct_domains_from_managed)
        .unwrap_or_default();
    let managed_route_excludes = app_cfg
        .managed_tailscale_compat
        .as_ref()
        .map(derive_extra_route_excludes_from_managed)
        .unwrap_or_default();

    let configured_tailnet_suffixes = app_cfg
        .tailscale_compat_defaults
        .as_ref()
        .map(|defaults| defaults.tailnet_suffixes.clone())
        .filter(|items| !items.is_empty())
        .unwrap_or(managed_tailnet_suffixes);
    let configured_direct_domains = app_cfg
        .tailscale_compat_defaults
        .as_ref()
        .map(|defaults| defaults.direct_domains.clone())
        .filter(|items| !items.is_empty())
        .unwrap_or(managed_direct_domains);
    let configured_route_excludes = app_cfg
        .tailscale_compat_defaults
        .as_ref()
        .map(|defaults| defaults.route_exclude_address.clone())
        .filter(|items| !items.is_empty())
        .unwrap_or(managed_route_excludes);

    let tailnet_suffixes = merge_string_lists(
        configured_tailnet_suffixes,
        env_list("MIHOMOCLI_TAILSCALE_SUFFIXES"),
        env_list("MIHOMOCLI_TAILSCALE_SUFFIX"),
        args.tailscale_tailnet_suffixes,
    );
    let direct_domains = merge_string_lists(
        configured_direct_domains,
        env_list("MIHOMOCLI_TAILSCALE_DIRECT_DOMAINS"),
        Vec::new(),
        args.tailscale_direct_domains,
    );
    let direct_cidrs = merge_string_lists(
        configured_route_excludes,
        env_list("MIHOMOCLI_TAILSCALE_DIRECT_CIDRS"),
        Vec::new(),
        args.route_exclude_address_add,
    );

    println!("Refreshing generated Clash Verge config from subscription:");
    println!("  {}", subscription);
    println!("Using Tailscale tailnet suffixes:");
    println!(
        "  {}",
        if tailnet_suffixes.is_empty() {
            "<none>".to_string()
        } else {
            tailnet_suffixes.join(", ")
        }
    );
    println!("Using extra Tailscale direct domains:");
    println!(
        "  {}",
        if direct_domains.is_empty() {
            "<none>".to_string()
        } else {
            direct_domains.join(", ")
        }
    );
    println!("Using extra Tailscale route-exclude CIDRs:");
    println!(
        "  {}",
        if direct_cidrs.is_empty() {
            "<none>".to_string()
        } else {
            direct_cidrs.join(", ")
        }
    );
    println!("Using Clash mode:");
    println!("  {}", mode.as_str());
    println!("Using sniffer preset:");
    println!(
        "  {}",
        match sniffer_preset {
            SnifferPreset::Off => "off",
            SnifferPreset::Tun => "tun",
        }
    );

    let merge_args = MergeArgs {
        template: None,
        base_config: None,
        subscriptions_file: None,
        subscriptions: vec![subscription],
        output: None,
        mode,
        sniffer_preset,
        sync_to_clash_verge: true,
        sync_to_clash_verge_sources: true,
        stdout: false,
        dev_rules: true,
        dev_rules_via: DEFAULT_DEV_RULE_VIA.to_string(),
        dev_rules_show: false,
        use_last: false,
        subscription_ua: None,
        subscription_allow_base64: false,
        external_controller_url: None,
        external_controller_port: None,
        external_controller_secret: None,
        fake_ip_filter_add: Vec::new(),
        fake_ip_filter_mode: None,
        k8s_cidr_exclude: Vec::new(),
        route_exclude_address_add: direct_cidrs,
        fake_ip_bypass: Vec::new(),
        dry_run: args.dry_run,
        tailscale_compatible: !args.no_tailscale_compatible,
        tailscale_tailnet_suffixes: tailnet_suffixes,
        tailscale_direct_domains: direct_domains,
    };

    run_merge(merge_args).await
}

async fn run_runtime(args: RuntimeArgs) -> anyhow::Result<()> {
    let paths = AppPaths::new()?;
    paths.ensure_runtime_dirs().await?;

    match args.command {
        RuntimeCommand::Status(args) => run_runtime_status(&paths, &args).await,
        RuntimeCommand::Reload => run_runtime_reload(&paths).await,
        RuntimeCommand::Mode(args) => run_runtime_mode(&paths, args.mode).await,
    }
}

async fn run_runtime_status(paths: &AppPaths, args: &DoctorArgs) -> anyhow::Result<()> {
    let runtime_paths = existing_runtime_paths(paths).await;
    let runtime_summaries = load_runtime_summaries(&runtime_paths).await;

    println!("mihomo-cli runtime status");
    println!();

    if runtime_summaries.is_empty() {
        println!("Runtime files:");
        println!("  status: no local runtime files detected");
        return Ok(());
    }

    println!("Runtime files:");
    for summary in &runtime_summaries {
        print_runtime_summary(summary);
    }

    if runtime_summaries.len() >= 2 {
        let primary = &runtime_summaries[0];
        let aligned = runtime_summaries[1..]
            .iter()
            .all(|candidate| runtime_summary_core_eq(primary, candidate));
        println!(
            "  file-state-aligned: {}",
            if aligned { "yes" } else { "no" }
        );
    }

    if let Some(source_summary) = load_merge_profile_summary(paths).await {
        println!("Clash Verge source profile:");
        print_runtime_summary(&source_summary);
    }

    println!();
    if let Some(controller) = runtime_summaries
        .iter()
        .find_map(|summary| summary.controller.clone())
    {
        print_controller_summary(&controller, args);
    } else {
        println!("Controller:");
        println!("  status: unavailable (no controller settings found in local config)");
    }

    Ok(())
}

async fn run_runtime_reload(paths: &AppPaths) -> anyhow::Result<()> {
    let runtime_paths = existing_runtime_paths(paths).await;
    let primary_path = preferred_runtime_path(&runtime_paths)
        .ok_or_else(|| anyhow!("no local Clash Verge runtime config was detected"))?;
    let cfg = load_runtime_config(primary_path).await?;

    reload_clash_verge_runtime(&cfg, &runtime_paths).await?;
    println!("runtime reload completed from {}", primary_path.display());
    Ok(())
}

async fn run_runtime_mode(paths: &AppPaths, mode: ConfigMode) -> anyhow::Result<()> {
    let runtime_paths = existing_runtime_paths(paths).await;
    let primary_path = preferred_runtime_path(&runtime_paths)
        .ok_or_else(|| anyhow!("no local Clash Verge runtime config was detected"))?
        .clone();

    let mut reload_cfg: Option<mihomo_core::ClashConfig> = None;
    for path in &runtime_paths {
        let mut cfg = load_runtime_config(path).await?;
        apply_mode_override(&mut cfg, mode);
        fs::write(path, cfg.to_yaml_string()?)
            .await
            .with_context(|| format!("failed to write runtime mode to {}", path.display()))?;
        println!("updated runtime mode in {}", path.display());
        if *path == primary_path {
            reload_cfg = Some(cfg);
        }
    }

    if let Some(merge_path) = paths.detected_clash_verge_profile_merge_path() {
        if fs::try_exists(&merge_path).await.unwrap_or(false) {
            sync_merge_profile_mode(&merge_path, mode).await?;
        }
    }

    let reload_cfg = reload_cfg.ok_or_else(|| {
        anyhow!(
            "failed to prepare reload config from preferred runtime path {}",
            primary_path.display()
        )
    })?;
    reload_clash_verge_runtime(&reload_cfg, &runtime_paths).await?;
    println!("runtime mode is now {}", mode.as_str());
    Ok(())
}

async fn detect_current_clash_verge_subscription_url(paths: &AppPaths) -> anyhow::Result<String> {
    let profiles_path = paths
        .detected_clash_verge_profiles_path()
        .ok_or_else(|| anyhow!("could not detect Clash Verge profiles.yaml path"))?;

    let raw = fs::read_to_string(&profiles_path)
        .await
        .with_context(|| format!("failed to read {}", profiles_path.display()))?;
    let profiles: ClashVergeProfiles = serde_yaml::from_str(&raw)
        .with_context(|| format!("failed to parse {}", profiles_path.display()))?;

    extract_current_clash_verge_subscription_url(&profiles).ok_or_else(|| {
        anyhow!(
            "no current Clash Verge remote subscription URL found in {}",
            profiles_path.display()
        )
    })
}

fn extract_current_clash_verge_subscription_url(profiles: &ClashVergeProfiles) -> Option<String> {
    let current = profiles.current.as_ref()?;
    profiles
        .items
        .iter()
        .find(|item| item.uid.as_deref() == Some(current.as_str()))
        .and_then(|item| item.url.as_ref())
        .map(|url| url.trim().to_string())
        .filter(|url| !url.is_empty())
}

fn env_list(name: &str) -> Vec<String> {
    std::env::var(name)
        .ok()
        .map(|raw| split_list_items(&raw))
        .unwrap_or_default()
}

fn split_list_items(raw: &str) -> Vec<String> {
    raw.split(|ch: char| ch == ',' || ch.is_whitespace())
        .filter(|item| !item.is_empty())
        .map(|item| item.to_string())
        .collect()
}

fn merge_string_lists(
    primary: Vec<String>,
    secondary: Vec<String>,
    tertiary: Vec<String>,
    explicit: Vec<String>,
) -> Vec<String> {
    let mut merged = Vec::new();
    extend_unique(&mut merged, primary);
    extend_unique(&mut merged, secondary);
    extend_unique(&mut merged, tertiary);
    extend_unique(&mut merged, explicit);
    merged
}

fn extend_unique(dest: &mut Vec<String>, values: Vec<String>) {
    for value in values {
        if !dest.iter().any(|existing| existing == &value) {
            dest.push(value);
        }
    }
}

fn config_mode_from_env(name: &str) -> Option<ConfigMode> {
    match std::env::var(name)
        .ok()?
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "rule" => Some(ConfigMode::Rule),
        "global" => Some(ConfigMode::Global),
        "direct" => Some(ConfigMode::Direct),
        other => {
            warn!(env = %name, value = %other, "ignoring invalid config mode from environment");
            None
        }
    }
}

fn sniffer_preset_from_env(name: &str) -> Option<SnifferPreset> {
    match std::env::var(name)
        .ok()?
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "tun" => Some(SnifferPreset::Tun),
        "off" => Some(SnifferPreset::Off),
        other => {
            warn!(env = %name, value = %other, "ignoring invalid sniffer preset from environment");
            None
        }
    }
}

fn derive_tailnet_suffixes_from_managed(managed: &ManagedTailscaleCompat) -> Vec<String> {
    managed
        .fake_ip_filter
        .iter()
        .filter_map(|entry| entry.strip_prefix("+.tail."))
        .map(|suffix| suffix.to_string())
        .collect()
}

fn derive_direct_domains_from_managed(managed: &ManagedTailscaleCompat) -> Vec<String> {
    let mut domains = Vec::new();

    for rule in &managed.rules {
        if TAILSCALE_BASE_DIRECT_RULES.contains(&rule.as_str()) {
            continue;
        }

        let mut parts = rule.splitn(3, ',');
        let Some(kind) = parts.next() else {
            continue;
        };
        let Some(target) = parts.next() else {
            continue;
        };
        let Some(via) = parts.next() else {
            continue;
        };
        if !via.eq_ignore_ascii_case("DIRECT") {
            continue;
        }

        match kind {
            "DOMAIN" => {
                if !target.is_empty() {
                    domains.push(target.to_string());
                }
            }
            "DOMAIN-SUFFIX" => {
                if !target.starts_with("tail.")
                    && !target.eq_ignore_ascii_case("tailscale.com")
                    && !target.eq_ignore_ascii_case("ts.net")
                {
                    domains.push(format!("+.{}", target));
                }
            }
            _ => {}
        }
    }

    let mut deduped = Vec::new();
    extend_unique(&mut deduped, domains);
    deduped
}

fn derive_extra_route_excludes_from_managed(managed: &ManagedTailscaleCompat) -> Vec<String> {
    managed
        .route_exclude_address
        .iter()
        .filter(|entry| !TAILSCALE_ROUTE_EXCLUDES.contains(&entry.as_str()))
        .cloned()
        .collect()
}

#[derive(Debug, Clone)]
struct RuntimeSummary {
    path: PathBuf,
    mode: Option<String>,
    tun_enabled: Option<bool>,
    tun_auto_route: Option<bool>,
    sniffer_enabled: bool,
    enhanced_mode: Option<String>,
    fake_ip_range: Option<String>,
    route_excludes: Vec<String>,
    rules_count: usize,
    controller: Option<ControllerEndpoint>,
}

#[derive(Debug, Clone)]
struct ControllerEndpoint {
    host: Option<String>,
    port: Option<u16>,
    secret: Option<String>,
    unix_socket: Option<String>,
}

#[derive(Debug, Clone)]
struct ConnectionRecord {
    host: Option<String>,
    inbound_name: Option<String>,
    source_ip: Option<String>,
    chains: Vec<String>,
}

#[derive(Debug, Clone)]
struct ProxyEntry {
    kind: &'static str,
    host: String,
    port: String,
}

async fn run_doctor(args: DoctorArgs) -> anyhow::Result<()> {
    let paths = AppPaths::new()?;
    paths.ensure_runtime_dirs().await?;

    let runtime_paths = paths.detected_clash_verge_runtime_config_paths();
    let runtime_summaries = load_runtime_summaries(&runtime_paths).await;

    println!("mihomo-cli doctor");
    println!();

    if runtime_summaries.is_empty() {
        println!("Clash Verge runtime files:");
        println!("  status: no local runtime files detected");
    } else {
        println!("Clash Verge runtime files:");
        for summary in &runtime_summaries {
            print_runtime_summary(summary);
        }

        if runtime_summaries.len() >= 2 {
            let primary = &runtime_summaries[0];
            let aligned = runtime_summaries[1..]
                .iter()
                .all(|candidate| runtime_summary_core_eq(primary, candidate));
            println!(
                "  file-state-aligned: {}",
                if aligned { "yes" } else { "no" }
            );
        }
    }

    println!();
    print_system_proxy_summary();
    println!();
    print_tailscale_summary();
    println!();

    if let Some(controller) = runtime_summaries
        .iter()
        .find_map(|summary| summary.controller.clone())
    {
        print_controller_summary(&controller, &args);
    } else {
        println!("Controller:");
        println!("  status: unavailable (no controller settings found in local config)");
    }

    Ok(())
}

async fn load_runtime_summaries(paths: &[PathBuf]) -> Vec<RuntimeSummary> {
    let mut summaries = Vec::new();

    for path in paths {
        let Ok(true) = fs::try_exists(path).await else {
            continue;
        };
        let Ok(raw) = fs::read_to_string(path).await else {
            continue;
        };
        let Ok(cfg) = mihomo_core::ClashConfig::from_yaml_str(&raw) else {
            continue;
        };
        summaries.push(runtime_summary_from_config(path, &cfg));
    }

    summaries
}

async fn load_runtime_config(path: &Path) -> anyhow::Result<mihomo_core::ClashConfig> {
    let raw = fs::read_to_string(path)
        .await
        .with_context(|| format!("failed to read {}", path.display()))?;
    mihomo_core::ClashConfig::from_yaml_str(&raw)
        .with_context(|| format!("failed to parse {}", path.display()))
}

async fn existing_runtime_paths(paths: &AppPaths) -> Vec<PathBuf> {
    let mut existing = Vec::new();
    for path in paths.detected_clash_verge_runtime_config_paths() {
        if fs::try_exists(&path).await.unwrap_or(false) {
            existing.push(path);
        }
    }
    existing
}

fn preferred_runtime_path(paths: &[PathBuf]) -> Option<&PathBuf> {
    paths
        .iter()
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name == "clash-verge.yaml")
        })
        .or_else(|| {
            paths.iter().find(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name == "config.yaml")
            })
        })
        .or_else(|| paths.first())
}

fn runtime_summary_from_config(path: &Path, cfg: &mihomo_core::ClashConfig) -> RuntimeSummary {
    let mode = cfg
        .extra
        .get("mode")
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned);

    let tun_map = cfg.extra.get("tun").and_then(|value| value.as_mapping());
    let tun_enabled = tun_map.and_then(|map| bool_from_mapping(map, "enable"));
    let tun_auto_route = tun_map.and_then(|map| bool_from_mapping(map, "auto-route"));
    let route_excludes = tun_map
        .map(|map| string_list_from_mapping(map, "route-exclude-address"))
        .unwrap_or_default();

    let dns_map = cfg.extra.get("dns").and_then(|value| value.as_mapping());
    let enhanced_mode = dns_map.and_then(|map| string_from_mapping(map, "enhanced-mode"));
    let fake_ip_range = dns_map.and_then(|map| string_from_mapping(map, "fake-ip-range"));

    let sniffer_enabled = cfg.extra.contains_key("sniffer");

    RuntimeSummary {
        path: path.to_path_buf(),
        mode,
        tun_enabled,
        tun_auto_route,
        sniffer_enabled,
        enhanced_mode,
        fake_ip_range,
        route_excludes,
        rules_count: cfg.rules.len(),
        controller: parse_controller_endpoint(cfg),
    }
}

async fn load_merge_profile_summary(paths: &AppPaths) -> Option<RuntimeSummary> {
    let merge_path = paths.detected_clash_verge_profile_merge_path()?;
    if !fs::try_exists(&merge_path).await.unwrap_or(false) {
        return None;
    }
    let cfg = load_runtime_config(&merge_path).await.ok()?;
    Some(runtime_summary_from_config(&merge_path, &cfg))
}

async fn sync_merge_profile_mode(path: &Path, mode: ConfigMode) -> anyhow::Result<()> {
    let raw = fs::read_to_string(path)
        .await
        .with_context(|| format!("failed to read {}", path.display()))?;
    let mut doc: serde_yaml::Value = serde_yaml::from_str(&raw)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    ensure_root_mapping(&mut doc).insert(
        serde_yaml::Value::String("mode".to_string()),
        serde_yaml::Value::String(mode.as_str().to_string()),
    );
    fs::write(path, serde_yaml::to_string(&doc)?)
        .await
        .with_context(|| format!("failed to write {}", path.display()))?;
    println!("updated source profile mode in {}", path.display());
    Ok(())
}

fn parse_controller_endpoint(cfg: &mihomo_core::ClashConfig) -> Option<ControllerEndpoint> {
    let http = cfg
        .extra
        .get("external-controller")
        .and_then(|value| value.as_str())
        .and_then(parse_host_port);

    let unix_socket = cfg
        .extra
        .get("external-controller-unix")
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned);

    let secret = cfg
        .extra
        .get("secret")
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned);

    if http.is_none() && unix_socket.is_none() {
        return None;
    }

    Some(ControllerEndpoint {
        host: http.as_ref().map(|(host, _)| host.clone()),
        port: http.map(|(_, port)| port),
        secret,
        unix_socket,
    })
}

fn print_runtime_summary(summary: &RuntimeSummary) {
    println!("  path: {}", summary.path.display());
    println!(
        "    mode={}, tun={}, auto-route={}, sniffer={}",
        summary.mode.as_deref().unwrap_or("<unset>"),
        format_bool_opt(summary.tun_enabled),
        format_bool_opt(summary.tun_auto_route),
        if summary.sniffer_enabled { "on" } else { "off" }
    );
    println!(
        "    dns.enhanced-mode={}, fake-ip-range={}, rules={}",
        summary.enhanced_mode.as_deref().unwrap_or("<unset>"),
        summary.fake_ip_range.as_deref().unwrap_or("<unset>"),
        summary.rules_count
    );
    println!(
        "    route-excludes={}",
        if summary.route_excludes.is_empty() {
            "<none>".to_string()
        } else {
            summary.route_excludes.join(", ")
        }
    );
    if let Some(controller) = summary.controller.as_ref() {
        let http = match (&controller.host, controller.port) {
            (Some(host), Some(port)) => format!("{host}:{port}"),
            _ => "<unset>".to_string(),
        };
        println!(
            "    controller=http:{} unix:{}",
            http,
            controller.unix_socket.as_deref().unwrap_or("<unset>")
        );
    }
}

fn runtime_summary_core_eq(left: &RuntimeSummary, right: &RuntimeSummary) -> bool {
    left.mode == right.mode
        && left.tun_enabled == right.tun_enabled
        && left.tun_auto_route == right.tun_auto_route
        && left.sniffer_enabled == right.sniffer_enabled
        && left.enhanced_mode == right.enhanced_mode
        && left.fake_ip_range == right.fake_ip_range
        && left.route_excludes == right.route_excludes
        && left.rules_count == right.rules_count
}

fn print_system_proxy_summary() {
    println!("System proxy:");

    if !cfg!(target_os = "macos") {
        println!("  status: unsupported on this platform");
        return;
    }

    match std::process::Command::new("scutil").arg("--proxy").output() {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let proxies = parse_scutil_proxy(&stdout);
            if proxies.is_empty() {
                println!("  status: scutil reports no enabled system proxies");
            } else {
                println!("  status: enabled");
                for proxy in proxies {
                    println!("  {} -> {}:{}", proxy.kind, proxy.host, proxy.port);
                }
            }
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            println!(
                "  status: unavailable ({})",
                trimmed_single_line(&stderr).unwrap_or_else(|| "scutil failed".to_string())
            );
        }
        Err(err) => {
            println!("  status: unavailable ({err})");
        }
    }
}

fn parse_scutil_proxy(output: &str) -> Vec<ProxyEntry> {
    let mut http_enable = false;
    let mut https_enable = false;
    let mut socks_enable = false;
    let mut http_proxy = String::new();
    let mut https_proxy = String::new();
    let mut socks_proxy = String::new();
    let mut http_port = String::new();
    let mut https_port = String::new();
    let mut socks_port = String::new();

    for line in output.lines() {
        let mut parts = line.splitn(2, ':');
        let Some(key) = parts.next().map(str::trim) else {
            continue;
        };
        let Some(value) = parts.next().map(str::trim) else {
            continue;
        };

        match key {
            "HTTPEnable" => http_enable = value == "1",
            "HTTPProxy" => http_proxy = value.to_string(),
            "HTTPPort" => http_port = value.to_string(),
            "HTTPSEnable" => https_enable = value == "1",
            "HTTPSProxy" => https_proxy = value.to_string(),
            "HTTPSPort" => https_port = value.to_string(),
            "SOCKSEnable" => socks_enable = value == "1",
            "SOCKSProxy" => socks_proxy = value.to_string(),
            "SOCKSPort" => socks_port = value.to_string(),
            _ => {}
        }
    }

    let mut proxies = Vec::new();
    if http_enable && !http_proxy.is_empty() {
        proxies.push(ProxyEntry {
            kind: "HTTP",
            host: http_proxy,
            port: http_port,
        });
    }
    if https_enable && !https_proxy.is_empty() {
        proxies.push(ProxyEntry {
            kind: "HTTPS",
            host: https_proxy,
            port: https_port,
        });
    }
    if socks_enable && !socks_proxy.is_empty() {
        proxies.push(ProxyEntry {
            kind: "SOCKS",
            host: socks_proxy,
            port: socks_port,
        });
    }
    proxies
}

fn print_tailscale_summary() {
    println!("Tailscale:");

    let tailscale_candidates = [
        "/Applications/Tailscale.app/Contents/MacOS/Tailscale",
        "tailscale",
    ];

    let mut last_error: Option<String> = None;
    for candidate in tailscale_candidates {
        let output = std::process::Command::new(candidate)
            .args(["status", "--json"])
            .output();
        match output {
            Ok(output) if output.status.success() => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                match serde_json::from_str::<serde_json::Value>(&stdout) {
                    Ok(json) => {
                        let backend_state = json
                            .get("BackendState")
                            .and_then(|value| value.as_str())
                            .unwrap_or("<unknown>");
                        let health_count = json
                            .get("Health")
                            .and_then(|value| value.as_array())
                            .map(|items| items.len())
                            .unwrap_or(0);
                        println!(
                            "  status: backend={}, health-warnings={}",
                            backend_state, health_count
                        );
                        if let Some(health) = json.get("Health").and_then(|value| value.as_array())
                        {
                            for item in health.iter().take(3) {
                                if let Some(message) = item.as_str() {
                                    println!("  warning: {}", message);
                                }
                            }
                        }
                        return;
                    }
                    Err(err) => {
                        last_error = Some(format!("invalid JSON from {}: {}", candidate, err));
                    }
                }
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let stdout = String::from_utf8_lossy(&output.stdout);
                last_error = trimmed_single_line(&stderr)
                    .or_else(|| trimmed_single_line(&stdout))
                    .or_else(|| Some(format!("{} exited with {}", candidate, output.status)));
            }
            Err(err) => {
                last_error = Some(format!("{} unavailable: {}", candidate, err));
            }
        }
    }

    println!(
        "  status: unavailable ({})",
        last_error.unwrap_or_else(|| "tailscale CLI probe failed".to_string())
    );
}

fn print_controller_summary(controller: &ControllerEndpoint, args: &DoctorArgs) {
    println!("Controller:");

    match (&controller.host, controller.port) {
        (Some(host), Some(port)) => println!("  configured-http: {}:{}", host, port),
        _ => println!("  configured-http: <unset>"),
    }
    println!(
        "  configured-unix: {}",
        controller.unix_socket.as_deref().unwrap_or("<unset>")
    );

    if let Some(probe) = probe_controller_http(controller) {
        println!("  live-api: http");
        if let Some(version) = probe.version_line {
            println!("  version: {}", version);
        }
        print_connection_summary(probe.connections_json.as_deref(), args);
        return;
    }

    if let Some(probe) = probe_controller_unix(controller) {
        println!("  live-api: unix-socket");
        if let Some(version) = probe.version_line {
            println!("  version: {}", version);
        }
        print_connection_summary(probe.connections_json.as_deref(), args);
        return;
    }

    println!("  live-api: unavailable");
    println!("  note: controller could not be reached over HTTP or unix socket");
}

struct ControllerProbe {
    version_line: Option<String>,
    connections_json: Option<String>,
}

fn probe_controller_http(controller: &ControllerEndpoint) -> Option<ControllerProbe> {
    let host = controller.host.as_ref()?;
    let port = controller.port?;
    let base = format!("http://{}:{}", normalize_controller_host(host), port);
    let version_text = curl_controller_get(controller, &(base.clone() + "/version"))?;
    let connections_text = curl_controller_get(controller, &(base + "/connections"));

    Some(ControllerProbe {
        version_line: trimmed_single_line(&version_text),
        connections_json: connections_text,
    })
}

fn probe_controller_unix(controller: &ControllerEndpoint) -> Option<ControllerProbe> {
    let socket = controller.unix_socket.as_ref()?;
    if !Path::new(socket).exists() {
        return None;
    }

    let version_text =
        curl_controller_get_with_socket(controller, socket, "http://localhost/version")?;
    if version_text.trim().is_empty() {
        return None;
    }

    let connections_text =
        curl_controller_get_with_socket(controller, socket, "http://localhost/connections");

    Some(ControllerProbe {
        version_line: trimmed_single_line(&version_text),
        connections_json: connections_text,
    })
}

fn curl_controller_get(controller: &ControllerEndpoint, url: &str) -> Option<String> {
    let mut command = std::process::Command::new("curl");
    command.args(["-s", "-f"]);
    if let Some(secret) = controller.secret.as_ref() {
        if !secret.is_empty() {
            command.args(["-H", &format!("Authorization: Bearer {}", secret)]);
        }
    }
    command.arg(url);
    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).to_string();
    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}

fn curl_controller_get_with_socket(
    controller: &ControllerEndpoint,
    socket: &str,
    url: &str,
) -> Option<String> {
    let mut command = std::process::Command::new("curl");
    command.args(["--unix-socket", socket, "-s", "-f"]);
    if let Some(secret) = controller.secret.as_ref() {
        if !secret.is_empty() {
            command.args(["-H", &format!("Authorization: Bearer {}", secret)]);
        }
    }
    command.arg(url);
    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).to_string();
    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}

fn print_connection_summary(connections_json: Option<&str>, args: &DoctorArgs) {
    if !args.show_connections {
        println!("  connections: skipped");
        return;
    }

    let Some(raw) = connections_json else {
        println!("  connections: unavailable");
        return;
    };

    let Ok(json) = serde_json::from_str::<serde_json::Value>(raw) else {
        println!("  connections: unavailable (invalid JSON)");
        return;
    };

    let records = extract_connection_records(&json);
    if records.is_empty() {
        println!("  connections: no active connections reported");
        return;
    }

    println!("  active-connections: {}", records.len());

    let focus_domains = if args.focus_domains.is_empty() {
        vec![
            "chat.openai.com".to_string(),
            "chatgpt.com".to_string(),
            "ab.chatgpt.com".to_string(),
            "api.anthropic.com".to_string(),
        ]
    } else {
        args.focus_domains.clone()
    };

    let focused: Vec<&ConnectionRecord> = records
        .iter()
        .filter(|record| connection_matches_focus(record, &focus_domains))
        .collect();

    if focused.is_empty() {
        println!("  focus-matches: none");
        return;
    }

    let path_hint = infer_connection_path(&focused);
    println!("  focus-matches: {}", focused.len());
    println!("  likely-path: {}", path_hint);
    for record in focused.iter().take(6) {
        println!(
            "  connection: host={} inbound={} source={} chains={}",
            record.host.as_deref().unwrap_or("<unknown>"),
            record.inbound_name.as_deref().unwrap_or("<unknown>"),
            record.source_ip.as_deref().unwrap_or("<unknown>"),
            if record.chains.is_empty() {
                "<none>".to_string()
            } else {
                record.chains.join(">")
            }
        );
    }
}

fn extract_connection_records(json: &serde_json::Value) -> Vec<ConnectionRecord> {
    let Some(items) = json
        .get("connections")
        .and_then(|value| value.as_array())
        .or_else(|| json.as_array())
    else {
        return Vec::new();
    };

    items.iter().map(connection_record_from_json).collect()
}

fn connection_record_from_json(value: &serde_json::Value) -> ConnectionRecord {
    let host = json_string_path(value, &["metadata", "host"])
        .or_else(|| json_string_path(value, &["metadata", "destinationIP"]))
        .or_else(|| json_string_path(value, &["host"]));
    let inbound_name = json_string_path(value, &["metadata", "inboundName"])
        .or_else(|| json_string_path(value, &["inboundName"]));
    let source_ip = json_string_path(value, &["metadata", "sourceIP"])
        .or_else(|| json_string_path(value, &["sourceIP"]));
    let chains = json_string_list_path(value, &["chains"])
        .or_else(|| json_string_list_path(value, &["metadata", "chains"]))
        .unwrap_or_default();

    ConnectionRecord {
        host,
        inbound_name,
        source_ip,
        chains,
    }
}

fn connection_matches_focus(record: &ConnectionRecord, focus_domains: &[String]) -> bool {
    let Some(host) = record.host.as_ref() else {
        return false;
    };
    focus_domains.iter().any(|focus| host.contains(focus))
}

fn infer_connection_path(records: &[&ConnectionRecord]) -> &'static str {
    if records.iter().any(|record| {
        record.source_ip.as_deref() == Some("127.0.0.1")
            || record
                .inbound_name
                .as_deref()
                .map(|name| name.contains("MIXED"))
                == Some(true)
    }) {
        "local proxy / DEFAULT-MIXED"
    } else if records.iter().any(|record| {
        record
            .inbound_name
            .as_deref()
            .map(|name| name.to_ascii_uppercase().contains("TUN"))
            == Some(true)
    }) {
        "transparent TUN"
    } else {
        "unknown"
    }
}

fn json_string_path(value: &serde_json::Value, path: &[&str]) -> Option<String> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    current.as_str().map(ToOwned::to_owned)
}

fn json_string_list_path(value: &serde_json::Value, path: &[&str]) -> Option<Vec<String>> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    Some(
        current
            .as_array()?
            .iter()
            .filter_map(|value| value.as_str().map(ToOwned::to_owned))
            .collect(),
    )
}

fn string_from_mapping(map: &serde_yaml::Mapping, key: &str) -> Option<String> {
    map.get(Value::String(key.to_string()))
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned)
}

fn bool_from_mapping(map: &serde_yaml::Mapping, key: &str) -> Option<bool> {
    map.get(Value::String(key.to_string()))
        .and_then(|value| value.as_bool())
}

fn string_list_from_mapping(map: &serde_yaml::Mapping, key: &str) -> Vec<String> {
    map.get(Value::String(key.to_string()))
        .and_then(|value| value.as_sequence())
        .map(|seq| {
            seq.iter()
                .filter_map(|value| value.as_str().map(ToOwned::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

fn format_bool_opt(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "on",
        Some(false) => "off",
        None => "<unset>",
    }
}

fn trimmed_single_line(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.lines().next().unwrap_or(trimmed).trim().to_string())
    }
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

    ensure_default_template(&paths).await?;

    let template_path = args
        .template
        .as_ref()
        .map(|p| resolve_template_path(&paths, p))
        .unwrap_or_else(|| paths.default_template_path());

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

    // Inject manually-managed proxies (e.g. a private trojan server) before applying base-config,
    // so that base-config group rebuild sees all proxy names.
    if !app_cfg.manual_servers.is_empty() {
        let added = inject_manual_servers(&mut merged, &app_cfg).await?;
        if added > 0 {
            info!(added = added, "injected manual server proxies");
        }
    }

    if let Some(base) = base_config.as_ref() {
        merged = mihomo_core::merge::apply_base_config(merged, base);
    }

    apply_mode_override(&mut merged, args.mode);
    apply_sniffer_preset(&mut merged, args.sniffer_preset);

    if let Some(previous) = app_cfg.managed_tailscale_compat.as_ref() {
        remove_tailscale_managed_items(&mut merged, previous);
    }

    let mut dev_rules_listing = None;
    let mut summary_dev_via: Option<String> = None;
    let mut summary_dev_added: usize = 0;
    if args.dev_rules || args.dev_rules_show {
        let resolved_via =
            resolve_dev_rules_via(&args.dev_rules_via, DEFAULT_DEV_RULE_VIA, &merged);
        if resolved_via != args.dev_rules_via && args.dev_rules {
            warn!(
                requested = %args.dev_rules_via,
                using = %resolved_via,
                "--dev-rules-via not found in config; using fallback"
            );
        }

        let list = build_dev_rules(&resolved_via);
        if args.dev_rules {
            let mut combined = list.clone();
            combined.extend(merged.rules.into_iter());
            merged.rules = combined;
            summary_dev_via = Some(resolved_via.clone());
            summary_dev_added = list.len();
        } else {
            // even if not applied, keep via for summary visibility
            summary_dev_via = Some(resolved_via.clone());
        }
        dev_rules_listing = Some(list);
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

    // Apply external-controller overrides if provided
    if args.external_controller_url.is_some()
        || args.external_controller_port.is_some()
        || args.external_controller_secret.is_some()
    {
        use serde_yaml::Value;

        // Read any existing external-controller value like "host:port"
        let mut existing_host: Option<String> = None;
        let mut existing_port: Option<u16> = None;
        if let Some(Value::String(s)) = merged.extra.get("external-controller") {
            if let Some((h, p)) = parse_host_port(s) {
                existing_host = Some(h);
                existing_port = Some(p);
            }
        }

        let host = args
            .external_controller_url
            .clone()
            .or(existing_host)
            .unwrap_or_else(|| "127.0.0.1".to_string());
        let port = args
            .external_controller_port
            .or(existing_port)
            .unwrap_or(9090);
        merged.extra.insert(
            "external-controller".to_string(),
            Value::String(format!("{}:{}", host, port)),
        );

        if let Some(secret) = args.external_controller_secret.as_ref() {
            merged
                .extra
                .insert("secret".to_string(), Value::String(secret.clone()));
        }
    }

    // Append fake-ip bypass entries: combine new clearer option with legacy flag
    let mut bypass_entries: Vec<String> = Vec::new();
    bypass_entries.extend(args.fake_ip_bypass.iter().cloned());
    bypass_entries.extend(args.fake_ip_filter_add.iter().cloned());
    if !bypass_entries.is_empty() {
        use serde_yaml::{Mapping, Value};
        // Ensure dns mapping exists
        let dns_value = merged
            .extra
            .entry("dns".to_string())
            .or_insert_with(|| Value::Mapping(Mapping::new()));
        if let Value::Mapping(dns_map) = dns_value {
            // Ensure fake-ip-filter sequence exists
            let key = Value::String("fake-ip-filter".to_string());
            let filter_seq = dns_map
                .entry(key)
                .or_insert_with(|| Value::Sequence(Vec::new()));
            if let Value::Sequence(seq) = filter_seq {
                for item in bypass_entries {
                    seq.push(Value::String(item));
                }
            }
            // Force blacklist mode when user requests bypass entries
            let mode_key = Value::String("fake-ip-filter-mode".to_string());
            let current_mode = dns_map.get(&mode_key).and_then(|v| v.as_str());
            let desired = "blacklist";
            if current_mode.map(|s| s.eq_ignore_ascii_case(desired)) != Some(true) {
                if let Some(cm) = current_mode {
                    warn!(current = %cm, "overriding fake-ip-filter-mode to 'blacklist' for --fake-ip-bypass");
                }
                dns_map.insert(mode_key, Value::String(desired.to_string()));
            }
        }
    }

    // Apply fake-ip-filter-mode if provided explicitly (advanced)
    if let Some(mode) = args.fake_ip_filter_mode.as_ref() {
        let m = mode.to_ascii_lowercase();
        if m == "blacklist" || m == "whitelist" {
            use serde_yaml::{Mapping, Value};
            let dns_value = merged
                .extra
                .entry("dns".to_string())
                .or_insert_with(|| Value::Mapping(Mapping::new()));
            if let Value::Mapping(dns_map) = dns_value {
                // If user also used --fake-ip-bypass and asks for whitelist, warn and keep blacklist
                let requested_whitelist = m == "whitelist";
                let used_bypass = !args.fake_ip_bypass.is_empty();
                if requested_whitelist && used_bypass {
                    warn!("--fake-ip-bypass works with blacklist mode; keeping 'blacklist' instead of requested 'whitelist'");
                } else {
                    dns_map.insert(
                        Value::String("fake-ip-filter-mode".to_string()),
                        Value::String(m),
                    );
                }
            }
        } else {
            warn!(
                value = %mode,
                "invalid --fake-ip-filter-mode (expected 'blacklist' or 'whitelist')"
            );
        }
    }

    // Ensure Kubernetes cluster DNS names are not forced into fake-ip.
    //
    // When tun + dns-hijack is enabled, in-cluster lookups like
    // *.svc.cluster.local may be intercepted and incorrectly resolved into the
    // fake-ip range (198.18.0.0/16), breaking Kubernetes service discovery.
    //
    // Keep this minimal and only apply in fake-ip mode when filter mode is not
    // whitelist.
    {
        use serde_yaml::{Mapping, Value};

        let dns_value = merged
            .extra
            .entry("dns".to_string())
            .or_insert_with(|| Value::Mapping(Mapping::new()));

        if let Value::Mapping(dns_map) = dns_value {
            let enhanced_key = Value::String("enhanced-mode".to_string());
            let mode_key = Value::String("fake-ip-filter-mode".to_string());
            let filter_key = Value::String("fake-ip-filter".to_string());

            let enhanced = dns_map
                .get(&enhanced_key)
                .and_then(|v| v.as_str())
                .unwrap_or("");

            if enhanced.eq_ignore_ascii_case("fake-ip") {
                let filter_mode = dns_map
                    .get(&mode_key)
                    .and_then(|v| v.as_str())
                    .unwrap_or("blacklist");

                if !filter_mode.eq_ignore_ascii_case("whitelist") {
                    // Cover both the canonical FQDNs and resolver-expanded names
                    // (when search domains are appended).
                    let wanted = ["+.cluster.local", "*.cluster.local.*"];

                    let filter_seq = dns_map
                        .entry(filter_key)
                        .or_insert_with(|| Value::Sequence(Vec::new()));

                    if let Value::Sequence(seq) = filter_seq {
                        for item in wanted {
                            let exists = seq.iter().any(|v| v.as_str() == Some(item));
                            if !exists {
                                seq.push(Value::String(item.to_string()));
                                info!(value = %item, "auto-added fake-ip bypass");
                            }
                        }
                    }
                }
            }
        }
    }

    if args.tailscale_compatible {
        apply_tailscale_compatibility(
            &mut merged,
            &args.tailscale_tailnet_suffixes,
            &args.tailscale_direct_domains,
        );
        app_cfg.managed_tailscale_compat = Some(build_tailscale_managed_state(
            &args.tailscale_tailnet_suffixes,
            &args.tailscale_direct_domains,
            &args.route_exclude_address_add,
        ));
    } else if app_cfg.managed_tailscale_compat.is_some() {
        app_cfg.managed_tailscale_compat = None;
    }

    let fallback_via = resolve_dev_rules_via(&args.dev_rules_via, DEFAULT_DEV_RULE_VIA, &merged);
    ensure_fallback_match_rule(&mut merged, &fallback_via);

    // Avoid hijacking Kubernetes pod/service CIDRs in tun mode.
    // This keeps in-cluster traffic (including DNS to kube-dns) out of the tun
    // device so service discovery remains stable.
    {
        use serde_yaml::{Mapping, Value};

        let tun_value = merged
            .extra
            .entry("tun".to_string())
            .or_insert_with(|| Value::Mapping(Mapping::new()));

        if let Value::Mapping(tun_map) = tun_value {
            let key = Value::String("route-exclude-address".to_string());
            let seq_value = tun_map
                .entry(key)
                .or_insert_with(|| Value::Sequence(Vec::new()));

            if let Value::Sequence(seq) = seq_value {
                let mut cidrs: Vec<String> =
                    vec!["10.42.0.0/16".to_string(), "10.43.0.0/16".to_string()];
                cidrs.extend(args.k8s_cidr_exclude.iter().cloned());
                cidrs.extend(args.route_exclude_address_add.iter().cloned());

                for cidr in cidrs {
                    if !cidr.contains('/') {
                        warn!(value = %cidr, "invalid CIDR for route-exclude-address addition (expected like 10.42.0.0/16)");
                        continue;
                    }
                    let exists = seq.iter().any(|v| v.as_str() == Some(cidr.as_str()));
                    if !exists {
                        seq.push(Value::String(cidr.clone()));
                        info!(value = %cidr, "auto-added tun route-exclude-address");
                    }
                }
            }
        }
    }

    // If dry-run, print a concise summary and skip writing
    if args.dry_run {
        print_merge_summary(
            &merged,
            &args,
            summary_dev_via.as_deref(),
            summary_dev_added,
            &paths,
        );
        if let Some(list) = dev_rules_listing.as_ref().filter(|_| args.dev_rules_show) {
            for rule in list {
                eprintln!("dev-rule: {}", rule);
            }
        }
        return Ok(());
    }

    let yaml = merged.to_yaml_string()?;

    let output_path = args
        .output
        .clone()
        .unwrap_or_else(|| paths.generated_clash_verge_path());

    if args.stdout {
        println!("{}", yaml);
    } else {
        ensure_parent(&output_path).await?;
        let deployer = FileDeployer {
            path: output_path.clone(),
        };
        deployer.deploy(&yaml).await.with_context(|| {
            format!("failed to write merged config to {}", output_path.display())
        })?;
        println!("merged config written to {}", output_path.display());

        if args.sync_to_clash_verge {
            let clash_verge_paths = paths.detected_clash_verge_runtime_config_paths();
            if clash_verge_paths.is_empty() {
                return Err(anyhow!(
                    "--sync-to-clash-verge requested, but no local Clash Verge runtime config was detected"
                ));
            }
            for clash_verge_path in &clash_verge_paths {
                ensure_parent(&clash_verge_path).await?;
                if clash_verge_path.exists() {
                    if let Some(backup) = backup_existing_file(&clash_verge_path).await? {
                        println!(
                            "backed up existing Clash Verge config to {}",
                            backup.display()
                        );
                    }
                }
                let deployer = FileDeployer {
                    path: clash_verge_path.clone(),
                };
                deployer.deploy(&yaml).await.with_context(|| {
                    format!(
                        "failed to sync merged config to Clash Verge runtime path {}",
                        clash_verge_path.display()
                    )
                })?;
                println!("synced config to {}", clash_verge_path.display());
            }

            if let Err(err) = reload_clash_verge_runtime(&merged, &clash_verge_paths).await {
                warn!(error = %err, "failed to auto-reload Clash Verge runtime after sync");
            }
        }

        if args.sync_to_clash_verge_sources {
            sync_clash_verge_source_configs(&paths, &merged).await?;
        }
    }

    if let Some(list) = dev_rules_listing.as_ref().filter(|_| args.dev_rules_show) {
        for rule in list {
            eprintln!("dev-rule: {}", rule);
        }
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

fn print_merge_summary(
    merged: &mihomo_core::ClashConfig,
    args: &MergeArgs,
    dev_via: Option<&str>,
    dev_added: usize,
    paths: &AppPaths,
) {
    use serde_yaml::Value;

    let proxies = merged.proxy_names().len();
    let groups = merged.proxy_group_names().len();
    let rules = merged.rules.len();

    // DNS fake-ip summary
    let mut dns_mode: Option<String> = None;
    let mut dns_filter_total: Option<usize> = None;
    let mut dns_fake_ip_range: Option<String> = None;
    if let Some(Value::Mapping(dns)) = merged.extra.get("dns") {
        if let Some(Value::String(m)) = dns.get(&Value::String("fake-ip-filter-mode".into())) {
            dns_mode = Some(m.clone());
        }
        if let Some(Value::Sequence(seq)) = dns.get(&Value::String("fake-ip-filter".into())) {
            dns_filter_total = Some(seq.len());
        }
        if let Some(Value::String(range)) = dns.get(&Value::String("fake-ip-range".into())) {
            dns_fake_ip_range = Some(range.clone());
        }
    }

    // External controller
    let mut ext_ctrl: Option<String> = None;
    if let Some(Value::String(s)) = merged.extra.get("external-controller") {
        ext_ctrl = Some(s.clone());
    }
    let secret_present = merged
        .extra
        .get("secret")
        .and_then(|v| v.as_str())
        .is_some();

    println!("dry-run summary:");
    println!(
        "- proxies: {}, groups: {}, rules: {}",
        proxies, groups, rules
    );
    println!(
        "- fake-ip: mode={}, range={}, filter+={} (requested), total={}",
        dns_mode.as_deref().unwrap_or("<none>"),
        dns_fake_ip_range.as_deref().unwrap_or("<unset>"),
        args.fake_ip_bypass.len(),
        dns_filter_total
            .map(|n| n.to_string())
            .unwrap_or_else(|| "<unknown>".into())
    );
    println!(
        "- dev-rules: enabled={}, via={}, added={}",
        if args.dev_rules { "true" } else { "false" },
        dev_via.unwrap_or("<n/a>"),
        if args.dev_rules { dev_added } else { 0 }
    );
    println!("- mode: {}", args.mode.as_str());
    println!(
        "- sniffer-preset: {}",
        match args.sniffer_preset {
            SnifferPreset::Off => "off",
            SnifferPreset::Tun => "tun",
        }
    );
    println!("- manual-servers: <see app.yaml> (not shown in dry-run summary)");
    println!(
        "- external-controller: {}, secret={}",
        ext_ctrl.unwrap_or_else(|| "<unset>".into()),
        if secret_present { "set" } else { "unset" }
    );
    let would_write = args
        .output
        .clone()
        .unwrap_or_else(|| paths.generated_clash_verge_path());
    println!(
        "- output: would write to {} (suppressed by --dry-run)",
        would_write.display()
    );
    if args.sync_to_clash_verge {
        let targets = paths.detected_clash_verge_runtime_config_paths();
        let target = if targets.is_empty() {
            "<not detected>".to_string()
        } else {
            targets
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        };
        println!("- clash-verge-sync: {}", target);
    }
    if args.sync_to_clash_verge_sources {
        let target = paths
            .detect_clash_verge_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "<not detected>".into());
        println!("- clash-verge-source-sync: {}", target);
    }
    println!(
        "- tailscale-compatible: {}",
        if args.tailscale_compatible {
            "true"
        } else {
            "false"
        }
    );
    if args.tailscale_compatible {
        println!(
            "- tailscale-tailnet-suffixes: {}",
            if args.tailscale_tailnet_suffixes.is_empty() {
                "<none>".to_string()
            } else {
                args.tailscale_tailnet_suffixes.join(",")
            }
        );
        println!(
            "- tailscale-direct-domains: {}",
            if args.tailscale_direct_domains.is_empty() {
                "<none>".to_string()
            } else {
                args.tailscale_direct_domains.join(",")
            }
        );
    }
}

fn normalize_direct_domain(domain: &str) -> Option<String> {
    let normalized = domain.trim().trim_matches('.').to_ascii_lowercase();
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

fn build_tailscale_fake_ip_bypass(
    tailnet_suffixes: &[String],
    direct_domains: &[String],
) -> Vec<String> {
    let mut items: Vec<String> = TAILSCALE_BASE_FAKE_IP_BYPASS
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    for suffix in tailnet_suffixes {
        if let Some(suffix) = normalize_direct_domain(suffix) {
            items.push(format!("+.tail.{suffix}"));
        }
    }
    for domain in direct_domains {
        if let Some(domain) = normalize_direct_domain(domain) {
            if let Some(stripped) = domain.strip_prefix("+.") {
                items.push(format!("+.{stripped}"));
            } else {
                items.push(domain);
            }
        }
    }
    items.sort();
    items.dedup();
    items
}

fn build_tailscale_direct_rules(
    tailnet_suffixes: &[String],
    direct_domains: &[String],
) -> Vec<String> {
    let mut rules: Vec<String> = TAILSCALE_BASE_DIRECT_RULES
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    for suffix in tailnet_suffixes {
        if let Some(suffix) = normalize_direct_domain(suffix) {
            rules.push(format!("DOMAIN-SUFFIX,tail.{suffix},DIRECT"));
        }
    }
    for domain in direct_domains {
        if let Some(domain) = normalize_direct_domain(domain) {
            if let Some(stripped) = domain.strip_prefix("+.") {
                rules.push(format!("DOMAIN-SUFFIX,{stripped},DIRECT"));
            } else {
                rules.push(format!("DOMAIN,{domain},DIRECT"));
            }
        }
    }
    rules.sort();
    rules.dedup();
    rules
}

fn build_tailscale_managed_state(
    tailnet_suffixes: &[String],
    direct_domains: &[String],
    route_exclude_address_add: &[String],
) -> ManagedTailscaleCompat {
    let mut route_exclude_address: Vec<String> = TAILSCALE_ROUTE_EXCLUDES
        .iter()
        .map(|cidr| (*cidr).to_string())
        .collect();
    route_exclude_address.extend(route_exclude_address_add.iter().cloned());
    route_exclude_address.sort();
    route_exclude_address.dedup();

    ManagedTailscaleCompat {
        fake_ip_filter: build_tailscale_fake_ip_bypass(tailnet_suffixes, direct_domains),
        route_exclude_address,
        rules: build_tailscale_direct_rules(tailnet_suffixes, direct_domains),
    }
}

fn remove_tailscale_managed_items(
    merged: &mut mihomo_core::ClashConfig,
    managed: &ManagedTailscaleCompat,
) {
    use serde_yaml::Value;
    use std::collections::HashSet;

    let managed_filters: HashSet<&str> =
        managed.fake_ip_filter.iter().map(String::as_str).collect();
    if let Some(filters) = merged
        .extra
        .get_mut("dns")
        .and_then(|v| v.as_mapping_mut())
        .and_then(|map| map.get_mut(Value::String("fake-ip-filter".to_string())))
        .and_then(|value| value.as_sequence_mut())
    {
        filters.retain(|item| match item.as_str() {
            Some(value) => !managed_filters.contains(value),
            None => true,
        });
    }

    let managed_route_excludes: HashSet<&str> = managed
        .route_exclude_address
        .iter()
        .map(String::as_str)
        .collect();
    if let Some(excludes) = merged
        .extra
        .get_mut("tun")
        .and_then(|v| v.as_mapping_mut())
        .and_then(|map| map.get_mut(Value::String("route-exclude-address".to_string())))
        .and_then(|value| value.as_sequence_mut())
    {
        excludes.retain(|item| match item.as_str() {
            Some(value) => !managed_route_excludes.contains(value),
            None => true,
        });
    }

    let managed_rules: HashSet<&str> = managed.rules.iter().map(String::as_str).collect();
    merged
        .rules
        .retain(|rule| !managed_rules.contains(rule.as_str()));
}

fn apply_tailscale_compatibility(
    merged: &mut mihomo_core::ClashConfig,
    tailnet_suffixes: &[String],
    direct_domains: &[String],
) {
    use serde_yaml::{Mapping, Value};
    let fake_ip_bypass = build_tailscale_fake_ip_bypass(tailnet_suffixes, direct_domains);
    let direct_rules = build_tailscale_direct_rules(tailnet_suffixes, direct_domains);

    let dns_value = merged
        .extra
        .entry("dns".to_string())
        .or_insert_with(|| Value::Mapping(Mapping::new()));

    if let Value::Mapping(dns_map) = dns_value {
        let enhanced_key = Value::String("enhanced-mode".to_string());
        let range_key = Value::String("fake-ip-range".to_string());
        let mode_key = Value::String("fake-ip-filter-mode".to_string());
        let filter_key = Value::String("fake-ip-filter".to_string());

        let enhanced = dns_map
            .get(&enhanced_key)
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if enhanced.eq_ignore_ascii_case("fake-ip") {
            dns_map.insert(mode_key, Value::String("blacklist".to_string()));

            let current_range = dns_map
                .get(&range_key)
                .and_then(|v| v.as_str())
                .map(str::to_owned);

            match current_range.as_deref() {
                Some(range) if range.starts_with("198.18.") || range.starts_with("198.19.") => {
                    dns_map.insert(range_key, Value::String(SAFE_FAKE_IP_RANGE.to_string()));
                    warn!(
                        previous = %range,
                        replacement = %SAFE_FAKE_IP_RANGE,
                        "rewrote fake-ip-range to avoid Tailscale conflict"
                    );
                }
                None => {
                    dns_map.insert(range_key, Value::String(SAFE_FAKE_IP_RANGE.to_string()));
                    info!(
                        value = %SAFE_FAKE_IP_RANGE,
                        "set default fake-ip-range compatible with Tailscale"
                    );
                }
                _ => {}
            }

            let filter_seq = dns_map
                .entry(filter_key)
                .or_insert_with(|| Value::Sequence(Vec::new()));
            if let Value::Sequence(seq) = filter_seq {
                for item in &fake_ip_bypass {
                    if !seq.iter().any(|v| v.as_str() == Some(item.as_str())) {
                        seq.push(Value::String(item.clone()));
                        info!(value = %item, "auto-added Tailscale fake-ip bypass");
                    }
                }
            }
        }
    }

    let tun_value = merged
        .extra
        .entry("tun".to_string())
        .or_insert_with(|| Value::Mapping(Mapping::new()));

    if let Value::Mapping(tun_map) = tun_value {
        let key = Value::String("route-exclude-address".to_string());
        let seq_value = tun_map
            .entry(key)
            .or_insert_with(|| Value::Sequence(Vec::new()));

        if let Value::Sequence(seq) = seq_value {
            for cidr in TAILSCALE_ROUTE_EXCLUDES {
                if !seq.iter().any(|v| v.as_str() == Some(cidr)) {
                    seq.push(Value::String(cidr.to_string()));
                    info!(value = %cidr, "auto-added Tailscale tun route exclusion");
                }
            }
        }
    }

    let mut prefixed_rules = Vec::new();
    for rule in &direct_rules {
        if !merged.rules.iter().any(|existing| existing == rule) {
            prefixed_rules.push(rule.clone());
        }
    }
    if !prefixed_rules.is_empty() {
        prefixed_rules.extend(std::mem::take(&mut merged.rules));
        merged.rules = prefixed_rules;
    }
}

fn has_fallback_rule(cfg: &mihomo_core::ClashConfig) -> bool {
    cfg.rules.iter().any(|rule| {
        let normalized = rule.trim().to_ascii_uppercase();
        normalized.starts_with("MATCH,")
            || normalized.starts_with("FINAL,")
            || normalized.starts_with("IP-CIDR,0.0.0.0/0,")
            || normalized.starts_with("IP-CIDR6,::/0,")
    })
}

fn ensure_fallback_match_rule(cfg: &mut mihomo_core::ClashConfig, via: &str) {
    if has_fallback_rule(cfg) {
        return;
    }
    cfg.rules.push(format!("MATCH,{via}"));
}

fn apply_mode_override(cfg: &mut mihomo_core::ClashConfig, mode: ConfigMode) {
    cfg.extra
        .insert("mode".to_string(), Value::String(mode.as_str().to_string()));
}

fn apply_sniffer_preset(cfg: &mut mihomo_core::ClashConfig, preset: SnifferPreset) {
    match preset {
        SnifferPreset::Off => {
            cfg.extra.shift_remove("sniffer");
        }
        SnifferPreset::Tun => {
            let sniffer = serde_yaml::from_str::<Value>(
                r#"
enable: true
force-dns-mapping: true
parse-pure-ip: true
override-destination: false
sniff:
  HTTP:
    ports: [80, 8080-8880]
    override-destination: true
  TLS:
    ports: [443, 8443]
  QUIC:
    ports: [443, 8443]
"#,
            )
            .expect("valid built-in sniffer preset");
            cfg.extra.insert("sniffer".to_string(), sniffer);
        }
    }
}

async fn sync_clash_verge_source_configs(
    paths: &AppPaths,
    merged: &mihomo_core::ClashConfig,
) -> anyhow::Result<()> {
    use serde_yaml::Value;

    let dns_path = paths
        .detected_clash_verge_dns_config_path()
        .ok_or_else(|| {
            anyhow!("--sync-to-clash-verge-sources requested, but dns_config.yaml was not detected")
        })?;
    let merge_path = paths
        .detected_clash_verge_profile_merge_path()
        .ok_or_else(|| {
            anyhow!(
                "--sync-to-clash-verge-sources requested, but profiles/Merge.yaml was not detected"
            )
        })?;

    let dns_source = fs::read_to_string(&dns_path)
        .await
        .with_context(|| format!("failed to read {}", dns_path.display()))?;
    let merge_source = fs::read_to_string(&merge_path)
        .await
        .with_context(|| format!("failed to read {}", merge_path.display()))?;

    let mut dns_doc: Value = serde_yaml::from_str(&dns_source)
        .with_context(|| format!("failed to parse {}", dns_path.display()))?;
    let mut merge_doc: Value = serde_yaml::from_str(&merge_source)
        .with_context(|| format!("failed to parse {}", merge_path.display()))?;

    let merged_dns = merged.extra.get("dns").cloned();
    let merged_tun = merged.extra.get("tun").cloned();
    let merged_hosts = merged.extra.get("hosts").cloned();
    let merged_mode = merged.extra.get("mode").cloned();
    let merged_sniffer = merged.extra.get("sniffer").cloned();

    if let Some(ref src_dns) = merged_dns {
        let dns_map = ensure_mapping_entry(&mut dns_doc, "dns");
        if let Some(src_map) = src_dns.as_mapping() {
            copy_mapping_key(src_map, dns_map, "enhanced-mode");
            copy_mapping_key(src_map, dns_map, "fake-ip-range");
            copy_mapping_key(src_map, dns_map, "fake-ip-filter-mode");
            copy_mapping_key(src_map, dns_map, "fake-ip-filter");
        }
    }

    if let Some(ref src_tun) = merged_tun {
        let tun_map = ensure_mapping_entry(&mut dns_doc, "tun");
        if let Some(src_map) = src_tun.as_mapping() {
            copy_mapping_key(src_map, tun_map, "route-exclude-address");
        }
    }

    if let Some(ref src_dns) = merged_dns {
        let dns_map = ensure_mapping_entry(&mut merge_doc, "dns");
        if let Some(src_map) = src_dns.as_mapping() {
            copy_mapping_key(src_map, dns_map, "fake-ip-filter");
        }
    }

    if let Some(ref src_tun) = merged_tun {
        let tun_map = ensure_mapping_entry(&mut merge_doc, "tun");
        if let Some(src_map) = src_tun.as_mapping() {
            copy_mapping_key(src_map, tun_map, "route-exclude-address");
        }
    }

    if let Some(src_mode) = merged_mode {
        let root_map = ensure_root_mapping(&mut merge_doc);
        root_map.insert(Value::String("mode".to_string()), src_mode);
    }

    {
        let root_map = ensure_root_mapping(&mut merge_doc);
        let key = Value::String("sniffer".to_string());
        if let Some(src_sniffer) = merged_sniffer {
            root_map.insert(key, src_sniffer);
        } else {
            root_map.remove(&key);
        }
    }

    {
        let rules_value = ensure_root_mapping(&mut merge_doc)
            .entry(Value::String("rules".to_string()))
            .or_insert_with(|| Value::Sequence(Vec::new()));
        if let Value::Sequence(seq) = rules_value {
            seq.clear();
            seq.extend(merged.rules.iter().cloned().map(Value::String));
        }
    }

    if let Some(src_hosts) = merged_hosts {
        let root_map = ensure_root_mapping(&mut merge_doc);
        root_map.insert(Value::String("hosts".to_string()), src_hosts);
    }

    fs::write(&dns_path, serde_yaml::to_string(&dns_doc)?)
        .await
        .with_context(|| format!("failed to write {}", dns_path.display()))?;
    fs::write(&merge_path, serde_yaml::to_string(&merge_doc)?)
        .await
        .with_context(|| format!("failed to write {}", merge_path.display()))?;

    println!("synced Clash Verge source config {}", dns_path.display());
    println!("synced Clash Verge source config {}", merge_path.display());

    Ok(())
}

async fn reload_clash_verge_runtime(
    merged: &mihomo_core::ClashConfig,
    runtime_paths: &[PathBuf],
) -> anyhow::Result<()> {
    use serde_json::json;
    use serde_yaml::Value;

    let external_controller = merged
        .extra
        .get("external-controller")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("merged config does not define external-controller"))?;
    let (host, port) = parse_host_port(external_controller).ok_or_else(|| {
        anyhow!(
            "invalid external-controller value '{}' (expected host:port)",
            external_controller
        )
    })?;

    let host = normalize_controller_host(&host);
    let secret = merged
        .extra
        .get("secret")
        .and_then(Value::as_str)
        .unwrap_or("");

    let runtime_path = runtime_paths
        .iter()
        .find(|path| path.file_name().and_then(|name| name.to_str()) == Some("config.yaml"))
        .or_else(|| runtime_paths.first())
        .ok_or_else(|| anyhow!("no synced Clash Verge runtime path available for reload"))?;

    let client = reqwest::Client::builder().build()?;
    let version_url = format!("http://{}:{}/version", host, port);
    let mut version_req = client.get(&version_url);
    if !secret.is_empty() {
        version_req = version_req.bearer_auth(secret);
    }
    version_req.send().await?.error_for_status()?;

    let reload_url = format!("http://{}:{}/configs", host, port);
    let mut reload_req = client.put(&reload_url).json(&json!({
        "path": runtime_path.display().to_string(),
    }));
    if !secret.is_empty() {
        reload_req = reload_req.bearer_auth(secret);
    }
    reload_req.send().await?.error_for_status()?;

    println!("reloaded Clash Verge runtime via {}", reload_url);
    Ok(())
}

fn ensure_root_mapping<'a>(doc: &'a mut serde_yaml::Value) -> &'a mut serde_yaml::Mapping {
    if !doc.is_mapping() {
        *doc = serde_yaml::Value::Mapping(serde_yaml::Mapping::new());
    }
    doc.as_mapping_mut().expect("mapping just initialized")
}

fn ensure_mapping_entry<'a>(
    doc: &'a mut serde_yaml::Value,
    key: &str,
) -> &'a mut serde_yaml::Mapping {
    use serde_yaml::{Mapping, Value};

    let root = ensure_root_mapping(doc);
    let entry = root
        .entry(Value::String(key.to_string()))
        .or_insert_with(|| Value::Mapping(Mapping::new()));
    if !entry.is_mapping() {
        *entry = Value::Mapping(Mapping::new());
    }
    entry.as_mapping_mut().expect("mapping just initialized")
}

fn copy_mapping_key(src: &serde_yaml::Mapping, dst: &mut serde_yaml::Mapping, key: &str) {
    let yaml_key = serde_yaml::Value::String(key.to_string());
    if let Some(value) = src.get(&yaml_key) {
        dst.insert(yaml_key, value.clone());
    }
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

fn resolve_dev_rules_via(via: &str, default_via: &str, cfg: &mihomo_core::ClashConfig) -> String {
    // If the requested via exists as a group or proxy, use it as-is.
    let group_names = cfg.proxy_group_names();
    let proxy_names = cfg.proxy_names();
    if group_names.iter().any(|n| n == via) || proxy_names.iter().any(|n| n == via) {
        return via.to_string();
    }

    // If the user explicitly set a via different from our default, respect it even if missing.
    // This preserves explicit intent; mihomo will surface the error if it's invalid.
    if via != default_via {
        return via.to_string();
    }

    // Prefer common selector name if present.
    let common = "🚀 节点选择";
    if group_names.iter().any(|n| n == common) {
        return common.to_string();
    }

    // Otherwise pick the first available group, then first proxy, else last-resort DIRECT.
    if let Some(first_group) = group_names.first() {
        return first_group.clone();
    }
    if let Some(first_proxy) = proxy_names.first() {
        return first_proxy.clone();
    }
    "DIRECT".to_string()
}

// Built-in developer/AI endpoints considered proxy-worthy.
// Tuple format: (rule kind, target)
// - Use DOMAIN for exact host matches
// - Use DOMAIN-SUFFIX for suffix matches
const DEV_RULE_TARGETS: &[(&str, &str)] = &[
    // Git & code hosting
    ("DOMAIN-SUFFIX", "api.github.com"),
    ("DOMAIN-SUFFIX", "github.com"),
    ("DOMAIN-SUFFIX", "github.dev"),
    ("DOMAIN-SUFFIX", "githubassets.com"),
    ("DOMAIN-SUFFIX", "githubusercontent.com"),
    ("DOMAIN-SUFFIX", "raw.githubusercontent.com"),
    ("DOMAIN-SUFFIX", "codeload.github.com"),
    ("DOMAIN-SUFFIX", "release-assets.githubusercontent.com"),
    ("DOMAIN-SUFFIX", "gitlab.com"),
    ("DOMAIN-SUFFIX", "bitbucket.org"),
    // Language ecosystems / registries
    ("DOMAIN-SUFFIX", "registry.npmjs.org"),
    ("DOMAIN-SUFFIX", "registry.yarnpkg.com"),
    ("DOMAIN-SUFFIX", "registry.npmjs.com"),
    ("DOMAIN-SUFFIX", "nodejs.org"),
    ("DOMAIN-SUFFIX", "pypi.org"),
    ("DOMAIN-SUFFIX", "files.pythonhosted.org"),
    ("DOMAIN-SUFFIX", "pythonhosted.org"),
    ("DOMAIN-SUFFIX", "crates.io"),
    ("DOMAIN-SUFFIX", "index.crates.io"),
    ("DOMAIN-SUFFIX", "static.crates.io"),
    ("DOMAIN-SUFFIX", "rubygems.org"),
    ("DOMAIN-SUFFIX", "golang.org"),
    ("DOMAIN-SUFFIX", "go.dev"),
    ("DOMAIN-SUFFIX", "proxy.golang.org"),
    ("DOMAIN-SUFFIX", "sum.golang.org"),
    ("DOMAIN-SUFFIX", "pkg.go.dev"),
    ("DOMAIN-SUFFIX", "golang.google.cn"),
    ("DOMAIN-SUFFIX", "rust-lang.org"),
    ("DOMAIN-SUFFIX", "static.rust-lang.org"),
    ("DOMAIN-SUFFIX", "doc.rust-lang.org"),
    // Kubernetes / cloud tooling
    ("DOMAIN-SUFFIX", "k8s.io"),
    ("DOMAIN-SUFFIX", "dl.k8s.io"),
    ("DOMAIN-SUFFIX", "k3s.io"),
    ("DOMAIN-SUFFIX", "vultr.com"),
    ("DOMAIN-SUFFIX", "vultrstatus.com"),
    // Containers / registries
    ("DOMAIN-SUFFIX", "docker.com"),
    ("DOMAIN-SUFFIX", "docker.io"),
    ("DOMAIN-SUFFIX", "registry-1.docker.io"),
    ("DOMAIN-SUFFIX", "ghcr.io"),
    ("DOMAIN-SUFFIX", "gcr.io"),
    ("DOMAIN-SUFFIX", "pkg.dev"),
    ("DOMAIN-SUFFIX", "quay.io"),
    // Nix infra
    ("DOMAIN", "cache.nixos.org"),
    ("DOMAIN-SUFFIX", "channels.nixos.org"),
    ("DOMAIN-SUFFIX", "releases.nixos.org"),
    ("DOMAIN-SUFFIX", "nixos.org"),
    ("DOMAIN-SUFFIX", "nix.dev"),
    ("DOMAIN-SUFFIX", "cachix.org"),
    ("DOMAIN-SUFFIX", "flakehub.com"),
    ("DOMAIN-SUFFIX", "determinate.systems"),
    // AI APIs
    ("DOMAIN-SUFFIX", "api.openai.com"),
    ("DOMAIN-SUFFIX", "api.anthropic.com"),
    ("DOMAIN-SUFFIX", "claude.ai"),
    ("DOMAIN-SUFFIX", "platform.claude.com"),
    ("DOMAIN-SUFFIX", "anthropic.com"),
    ("DOMAIN-SUFFIX", "openai.com"),
    ("DOMAIN-SUFFIX", "chatgpt.com"),
    ("DOMAIN-SUFFIX", "openrouter.ai"),
    ("DOMAIN-SUFFIX", "ai.google.dev"),
    ("DOMAIN-SUFFIX", "generativelanguage.googleapis.com"),
    ("DOMAIN-SUFFIX", "gemini.google.com"),
    ("DOMAIN-SUFFIX", "cursor.com"),
    ("DOMAIN-SUFFIX", "cursor.sh"),
];

fn build_dev_rules(via: &str) -> Vec<String> {
    DEV_RULE_TARGETS
        .iter()
        .map(|(kind, target)| format!("{kind},{target},{via}"))
        .collect()
}

fn domain_matches_rule(kind: &str, target: &str, domain: &str) -> bool {
    let d = domain.to_ascii_lowercase();
    let t = target.to_ascii_lowercase();
    match kind {
        "DOMAIN" => d == t,
        "DOMAIN-SUFFIX" => d == t || d.ends_with(&format!(".{t}")),
        "DOMAIN-KEYWORD" => d.contains(&t),
        _ => false,
    }
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
        for prefix in [
            "DOMAIN-SUFFIX,api.github.com,",
            "DOMAIN-SUFFIX,github.com,",
            "DOMAIN-SUFFIX,registry.npmjs.org,",
            "DOMAIN-SUFFIX,pypi.org,",
            "DOMAIN-SUFFIX,crates.io,",
            "DOMAIN-SUFFIX,index.crates.io,",
            "DOMAIN-SUFFIX,proxy.golang.org,",
            "DOMAIN-SUFFIX,golang.google.cn,",
            "DOMAIN-SUFFIX,rust-lang.org,",
            "DOMAIN-SUFFIX,static.rust-lang.org,",
            "DOMAIN-SUFFIX,k3s.io,",
            "DOMAIN-SUFFIX,vultr.com,",
            "DOMAIN-SUFFIX,api.openai.com,",
            "DOMAIN-SUFFIX,api.anthropic.com,",
            "DOMAIN-SUFFIX,claude.ai,",
            "DOMAIN-SUFFIX,platform.claude.com,",
            "DOMAIN-SUFFIX,anthropic.com,",
            "DOMAIN-SUFFIX,openai.com,",
            "DOMAIN-SUFFIX,chatgpt.com,",
            "DOMAIN,cache.nixos.org,",
            "DOMAIN-SUFFIX,channels.nixos.org,",
            "DOMAIN-SUFFIX,cachix.org,",
            "DOMAIN-SUFFIX,openrouter.ai,",
            "DOMAIN-SUFFIX,dl.k8s.io,",
        ] {
            assert!(
                rules.iter().any(|rule| rule.starts_with(prefix)),
                "missing {prefix}"
            );
        }
    }

    #[test]
    fn attach_group_appends_without_duplicates() {
        use serde_yaml::Value;

        let mut groups: Vec<Value> =
            vec![
                serde_yaml::from_str("name: \"BosLife\"\ntype: select\nproxies:\n  - A\n  - B\n")
                    .unwrap(),
            ];

        let names = vec!["B".to_string(), "jp-vultr".to_string()];
        assert!(attach_proxy_names_to_group(&mut groups, "BosLife", &names));

        let map = groups[0].as_mapping().unwrap();
        let seq = map
            .get(&Value::from("proxies"))
            .and_then(|v| v.as_sequence())
            .unwrap();
        let items: Vec<_> = seq.iter().filter_map(|v| v.as_str()).collect();
        assert_eq!(items, vec!["A", "B", "jp-vultr"]);
    }

    #[test]
    fn parse_scutil_proxy_detects_enabled_entries() {
        let raw = r#"
<dictionary> {
  HTTPEnable : 1
  HTTPPort : 7897
  HTTPProxy : 127.0.0.1
  HTTPSEnable : 1
  HTTPSPort : 7897
  HTTPSProxy : 127.0.0.1
  SOCKSEnable : 1
  SOCKSPort : 7897
  SOCKSProxy : 127.0.0.1
}
"#;

        let entries = parse_scutil_proxy(raw);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].kind, "HTTP");
        assert_eq!(entries[1].kind, "HTTPS");
        assert_eq!(entries[2].kind, "SOCKS");
    }

    #[test]
    fn split_list_items_handles_commas_spaces_and_newlines() {
        assert_eq!(
            split_list_items("foo.com, bar.com\nbaz.com"),
            vec!["foo.com", "bar.com", "baz.com"]
        );
    }

    #[test]
    fn extract_current_clash_verge_subscription_url_prefers_current_uid() {
        let profiles = ClashVergeProfiles {
            current: Some("active".to_string()),
            items: vec![
                ClashVergeProfileItem {
                    uid: Some("other".to_string()),
                    url: Some("https://example.com/other.yaml".to_string()),
                },
                ClashVergeProfileItem {
                    uid: Some("active".to_string()),
                    url: Some("https://example.com/active.yaml".to_string()),
                },
            ],
        };

        assert_eq!(
            extract_current_clash_verge_subscription_url(&profiles).as_deref(),
            Some("https://example.com/active.yaml")
        );
    }

    #[test]
    fn infer_connection_path_prefers_local_proxy_for_default_mixed() {
        let record = ConnectionRecord {
            host: Some("chatgpt.com".to_string()),
            inbound_name: Some("DEFAULT-MIXED".to_string()),
            source_ip: Some("127.0.0.1".to_string()),
            chains: vec!["BosLife".to_string()],
        };

        assert_eq!(
            infer_connection_path(&[&record]),
            "local proxy / DEFAULT-MIXED"
        );
    }

    #[test]
    fn tailscale_compatibility_rewrites_fake_ip_and_adds_exclusions() {
        use serde_yaml::Value;

        let raw = r#"
dns:
  enhanced-mode: fake-ip
  fake-ip-range: 198.18.0.1/16
  fake-ip-filter:
    - "*.local"
tun:
  enable: true
"#;
        let mut cfg: mihomo_core::ClashConfig = serde_yaml::from_str(raw).unwrap();

        apply_tailscale_compatibility(&mut cfg, &[String::from("zhsjf.cn")], &[]);

        let dns = cfg.extra.get("dns").and_then(|v| v.as_mapping()).unwrap();
        assert_eq!(
            dns.get(Value::String("fake-ip-range".into()))
                .and_then(|v| v.as_str()),
            Some(SAFE_FAKE_IP_RANGE)
        );
        let filters = dns
            .get(Value::String("fake-ip-filter".into()))
            .and_then(|v| v.as_sequence())
            .unwrap();
        for item in build_tailscale_fake_ip_bypass(&[String::from("zhsjf.cn")], &[]) {
            assert!(filters.iter().any(|v| v.as_str() == Some(item.as_str())));
        }

        let tun = cfg.extra.get("tun").and_then(|v| v.as_mapping()).unwrap();
        let excludes = tun
            .get(Value::String("route-exclude-address".into()))
            .and_then(|v| v.as_sequence())
            .unwrap();
        for cidr in TAILSCALE_ROUTE_EXCLUDES {
            assert!(excludes.iter().any(|v| v.as_str() == Some(cidr)));
        }
    }

    #[test]
    fn tailscale_compatibility_deduplicates_entries() {
        use serde_yaml::Value;

        let raw = r#"
dns:
  enhanced-mode: fake-ip
  fake-ip-range: 172.19.0.1/16
  fake-ip-filter:
    - +.tailscale.com
tun:
  route-exclude-address:
    - 100.64.0.0/10
"#;
        let mut cfg: mihomo_core::ClashConfig = serde_yaml::from_str(raw).unwrap();

        apply_tailscale_compatibility(&mut cfg, &[String::from("zhsjf.cn")], &[]);
        apply_tailscale_compatibility(&mut cfg, &[String::from("zhsjf.cn")], &[]);

        let dns = cfg.extra.get("dns").and_then(|v| v.as_mapping()).unwrap();
        let filters = dns
            .get(Value::String("fake-ip-filter".into()))
            .and_then(|v| v.as_sequence())
            .unwrap();
        assert_eq!(
            filters
                .iter()
                .filter(|v| v.as_str() == Some("+.tailscale.com"))
                .count(),
            1
        );

        let tun = cfg.extra.get("tun").and_then(|v| v.as_mapping()).unwrap();
        let excludes = tun
            .get(Value::String("route-exclude-address".into()))
            .and_then(|v| v.as_sequence())
            .unwrap();
        assert_eq!(
            excludes
                .iter()
                .filter(|v| v.as_str() == Some("100.64.0.0/10"))
                .count(),
            1
        );
        assert_eq!(
            cfg.rules
                .iter()
                .filter(|rule| rule == &&"DOMAIN-SUFFIX,tailscale.com,DIRECT".to_string())
                .count(),
            1
        );
    }

    #[test]
    fn tailscale_direct_rules_are_prefixed() {
        let raw = r#"
rules:
  - MATCH,Proxy
"#;
        let mut cfg: mihomo_core::ClashConfig = serde_yaml::from_str(raw).unwrap();

        apply_tailscale_compatibility(&mut cfg, &[String::from("zhsjf.cn")], &[]);

        assert_eq!(cfg.rules[0], "DOMAIN-SUFFIX,tail.zhsjf.cn,DIRECT");
        assert_eq!(cfg.rules[1], "DOMAIN-SUFFIX,tailscale.com,DIRECT");
        assert_eq!(cfg.rules[2], "DOMAIN-SUFFIX,ts.net,DIRECT");
        assert_eq!(cfg.rules[3], "MATCH,Proxy");
    }

    #[test]
    fn tailscale_tailnet_suffixes_are_parameterized() {
        assert_eq!(
            build_tailscale_fake_ip_bypass(&[String::from("example.com")], &[]),
            vec![
                "+.tail.example.com".to_string(),
                "+.tailscale.com".to_string(),
                "+.ts.net".to_string()
            ]
        );
        assert_eq!(
            build_tailscale_direct_rules(&[String::from("example.com")], &[]),
            vec![
                "DOMAIN-SUFFIX,tail.example.com,DIRECT".to_string(),
                "DOMAIN-SUFFIX,tailscale.com,DIRECT".to_string(),
                "DOMAIN-SUFFIX,ts.net,DIRECT".to_string()
            ]
        );
    }

    #[test]
    fn tailscale_direct_domains_are_parameterized() {
        assert_eq!(
            build_tailscale_fake_ip_bypass(
                &[String::from("example.com")],
                &[
                    String::from("derp.zhsjf.cn"),
                    String::from("+.corp.example.com")
                ]
            ),
            vec![
                "+.corp.example.com".to_string(),
                "+.tail.example.com".to_string(),
                "+.tailscale.com".to_string(),
                "+.ts.net".to_string(),
                "derp.zhsjf.cn".to_string()
            ]
        );
        assert_eq!(
            build_tailscale_direct_rules(
                &[String::from("example.com")],
                &[
                    String::from("derp.zhsjf.cn"),
                    String::from("+.corp.example.com")
                ]
            ),
            vec![
                "DOMAIN,derp.zhsjf.cn,DIRECT".to_string(),
                "DOMAIN-SUFFIX,corp.example.com,DIRECT".to_string(),
                "DOMAIN-SUFFIX,tail.example.com,DIRECT".to_string(),
                "DOMAIN-SUFFIX,tailscale.com,DIRECT".to_string(),
                "DOMAIN-SUFFIX,ts.net,DIRECT".to_string()
            ]
        );
    }

    #[test]
    fn tailscale_managed_state_tracks_extra_route_excludes() {
        let state = build_tailscale_managed_state(
            &[String::from("example.com")],
            &[String::from("derp.example.com")],
            &[String::from("203.0.113.10/32")],
        );

        assert!(state
            .fake_ip_filter
            .contains(&"+.tail.example.com".to_string()));
        assert!(state
            .rules
            .contains(&"DOMAIN,derp.example.com,DIRECT".to_string()));
        assert!(state
            .route_exclude_address
            .contains(&"203.0.113.10/32".to_string()));
    }

    #[test]
    fn remove_tailscale_managed_items_cleans_previous_entries() {
        use serde_yaml::Value;

        let mut cfg: mihomo_core::ClashConfig = serde_yaml::from_str(
            r#"
dns:
  fake-ip-filter:
    - ts-d.zhsjf.cn
    - keep.example.com
tun:
  route-exclude-address:
    - 114.215.124.90/32
    - 203.0.113.10/32
rules:
  - DOMAIN,ts-d.zhsjf.cn,DIRECT
  - MATCH,Proxy
"#,
        )
        .unwrap();

        let managed = ManagedTailscaleCompat {
            fake_ip_filter: vec!["ts-d.zhsjf.cn".to_string()],
            route_exclude_address: vec!["114.215.124.90/32".to_string()],
            rules: vec!["DOMAIN,ts-d.zhsjf.cn,DIRECT".to_string()],
        };

        remove_tailscale_managed_items(&mut cfg, &managed);

        let dns = cfg.extra.get("dns").and_then(|v| v.as_mapping()).unwrap();
        let filters = dns
            .get(Value::String("fake-ip-filter".into()))
            .and_then(|v| v.as_sequence())
            .unwrap();
        assert!(filters
            .iter()
            .any(|v| v.as_str() == Some("keep.example.com")));
        assert!(!filters.iter().any(|v| v.as_str() == Some("ts-d.zhsjf.cn")));

        let tun = cfg.extra.get("tun").and_then(|v| v.as_mapping()).unwrap();
        let excludes = tun
            .get(Value::String("route-exclude-address".into()))
            .and_then(|v| v.as_sequence())
            .unwrap();
        assert!(excludes
            .iter()
            .any(|v| v.as_str() == Some("203.0.113.10/32")));
        assert!(!excludes
            .iter()
            .any(|v| v.as_str() == Some("114.215.124.90/32")));

        assert_eq!(cfg.rules, vec!["MATCH,Proxy".to_string()]);
    }

    #[test]
    fn ensure_fallback_match_rule_appends_when_missing() {
        let mut cfg: mihomo_core::ClashConfig = serde_yaml::from_str(
            r#"
rules:
  - DOMAIN-SUFFIX,tailscale.com,DIRECT
"#,
        )
        .unwrap();

        ensure_fallback_match_rule(&mut cfg, "BosLife");

        assert_eq!(cfg.rules.last().map(String::as_str), Some("MATCH,BosLife"));
    }

    #[test]
    fn ensure_fallback_match_rule_keeps_existing_match() {
        let mut cfg: mihomo_core::ClashConfig = serde_yaml::from_str(
            r#"
rules:
  - DOMAIN-SUFFIX,tailscale.com,DIRECT
  - MATCH,Proxy
"#,
        )
        .unwrap();

        ensure_fallback_match_rule(&mut cfg, "BosLife");

        assert_eq!(
            cfg.rules,
            vec![
                "DOMAIN-SUFFIX,tailscale.com,DIRECT".to_string(),
                "MATCH,Proxy".to_string()
            ]
        );
    }

    #[test]
    fn normalize_controller_host_maps_wildcards_to_loopback() {
        assert_eq!(normalize_controller_host("0.0.0.0"), "127.0.0.1");
        assert_eq!(normalize_controller_host("::"), "127.0.0.1");
        assert_eq!(normalize_controller_host("[::]"), "127.0.0.1");
        assert_eq!(normalize_controller_host("127.0.0.1"), "127.0.0.1");
    }

    #[test]
    fn preferred_runtime_path_prefers_clash_verge_yaml() {
        let paths = vec![
            PathBuf::from("/tmp/clash-verge.yaml"),
            PathBuf::from("/tmp/config.yaml"),
        ];

        assert_eq!(
            preferred_runtime_path(&paths).map(|path| path.as_path()),
            Some(Path::new("/tmp/clash-verge.yaml"))
        );
    }

    #[test]
    fn preferred_runtime_path_falls_back_to_first_entry() {
        let paths = vec![PathBuf::from("/tmp/clash-verge.yaml")];

        assert_eq!(
            preferred_runtime_path(&paths).map(|path| path.as_path()),
            Some(Path::new("/tmp/clash-verge.yaml"))
        );
    }

    #[test]
    fn apply_mode_override_sets_rule_mode() {
        let mut cfg = mihomo_core::ClashConfig::default();
        cfg.extra
            .insert("mode".to_string(), Value::String("global".to_string()));

        apply_mode_override(&mut cfg, ConfigMode::Rule);

        assert_eq!(cfg.extra.get("mode").and_then(Value::as_str), Some("rule"));
    }

    #[test]
    fn apply_sniffer_preset_tun_installs_sniffer() {
        let mut cfg = mihomo_core::ClashConfig::default();

        apply_sniffer_preset(&mut cfg, SnifferPreset::Tun);

        let sniffer = cfg
            .extra
            .get("sniffer")
            .and_then(Value::as_mapping)
            .unwrap();
        assert_eq!(
            sniffer
                .get(Value::String("enable".into()))
                .and_then(Value::as_bool),
            Some(true)
        );
        let sniff = sniffer
            .get(Value::String("sniff".into()))
            .and_then(Value::as_mapping)
            .unwrap();
        assert!(sniff.contains_key(Value::String("HTTP".into())));
        assert!(sniff.contains_key(Value::String("TLS".into())));
        assert!(sniff.contains_key(Value::String("QUIC".into())));
    }

    #[test]
    fn apply_sniffer_preset_off_removes_sniffer() {
        let mut cfg: mihomo_core::ClashConfig = serde_yaml::from_str(
            r#"
sniffer:
  enable: true
"#,
        )
        .unwrap();

        apply_sniffer_preset(&mut cfg, SnifferPreset::Off);

        assert!(!cfg.extra.contains_key("sniffer"));
    }

    #[test]
    fn ensure_root_mapping_allows_mode_override_in_source_doc() {
        let mut doc = serde_yaml::from_str::<Value>(
            r#"
rules:
  - MATCH,Proxy
"#,
        )
        .unwrap();

        ensure_root_mapping(&mut doc).insert(
            Value::String("mode".to_string()),
            Value::String("rule".to_string()),
        );

        let root = doc.as_mapping().unwrap();
        assert_eq!(
            root.get(Value::String("mode".to_string()))
                .and_then(Value::as_str),
            Some("rule")
        );
    }
}

fn default_base_config_path(paths: &AppPaths) -> Option<PathBuf> {
    let candidate = paths.app_config_path().with_file_name("base-config.yaml");
    if candidate.exists() {
        return Some(candidate);
    }

    paths
        .detected_clash_verge_base_config_candidates()
        .into_iter()
        .find(|candidate| candidate.exists())
}

async fn ensure_parent(path: &Path) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await?;
    }
    Ok(())
}

async fn backup_existing_file(path: &Path) -> anyhow::Result<Option<PathBuf>> {
    if !fs::try_exists(path).await.unwrap_or(false) {
        return Ok(None);
    }

    let backup = path.with_extension("mihomocli.bak");
    fs::copy(path, &backup).await.with_context(|| {
        format!(
            "failed to back up {} to {}",
            path.display(),
            backup.display()
        )
    })?;
    Ok(Some(backup))
}

const DEFAULT_TEMPLATE_CONTENT: &str = include_str!("../../../examples/cvr_template.yaml");

async fn ensure_default_template(paths: &AppPaths) -> anyhow::Result<()> {
    let template_path = paths.default_template_path();

    if !fs::try_exists(&template_path).await.unwrap_or(false) {
        if let Some(parent) = template_path.parent() {
            fs::create_dir_all(parent).await?;
        }
        fs::write(&template_path, DEFAULT_TEMPLATE_CONTENT).await?;
        tracing::info!(path = %template_path.display(), "installed default template");
    }

    Ok(())
}

#[derive(Args)]
struct TestArgs {
    /// Path to mihomo binary (defaults to `mihomo` in PATH)
    #[arg(long = "mihomo-bin", default_value = "mihomo")]
    mihomo_bin: String,

    /// Config file to test (defaults to ~/.config/mihomocli/output/clash-verge.yaml)
    #[arg(long)]
    config: Option<PathBuf>,

    /// Working directory passed to mihomo via -d (defaults to ~/.config/mihomocli)
    #[arg(long = "mihomo-dir")]
    mihomo_dir: Option<PathBuf>,
}

async fn run_test(args: TestArgs) -> anyhow::Result<()> {
    use tokio::process::Command;

    let paths = AppPaths::new()?;
    let config_path = args
        .config
        .unwrap_or_else(|| paths.generated_clash_verge_path());
    let workdir = args
        .mihomo_dir
        .unwrap_or_else(|| paths.config_dir().to_path_buf());

    let status = Command::new(&args.mihomo_bin)
        .arg("-d")
        .arg(workdir)
        .arg("-f")
        .arg(&config_path)
        .arg("-m")
        .arg("-t")
        .status()
        .await?;

    if status.success() {
        println!("mihomo config test passed: {}", config_path.display());
        Ok(())
    } else {
        Err(anyhow!(
            "mihomo config test failed (exit code: {:?})",
            status.code()
        ))
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

/// Parse host:port from a string. Supports "host:port" and "[IPv6]:port".
fn parse_host_port(s: &str) -> Option<(String, u16)> {
    // Bracketed IPv6 like [::1]:9090
    if let Some(close_idx) = s.rfind(']') {
        let open_idx = s.find('[')?;
        if close_idx < s.len().saturating_sub(2) && s.as_bytes().get(close_idx + 1) == Some(&b':') {
            let host = s.get(open_idx + 1..close_idx)?.to_string();
            let port_str = s.get(close_idx + 2..)?;
            if let Ok(port) = port_str.parse::<u16>() {
                return Some((host, port));
            }
        }
        return None;
    }

    // Fallback: split by last ':'
    if let Some(idx) = s.rfind(':') {
        let (host, port_str) = s.split_at(idx);
        let port_str = &port_str[1..];
        if let Ok(port) = port_str.parse::<u16>() {
            return Some((host.to_string(), port));
        }
    }
    None
}

fn normalize_controller_host(host: &str) -> String {
    let trimmed = host.trim().trim_matches(['[', ']']);
    if trimmed == "0.0.0.0" || trimmed == "::" || trimmed == "*" || trimmed.is_empty() {
        "127.0.0.1".to_string()
    } else {
        trimmed.to_string()
    }
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

    /// Check whether a domain should go via proxy or direct
    Check(CheckArgs),

    /// List all built-in dev rule domains that are considered proxy-worthy
    #[command(
        about = "List built-in dev domains",
        long_about = "List the built-in developer/infra domains considered proxy-worthy.",
        after_long_help = r#"
Tips

  - Preview the dev rules without writing a file:

      mihomo-cli merge --dev-rules-show --dry-run

  - Apply dev rules to the merged config using a specific group:

      mihomo-cli merge -s https://example.com/sub.yaml --dev-rules-via Proxy --dry-run
"#
    )]
    DevList(DevListArgs),

    /// Manage manually-added server sources (file references with share links)
    #[command(about = "Manage manual server sources")]
    Server {
        #[command(subcommand)]
        command: ServerCmd,
    },
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
    /// Proxy or group name to route via (accepts special values: direct/reject)
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

#[derive(Args)]
struct CheckArgs {
    /// Domain to evaluate (e.g., github.com)
    #[arg(long)]
    domain: String,
}

#[derive(Subcommand)]
enum ServerCmd {
    /// Add or update a manual server source (a file with share links)
    Add(ServerAddArgs),
    /// List manual server sources
    List,
    /// Remove a manual server source by name
    Remove(ServerRemoveArgs),
}

#[derive(Args)]
struct ServerAddArgs {
    /// Unique name for this manual server source (e.g., jp-vultr)
    #[arg(long)]
    name: String,
    /// Path to a local file containing share links (trojan/vmess/ss), one per line
    #[arg(long)]
    file: PathBuf,
    /// Replace existing entry with the same name
    #[arg(long, default_value_t = false)]
    replace: bool,
    /// Add the entry disabled (won't be injected during merge)
    #[arg(long, default_value_t = false)]
    disabled: bool,

    /// Append injected proxy names into the specified proxy-group(s) (repeatable)
    #[arg(long = "attach-group")]
    attach_groups: Vec<String>,
}

#[derive(Args)]
struct ServerRemoveArgs {
    /// Name to remove
    #[arg(long)]
    name: String,
}

async fn run_manage(cmd: Manage) -> anyhow::Result<()> {
    let paths = AppPaths::new()?;
    paths.ensure_runtime_dirs().await?;
    match cmd {
        Manage::Cache(c) => manage_cache(&paths, c).await,
        Manage::Custom(c) => manage_custom(&paths, c).await,
        Manage::Check(c) => manage_check(&paths, c).await,
        Manage::DevList(args) => manage_dev_list(args).await,
        Manage::Server { command } => manage_server(&paths, command).await,
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
            // Normalize well-known targets to canonical forms
            let via_value = match args.via.to_ascii_lowercase().as_str() {
                "direct" => "DIRECT".to_string(),
                "reject" => "REJECT".to_string(),
                // common group name in templates
                "proxy" => "Proxy".to_string(),
                _ => args.via.clone(),
            };
            let rule = CustomRule {
                domain: args.domain,
                kind,
                via: via_value,
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

async fn manage_check(paths: &AppPaths, args: CheckArgs) -> anyhow::Result<()> {
    let cfg = storage::load_app_config(paths).await?;
    // Check user custom rules first (highest precedence)
    for r in &cfg.custom_rules {
        let kind = match r.kind {
            RuleKind::Domain => "DOMAIN",
            RuleKind::DomainSuffix => "DOMAIN-SUFFIX",
            RuleKind::DomainKeyword => "DOMAIN-KEYWORD",
        };
        if domain_matches_rule(kind, &r.domain, &args.domain) {
            if r.via.eq_ignore_ascii_case("direct") {
                println!("direct");
            } else {
                println!("proxy");
            }
            return Ok(());
        }
    }

    // Fallback: treat known dev endpoints as proxy-worthy
    for (kind, target) in DEV_RULE_TARGETS.iter() {
        if domain_matches_rule(kind, target, &args.domain) {
            println!("proxy");
            return Ok(());
        }
    }

    // Default: direct
    println!("direct");
    Ok(())
}

async fn manage_server(paths: &AppPaths, cmd: ServerCmd) -> anyhow::Result<()> {
    let mut cfg = storage::load_app_config(paths).await?;
    match cmd {
        ServerCmd::Add(args) => {
            // Validate file exists and is readable (do not read its contents here to avoid leaks).
            if !fs::try_exists(&args.file).await.unwrap_or(false) {
                return Err(anyhow!(
                    "manual server file does not exist: {}",
                    args.file.display()
                ));
            }

            let entry = ManualServerRef {
                name: args.name.clone(),
                file: args.file.clone(),
                attach_groups: args.attach_groups.clone(),
                enabled: !args.disabled,
            };

            if let Some(existing) = cfg.manual_servers.iter_mut().find(|s| s.name == args.name) {
                if args.replace {
                    *existing = entry;
                    storage::save_app_config(paths, &cfg).await?;
                    println!("manual server updated");
                } else {
                    println!("manual server already exists (use --replace to update)");
                }
            } else {
                cfg.manual_servers.push(entry);
                storage::save_app_config(paths, &cfg).await?;
                println!("manual server added");
            }
        }
        ServerCmd::List => {
            if cfg.manual_servers.is_empty() {
                println!("<no manual servers>");
            } else {
                for s in &cfg.manual_servers {
                    println!(
                        "{}\t{}\tenabled={}",
                        s.name,
                        s.file.display(),
                        if s.enabled { "true" } else { "false" }
                    );
                }
            }
        }
        ServerCmd::Remove(args) => {
            let before = cfg.manual_servers.len();
            cfg.manual_servers.retain(|s| s.name != args.name);
            let after = cfg.manual_servers.len();
            storage::save_app_config(paths, &cfg).await?;
            println!("removed {} manual server(s)", before.saturating_sub(after));
        }
    }
    Ok(())
}

#[derive(Args)]
struct DevListArgs {
    /// Output format: plain|yaml|json (default: plain)
    #[arg(long, default_value = "plain")]
    format: String,
}

async fn manage_dev_list(args: DevListArgs) -> anyhow::Result<()> {
    // Collect unique domain targets from built-in dev rules
    let mut set = HashSet::new();
    for (_, target) in DEV_RULE_TARGETS.iter() {
        set.insert(target.to_string());
    }
    let mut items: Vec<String> = set.into_iter().collect();
    items.sort();

    match args.format.as_str() {
        "json" => {
            println!("{}", serde_json::to_string_pretty(&items)?);
        }
        "yaml" => {
            println!("{}", serde_yaml::to_string(&items)?);
        }
        _ => {
            for d in items {
                println!("{}", d);
            }
        }
    }
    Ok(())
}

async fn inject_manual_servers(
    merged: &mut mihomo_core::ClashConfig,
    app_cfg: &mihomo_core::storage::AppConfig,
) -> anyhow::Result<usize> {
    let mut existing: HashSet<String> = merged.proxy_names().into_iter().collect();
    let mut added = 0usize;

    for s in app_cfg.manual_servers.iter().filter(|s| s.enabled) {
        let raw = fs::read_to_string(&s.file)
            .await
            .with_context(|| format!("failed to read manual server file {}", s.file.display()))?;

        let Some(cfg) = mihomo_core::subscription::parse_share_links_payload(&raw)? else {
            warn!(name = %s.name, file = %s.file.display(), "manual server file contains no supported share links");
            continue;
        };

        let mut injected_names: Vec<String> = Vec::new();
        for mut proxy in cfg.proxies.into_iter() {
            // Ensure proxy is a mapping and has a unique name.
            let orig_name = proxy_name(&proxy).unwrap_or_else(|| s.name.clone());
            let unique = unique_proxy_name(&orig_name, &mut existing);
            if unique != orig_name {
                warn!(from = %orig_name, to = %unique, "manual proxy name collision; renamed");
            }
            set_proxy_name(&mut proxy, &unique);

            merged.proxies.push(proxy);
            injected_names.push(unique);
            added += 1;
        }

        if !injected_names.is_empty() && !s.attach_groups.is_empty() {
            for group in &s.attach_groups {
                let ok =
                    attach_proxy_names_to_group(&mut merged.proxy_groups, group, &injected_names);
                if ok {
                    info!(group = %group, added = injected_names.len(), "attached manual proxies to group");
                } else {
                    warn!(group = %group, "requested attach-group not found; skipping");
                }
            }
        }
    }

    Ok(added)
}

fn attach_proxy_names_to_group(
    groups: &mut [serde_yaml::Value],
    group_name: &str,
    proxy_names: &[String],
) -> bool {
    use serde_yaml::Value;

    for g in groups.iter_mut() {
        let Value::Mapping(map) = g else {
            continue;
        };
        let name = map
            .get(Value::from("name"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if name != group_name {
            continue;
        }

        let proxies_key = Value::from("proxies");
        let entry = map
            .entry(proxies_key)
            .or_insert_with(|| Value::Sequence(Vec::new()));
        if !matches!(entry, Value::Sequence(_)) {
            *entry = Value::Sequence(Vec::new());
        }

        let Value::Sequence(seq) = entry else {
            return true;
        };
        let mut set: HashSet<String> = seq
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();
        for n in proxy_names {
            if set.insert(n.clone()) {
                seq.push(Value::from(n.as_str()));
            }
        }
        return true;
    }

    false
}

fn proxy_name(value: &serde_yaml::Value) -> Option<String> {
    use serde_yaml::Value;
    match value {
        Value::Mapping(map) => map
            .get(Value::from("name"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        _ => None,
    }
}

fn set_proxy_name(value: &mut serde_yaml::Value, name: &str) {
    use serde_yaml::Value;
    if let Value::Mapping(map) = value {
        map.insert(Value::from("name"), Value::from(name));
    }
}

fn unique_proxy_name(base: &str, existing: &mut HashSet<String>) -> String {
    if existing.insert(base.to_string()) {
        return base.to_string();
    }
    let mut i = 1usize;
    loop {
        let candidate = if i == 1 {
            format!("{base} (manual)")
        } else {
            format!("{base} (manual {i})")
        };
        if existing.insert(candidate.clone()) {
            return candidate;
        }
        i += 1;
    }
}
