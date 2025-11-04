# Repository Guidelines

## Project Structure & Module Organization
The workspace centers around `Cargo.toml` with two members: `crates/core` for domain logic and a front-end crate. In this repository the front-end is a CLI at `crates/cli` (a TUI can be added later). Core exposes modules such as `model.rs`, `subscription.rs`, `template.rs`, `merge.rs`, `output.rs`, and `storage.rs`; keep new shared utilities here. The CLI crate owns argument parsing and orchestration. Place reusable examples or starter templates in `examples/`, and reserve `~/.config/mihomo-tui` and `~/.cache/mihomo-tui` for runtime artifacts only.

Resource bundles (`Country.mmdb`, `geoip.dat`, `geosite.dat`) mirror clash-verge-rev behaviour and live under `~/.config/mihomo-tui/resources/`; refresh logic should stay compatible with upstream URLs.

CLI subscriptions recognise both Clash-ready YAML and the typical base64/share-link formats (trojan/vmess/shadowsocks) when explicitly enabled. Parsing helpers live in `crates/core/src/subscription/parser.rs`; extend them if upstream adds new link types.

UA behavior: the CLI sends `clash-verge/v2.4.2` as the default HTTP User-Agent when fetching subscriptions to coax providers into returning full Clash YAML (often with extensive DOMAIN-SUFFIX rules). Override with `--subscription-ua` if necessary.

When working alongside an existing clash-verge-rev setup, developers can point the
CLI at an exported config via `--base-config`; `merge::apply_base_config` reuses
ports/dns/rules/group metadata so the generated YAML mirrors the upstream final
output.

- `TODO`: Supporting full clash-verge-rev behaviour (without requiring a user-supplied
  base config) would involve porting its profile enhancements, rule templates, and
  runtime config merging logic. For now, rely on `~/.config/mihomo-tui/base-config.yaml`.

## Build, Test, and Development Commands
- `cargo build` compiles the entire workspace. Use `cargo build -p core` or `-p mihomo-cli` for crate-specific checks.
- `cargo run -p mihomo-cli -- merge ...` runs the CLI. Example:

  `cargo run -p mihomo-cli -- merge --template examples/default.yaml --subscription https://example.com/sub.yaml`

- `cargo test -p core` executes unit tests, especially the merge logic and subscription parsing.
- `cargo fmt` and `cargo clippy --all-targets --all-features` enforce formatting and linting before review.

## Coding Style & Naming Conventions
Stick to Rust 2021 idioms with 4-space indentation and `snake_case` for modules, functions, and fields. Prefer descriptive struct names (`Subscription`, `FileDeployer`). Use `rustfmt` defaults; never hand-edit generated formatting. Keep public APIs documented with `///` comments when behaviour is non-trivial. Log actionable events through `tracing` with structured fields.

## Testing Guidelines
Unit tests live beside implementation files in `crates/core/src`. Cover merge behaviour (ports, proxies, proxy groups) and subscriptions parsing edge cases (including base64/share-link lists). Name tests with `test_merge_ports`-style clarity. Run the full suite via `cargo test` before submitting.

## Commit & Pull Request Guidelines
Adopt Conventional Commits (`feat:`, `fix:`, `refactor:`, `chore:`) to describe intent. Scope commits narrowly—configuration paths and merge logic should land separately. Pull requests must summarise changes, note affected config directories (`~/.config/mihomo-tui`, `~/.cache/mihomo-tui`), and call out manual verification (e.g., `cargo run -p tui`). Attach screenshots only when UI layout changes; otherwise paste terminal output. Link related issues and describe follow-ups if work is partial.

## Configuration Tips
Ensure code auto-creates paths such as `~/.config/mihomo-tui/templates/` and `~/.config/mihomo-tui/output/config.yaml`. Never commit user-specific credentials or cached subscription files. Document any new environment variables or feature flags in `SPEC.md` or an adjacent README update.
