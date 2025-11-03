# mihomo-cli

`mihomo-cli` is a Rust-based command-line tool for merging Mihomo/Clash subscriptions using the same conventions as [clash-verge-rev](https://github.com/clash-verge-rev/clash-verge-rev). It parses base64/URI share links (trojan/vmess/shadowsocks), merges multiple subscriptions with a template, and can inherit ports, DNS, rules, and proxy-groups from an existing Clash config.

## Features
- Rust workspace with reusable core crate and CLI front-end
- Clash YAML parsing plus share-link decoding
- Template + subscription merge identical to clash-verge-rev semantics
- Optional base-config inheritance (`--base-config`) to reuse existing rules/groups
- Auto-downloads `Country.mmdb`, `geoip.dat`, `geosite.dat` into `~/.config/mihomo-tui/resources/`

## Quick Start
```bash
# Build
cargo build -p mihomo-cli

# Merge with template and subscription share link
mihomo-cli merge \ 
  --template examples/template.yaml \ 
  --subscription "https://example.com/subscription" \ 
  --stdout

# Merge while inheriting an existing clash config (e.g., clash-verge.yaml)
mihomo-cli merge \ 
  --template examples/template.yaml \ 
  --base-config resources/base-config.example.yaml \ 
  --subscription https://example.com/subscription \ 
  --output ~/.config/mihomo-tui/output/config.yaml
```

## Repository Layout
- `crates/core`: Clash models, merge logic, subscription parsing, storage helpers
- `crates/cli`: Command-line interface, argument handling, file deployment
- `examples/`: Example template/subscription files for local testing
- `resources/`: Base-config reference and documentation
- `SPEC.md`: Project specification and requirements
- `AGENTS.md`: Contributor guide tailored for automation/agents

## Acknowledgements
Huge thanks to the [clash-verge-rev](https://github.com/clash-verge-rev/clash-verge-rev) project for the original merge semantics and resource workflow that inspired this CLI.
