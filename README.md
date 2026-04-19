# mihomo-cli

`mihomo-cli` is a Rust-based command-line tool for merging Mihomo/Clash subscriptions using the same conventions as [clash-verge-rev](https://github.com/clash-verge-rev/clash-verge-rev). It merges multiple subscriptions with a template and can inherit ports, DNS, rules, and proxy-groups from an existing Clash config. By default it mimics Clash Verge’s HTTP User-Agent so providers return Clash YAML (with rules) when available. You can optionally enable decoding of base64/share-link lists (trojan/vmess/shadowsocks).

## Features
- Rust workspace with reusable core crate and CLI front-end
- Clash YAML parsing; optional share-link decoding (disabled by default)
- Template + subscription merge identical to clash-verge-rev semantics
- Optional base-config inheritance (`--base-config`) to reuse existing rules/groups
- Auto-downloads `Country.mmdb`, `geoip.dat`, `geosite.dat` into `~/.config/mihomocli/resources/`
- Caches last used subscription URL (for quick re-run without args)
- Manage quick custom domain->proxy rules (`manage custom add/list/remove`)

## Quick Start
Tip: Use the Nix dev shell for a pinned Rust toolchain. Either enter an interactive shell or invoke Cargo via `nix develop -c`.

If you use `direnv`/`nix-direnv`, keep your own local hook if you want automatic shell activation. The checked-in workflow here stays centered on `nix develop -c ...`.

Windows does not need Nix. The intended flow there is plain Cargo or a shipped `mihomo-cli.exe`:

```powershell
cargo build --release -p mihomo-cli
.\target\release\mihomo-cli.exe doctor
.\target\release\mihomo-cli.exe refresh-clash-verge
```

```
nix develop
```

```bash
# Initialize runtime directories and seed default template
mihomo-cli init
```

```bash
# Build (one‑shot)
nix develop -c cargo build -p mihomo-cli

# Merge with default (bundled) template and remote subscription (output defaults to
# ~/.config/mihomocli/output/clash-verge.yaml). The default User-Agent is
# 'clash-verge/v2.4.2', which often yields Clash YAML with rules.
mihomo-cli merge \
  --subscription "https://example.com/subscription"

# If your provider only returns base64/share-link lists, explicitly allow it:
mihomo-cli merge \
  --subscription "https://example.com/base64" \
  --subscription-allow-base64

# Want clash-verge-rev parity? Either drop your clash-verge.yaml at
# ~/.config/mihomocli/base-config.yaml, or let mihomo-cli auto-detect the local
# Clash Verge app directory and exported config.

# To also replace Clash Verge's runtime config.yaml in one step:
mihomo-cli merge \
  --use-last \
  --sync-to-clash-verge

# Practical daily flow after clicking refresh in Clash Verge:
nix develop -c cargo run -p mihomo-cli -- refresh-clash-verge

# Inspect or change the live local runtime without opening the GUI:
nix develop -c cargo run -p mihomo-cli -- runtime status
nix develop -c cargo run -p mihomo-cli -- runtime mode rule
```

## End-to-End Test (UA feature)

Some providers return a richer Clash YAML (including many DOMAIN-SUFFIX rules) only when the HTTP User-Agent matches known clients. The CLI defaults to `clash-verge/v2.4.2`, which usually triggers full outputs.

Example with a real provider URL (adjust to your environment/network):

```bash
mihomo-cli merge \
  --subscription "https://example.com/sub.yaml"

# If you need to override UA for debugging:
mihomo-cli merge \
  --subscription "https://example.com/sub.yaml" \
  --subscription-ua "clash-verge/v2.4.2"
```

Expected results:
- Resources `Country.mmdb`, `geoip.dat`, `geosite.dat` are auto-downloaded into `~/.config/mihomocli/resources/` if missing.
- Output written to `~/.config/mihomocli/output/clash-verge.yaml` unless `--stdout` is used.
- Merged YAML contains many DOMAIN-SUFFIX rules from the provider.

## CVR‑Aligned Template (no base-config)

The CLI auto-installs a CVR-aligned template at `~/.config/mihomocli/templates/cvr_template.yaml` and uses it by default when `--template` is omitted. To reference it explicitly or copy for customization:

```bash
mihomo-cli merge \
  --template examples/cvr_template.yaml \
  --subscription "https://example.com/sub.yaml"
```

Notes:
- The template mirrors CVR runtime settings (mixed-port, tun, dns, profile, etc.).
- It leaves `proxy-groups` and `rules` empty so the provider subscription defines them fully.
- `secret` is empty by default; set it in your own copy if needed.

Kubernetes + tun note

- If you run Kubernetes (k3s/k8s) on the same host with tun + dns-hijack enabled, exclude the Pod/Service CIDRs from tun.
- Defaults already add `10.42.0.0/16` and `10.43.0.0/16` (k3s defaults). Add more with `--k8s-cidr-exclude <CIDR>`.

Run Mihomo with the generated configuration and resources:

```bash
mihomo -d ~/.config/mihomocli/resources -f ~/.config/mihomocli/output/clash-verge.yaml
```

Or validate quickly using the built-in test helper (wraps `mihomo -t`):

```
nix develop -c mihomo-cli test \
  --mihomo-dir ~/.config/mihomocli \
  --config ~/.config/mihomocli/output/clash-verge.yaml
```

## Server Bootstrap (first run)

When running on servers with slow access to GitHub, you can avoid first-run stalls by pre-creating directories and preloading resources.

```bash
# 1) Create directories and seed template (no network fetch)
mihomo-cli init

# 2) Manually preload resources (optional if your server can reach GitHub)
mkdir -p ~/.config/mihomocli/resources

# Country.mmdb
curl -L -o ~/.config/mihomocli/resources/Country.mmdb \
  https://ghproxy.com/https://github.com/MetaCubeX/meta-rules-dat/releases/download/latest/country.mmdb

# geoip.dat
curl -L -o ~/.config/mihomocli/resources/geoip.dat \
  https://ghproxy.com/https://github.com/MetaCubeX/meta-rules-dat/releases/download/latest/geoip.dat

# geosite.dat
curl -L -o ~/.config/mihomocli/resources/geosite.dat \
  https://ghproxy.com/https://github.com/MetaCubeX/meta-rules-dat/releases/download/latest/geosite.dat
```

Notes:
- If `~/.config/mihomocli/resources/{Country.mmdb,geoip.dat,geosite.dat}` already exist, the CLI skips downloading them during `merge`.
- Resource URLs (built-in defaults):
  - Country.mmdb: `https://github.com/MetaCubeX/meta-rules-dat/releases/download/latest/country.mmdb`
  - geoip.dat: `https://github.com/MetaCubeX/meta-rules-dat/releases/download/latest/geoip.dat`
  - geosite.dat: `https://github.com/MetaCubeX/meta-rules-dat/releases/download/latest/geosite.dat`
- Alternative mirrors (prepend a proxy to the same URLs):
  - `https://ghproxy.com/` or `https://mirror.ghproxy.com/` or `https://github.moeyy.xyz/`
- Alternative data sources (compatible format):
  - Country.mmdb: `https://github.com/P3TERX/GeoLite.mmdb/releases/latest/download/Country.mmdb`
  - geoip/geosite: `https://github.com/Loyalsoldier/v2ray-rules-dat/releases/latest/download/{geoip.dat|geosite.dat}`


## CLI Flags of Interest

- `--subscription-ua <STRING>`: HTTP User-Agent used to fetch subscriptions. Default: `clash-verge/v2.4.2`.
- `--subscription-allow-base64`: Enable decoding base64/share-link subscriptions (trojan/vmess/ss). Disabled by default to prefer native Clash YAML from providers.
- `--sync-to-clash-verge`: After writing the normal output file, auto-detect Clash Verge's local `config.yaml`, back it up, and replace it with the generated config.
- `--no-dev-rules [--dev-rules-via <NAME>]`: Dev rules are enabled by default and prepend proxy rules for common developer registries and slow infra endpoints (GitHub/GitLab, npm/yarn, PyPI, crates.io, Go proxy, Vultr, Docker/GCR, `cache.nixos.org`, `channels.nixos.org`, `cachix.org`, AI agent APIs such as OpenAI/Claude/Gemini/Cursor/OpenRouter, etc.). Override the target group with `--dev-rules-via` or disable via `--no-dev-rules`. If the requested group `Proxy` is not present in the merged config, the CLI falls back to an existing group (preferring `🚀 节点选择`), otherwise the first group, then the first proxy, and finally `DIRECT`.
- `--dev-rules-show`: Print the generated dev rule list (useful for inspection without modifying output).
- External controller settings: `--external-controller-url <HOST>`, `--external-controller-port <PORT>`, and `--external-controller-secret <SECRET>` to set `external-controller` and `secret` in the merged output.

### Runtime management

`mihomo-cli runtime` is the GUI-free companion to `refresh-clash-verge`:

- `mihomo-cli runtime status`: show detected runtime files, current mode/tun/sniffer state, and live controller status when reachable
- `mihomo-cli runtime reload`: reload the running Mihomo instance from the synced runtime file
- `mihomo-cli runtime mode <rule|global|direct>`: update local runtime files, keep Clash Verge's `profiles/Merge.yaml` aligned where available, and reload the controller

This keeps file state and process state together, which is especially useful when we want Clash Verge to degrade into "subscription fetch + tray shell" rather than the authoritative control plane.

For Windows, the preparatory pieces are now in place:
- `AppPaths` uses native Windows config/cache roots for `mihomocli`
- Clash Verge path detection probes `%APPDATA%` and `%LOCALAPPDATA%`
- `doctor` reads WinINET proxy settings from the registry
- `runtime` is prepared to rely on the local HTTP controller instead of a unix socket

### Fake‑IP Modes and Bypass

- Modes (Mihomo DNS in `enhanced-mode: fake-ip`):
  - Blacklist: use fake‑ip for all domains except those in `dns.fake-ip-filter`.
  - Whitelist: only domains in `dns.fake-ip-filter` use fake‑ip; others resolve real IPs.

- Recommended for “指定的域名不走 fake‑ip”:
  - `--fake-ip-bypass <PATTERN>`: Append exemptions to `dns.fake-ip-filter` and ensure `fake-ip-filter-mode: blacklist`. Repeatable.
  - Examples: `--fake-ip-bypass '+.example.com' --fake-ip-bypass 'hs.example.com'`.

- Advanced (optional):
  - `--fake-ip-filter-add <PATTERN>`: Append entries to `dns.fake-ip-filter` without changing mode.
  - `--fake-ip-filter-mode <blacklist|whitelist>`: Explicitly set `dns.fake-ip-filter-mode`.

- Validate at runtime (with Mihomo running and DNS hooked):
  - `getent ahosts hs.example.com`
    - Exempted: returns real public IPs (not in `198.18.0.0/16`).
    - Not exempted: returns an IP inside `198.18.0.0/16` (default `fake-ip-range`).

### Dry‑Run Summary Example

Preview what would be generated, without writing the file:

```bash
mihomo-cli merge \
  -s https://example.com/sub.yaml \
  --fake-ip-bypass '+.example.com' \
  --fake-ip-bypass 'hs.example.com' \
  --dev-rules-via Proxy \
  --dry-run
```

### Site-owned Tailscale defaults

Keep tailnet and DERP values in `~/.config/mihomocli/app.yaml` so refresh flows
do not depend on shell scripts or hardcoded examples:

```yaml
tailscale_compat_defaults:
  tailnet_suffixes:
    - example.com
  direct_domains:
    - hs.example.com
    - derp.example.com
  route_exclude_address:
    - 203.0.113.10/32
```

When `tailscale-compatible` is enabled, `mihomo-cli` now treats Tailscale in
two layers:

- Cross-platform Mihomo behavior:
  - add fake-ip bypass entries for the configured tailnet suffixes/direct domains
  - add `DIRECT` domain rules for those names
  - add tailnet/service `DIRECT` rules for `100.64.0.0/10`, `100.100.100.100/32`, and `fd7a:115c:a1e0::/48`
  - keep those same CIDRs in `tun.route-exclude-address`
- Platform adapters:
  - macOS: sync tailnet-only hostnames plus `100.100.100.100` into system proxy bypass domains
  - Windows: sync tailnet-only hostnames plus `100.100.100.100` into WinINET `ProxyOverride`
  - Linux: no automatic system-proxy mutation (no single desktop-wide standard)

Typical output:

```
dry-run summary:
- proxies: 182, groups: 9, rules: 1203
- fake-ip: mode=blacklist, filter+=2 (requested), total=15
- dev-rules: enabled=true, via=Proxy, added=36
- external-controller: 127.0.0.1:9097, secret=unset
- output: would write to /home/<you>/.config/mihomocli/output/clash-verge.yaml (suppressed by --dry-run)
```

## Cache and Quick Rules

- Cache last subscription URL:
  - Show: `mihomo-cli manage cache show`
  - Clear: `mihomo-cli manage cache clear`
  - Reuse cached URL explicitly: pass `--use-last` to `merge` when no `-s/--subscription` is given.

- Quick custom rules (prepend to rules so they take precedence):
  - Add: `mihomo-cli manage custom add --domain cache.nixos.org --via proxy --kind suffix`
  - Add (DIRECT): `mihomo-cli manage custom add --domain cache.nixos.org --kind suffix --via direct`
  - List: `mihomo-cli manage custom list`
  - Remove: `mihomo-cli manage custom remove --domain cache.nixos.org --via proxy`
  - Check: `mihomo-cli manage check --domain github.com`  # prints `proxy` or `direct`
  - Dev domains list: `mihomo-cli manage dev-list [--format plain|yaml|json]`

## Recommended Daily Flow

After you click refresh for the active subscription in Clash Verge, run:

```bash
cd ~/code/mihomocli
nix develop -c cargo run -p mihomo-cli -- refresh-clash-verge
```

This command:
- reads the currently active Clash Verge remote subscription URL from `profiles.yaml`
- regenerates the final config with your enhanced dev rules
- backs up the existing Clash Verge runtime config
- syncs the generated YAML back into Clash Verge
- keeps the platform-specific Clash Verge path detection inside Rust code instead of shell glue

If you still have older automation calling `./scripts/refresh_clash_verge.sh`, it now only forwards to the CLI command and prints a deprecation notice.

## Repository Layout
- `crates/core`: Clash models, merge logic, subscription parsing, storage helpers
- `crates/cli`: Command-line interface, argument handling, file deployment (current front-end)
- `examples/`: Example template/subscription files for local testing
- `resources/`: Base-config reference and documentation
- `SPEC.md`: Project specification and requirements
- `AGENTS.md`: Contributor guide tailored for automation/agents

## Acknowledgements
Huge thanks to the [clash-verge-rev](https://github.com/clash-verge-rev/clash-verge-rev) project for the original merge semantics and resource workflow that inspired this CLI.

## Dev Workflow (flake + fmt + tests)

- Enter flake dev: `nix develop`
- Format: `nix develop -c cargo fmt`
- Lint: `nix develop -c cargo clippy --all-targets --all-features`
- Tests: `nix develop -c cargo test -p mihomo-core`
- E2E (local example):
  - `mihomo-cli merge --template examples/default.yaml --subscription examples/subscription.yaml --stdout`
- E2E (provider URL):
  - Use your real URL locally (do not commit). To reuse the cached last URL explicitly, add `--use-last` to `merge` without `-s`.
  - Align output with CVR by adding `--base-config /path/to/clash-verge.yaml` or using `examples/cvr_template.yaml`.
