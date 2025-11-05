# Repository Guidelines

## Project Structure & Module Organization
The workspace centers around `Cargo.toml` with two members: `crates/core` for domain logic and a front-end crate. In this repository the front-end is a CLI at `crates/cli` (a TUI can be added later). Core exposes modules such as `model.rs`, `subscription.rs`, `template.rs`, `merge.rs`, `output.rs`, and `storage.rs`; keep new shared utilities here. The CLI crate owns argument parsing and orchestration. Place reusable examples or starter templates in `examples/`, and reserve `~/.config/mihomocli` and `~/.cache/mihomocli` for runtime artifacts only.

Resource bundles (`Country.mmdb`, `geoip.dat`, `geosite.dat`) mirror clash-verge-rev behaviour and live under `~/.config/mihomocli/resources/`; refresh logic should stay compatible with upstream URLs.

CLI subscriptions recognise both Clash-ready YAML and the typical base64/share-link formats (trojan/vmess/shadowsocks) when explicitly enabled. Parsing helpers live in `crates/core/src/subscription/parser.rs`; extend them if upstream adds new link types.

UA behavior: the CLI sends `clash-verge/v2.4.2` as the default HTTP User-Agent when fetching subscriptions to coax providers into returning full Clash YAML (often with extensive DOMAIN-SUFFIX rules). Override with `--subscription-ua` if necessary.

When working alongside an existing clash-verge-rev setup, developers can point the
CLI at an exported config via `--base-config`; `merge::apply_base_config` reuses
ports/dns/rules/group metadata so the generated YAML mirrors the upstream final
output.

- `TODO`: Supporting full clash-verge-rev behaviour (without requiring a user-supplied
  base config) would involve porting its profile enhancements, rule templates, and
  runtime config merging logic. For now, rely on `~/.config/mihomocli/base-config.yaml`.

## Build, Test, and Development Commands
- Preferred: enter flake dev shell
  - `nix develop`
- Build:
  - `cargo build` (or `cargo build -p mihomo-cli`)
- Run CLI:
  - `cargo run -p mihomo-cli -- merge --template examples/default.yaml --subscription https://example.com/sub.yaml`
- Tests:
  - `cargo test -p mihomo-core`
- Formatting & linting (required before commit):
  - `cargo fmt`
  - `cargo clippy --all-targets --all-features`

## Coding Style & Naming Conventions
Stick to Rust 2021 idioms with 4-space indentation and `snake_case` for modules, functions, and fields. Prefer descriptive struct names (`Subscription`, `FileDeployer`). Use `rustfmt` defaults; never hand-edit generated formatting. Keep public APIs documented with `///` comments when behaviour is non-trivial. Log actionable events through `tracing` with structured fields.

## Testing Guidelines
Unit tests live beside implementation files in `crates/core/src`. Cover merge behaviour (ports, proxies, proxy groups) and subscriptions parsing edge cases (including base64/share-link lists). Name tests with `test_merge_ports`-style clarity. Run the full suite via `cargo test` before submitting. Use real provider URLs locally only; do not commit real URLs in docs or examples.

## Commit & Pull Request Guidelines
Adopt Conventional Commits (`feat:`, `fix:`, `refactor:`, `chore:`) to describe intent. Scope commits narrowly—configuration paths and merge logic should land separately. Pull requests must summarise changes, note affected config directories (`~/.config/mihomocli`, `~/.cache/mihomocli`), and call out manual verification (e.g., `cargo run -p tui`). Attach screenshots only when UI layout changes; otherwise paste terminal output. Link related issues and describe follow-ups if work is partial.

## Configuration Tips
Ensure code auto-creates paths such as `~/.config/mihomocli/templates/` and `~/.config/mihomocli/output/config.yaml`. Never commit user-specific credentials or cached subscription files. Document any new environment variables or feature flags in `SPEC.md` or an adjacent README update.

When adding merge-time conveniences, prefer CLI flags. The existing `--dev-rules` option prepends proxy rules for GitHub/Docker/GCR/cache.nixos.org and should remain stable (adjust targets via `--dev-rules-via`; `--dev-rules-show` prints the default list).

## Cache & Quick Rules (CLI)
- Cached last subscription URL: `mihomo-cli manage cache show|clear`. Reuse it explicitly via `--use-last` when calling `merge` without `-s`.
- Quick custom rules (prepend to rules):
  - Add: `mihomo-cli manage custom add --domain <dom> --via <proxy_or_group> [--kind domain|suffix|keyword]`
  - List: `mihomo-cli manage custom list`
  - Remove: `mihomo-cli manage custom remove --domain <dom> [--via <proxy_or_group>]`
