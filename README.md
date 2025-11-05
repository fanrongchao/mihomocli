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
```bash
# Build
# Recommended: enter flake dev environment, then build
nix develop -c cargo build -p mihomo-cli

# Merge with template and remote subscription (output defaults to
# ~/.config/mihomocli/output/config.yaml). The default User-Agent is
# 'clash-verge/v2.4.2', which often yields Clash YAML with rules.
mihomo-cli merge \
  --template examples/template.yaml \
  --subscription "https://example.com/subscription"

# If your provider only returns base64/share-link lists, explicitly allow it:
mihomo-cli merge \
  --template examples/template.yaml \
  --subscription "https://example.com/base64" \
  --subscription-allow-base64

# Want clash-verge-rev parity? Drop your clash-verge.yaml at
# ~/.config/mihomocli/base-config.yaml (or pass --base-config) so ports/dns/
# rules/groups are inherited automatically.
```

## End-to-End Test (UA feature)

Some providers return a richer Clash YAML (including many DOMAIN-SUFFIX rules) only when the HTTP User-Agent matches known clients. The CLI defaults to `clash-verge/v2.4.2`, which usually triggers full outputs.

Example with a real provider URL (adjust to your environment/network):

```bash
mihomo-cli merge \
  --template examples/default.yaml \
  --subscription "https://example.com/sub.yaml"

# If you need to override UA for debugging:
mihomo-cli merge \
  --template examples/default.yaml \
  --subscription "https://example.com/sub.yaml" \
  --subscription-ua "clash-verge/v2.4.2"
```

Expected results:
- Resources `Country.mmdb`, `geoip.dat`, `geosite.dat` are auto-downloaded into `~/.config/mihomocli/resources/` if missing.
- Output written to `~/.config/mihomocli/output/config.yaml` unless `--stdout` is used.
- Merged YAML contains many DOMAIN-SUFFIX rules from the provider.

## CVR‑Aligned Template (no base-config)

To get output structurally matching Clash Verge Rev without supplying `--base-config`, use the provided CVR‑aligned template:

```bash
mihomo-cli merge \
  --template examples/cvr_template.yaml \
  --subscription "https://example.com/sub.yaml"
```

Notes:
- The template mirrors CVR runtime settings (mixed-port, tun, dns, profile, etc.).
- It leaves `proxy-groups` and `rules` empty so the provider subscription defines them fully.
- `secret` is empty by default; set it in your own copy if needed.

Run Mihomo with the generated configuration and resources:

```bash
mihomo -d ~/.config/mihomocli/resources -f ~/.config/mihomocli/output/config.yaml
```

## CLI Flags of Interest

- `--subscription-ua <STRING>`: HTTP User-Agent used to fetch subscriptions. Default: `clash-verge/v2.4.2`.
- `--subscription-allow-base64`: Enable decoding base64/share-link subscriptions (trojan/vmess/ss). Disabled by default to prefer native Clash YAML from providers.

## Cache and Quick Rules

- Cache last subscription URL:
  - Show: `mihomo-cli manage cache show`
  - Clear: `mihomo-cli manage cache clear`
  - Reuse cached URL explicitly: pass `--use-last` to `merge` when no `-s/--subscription` is given.

- Quick custom rules (prepend to rules so they take precedence):
  - Add: `mihomo-cli manage custom add --domain cache.nixos.org --via Proxy --kind suffix`
  - List: `mihomo-cli manage custom list`
  - Remove: `mihomo-cli manage custom remove --domain cache.nixos.org --via Proxy`

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
- Format: `cargo fmt`
- Lint: `cargo clippy --all-targets --all-features`
- Tests: `cargo test -p mihomo-core`
- E2E (local example):
  - `mihomo-cli merge --template examples/default.yaml --subscription examples/subscription.yaml --stdout`
- E2E (provider URL):
  - Use your real URL locally (do not commit). To reuse the cached last URL explicitly, add `--use-last` to `merge` without `-s`.
  - Align output with CVR by adding `--base-config /path/to/clash-verge.yaml` or using `examples/cvr_template.yaml`.
