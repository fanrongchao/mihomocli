# Repository Guidelines

## Project Structure & Module Organization
The workspace centers around `Cargo.toml` with two members: `crates/core` for domain logic and `crates/tui` for the terminal UI binary. Core exposes modules such as `model.rs`, `subscription.rs`, `template.rs`, `merge.rs`, `output.rs`, and `storage.rs`; keep new shared utilities here. The TUI crate owns `main.rs`, `app.rs`, `ui.rs`, `events.rs`, and the `screens/` directory (`home.rs`, `subscriptions.rs`, `subscription_detail.rs`, `merge_preview.rs`). Place reusable examples or starter templates in `examples/`, and reserve `~/.config/mihomo-tui` and `~/.cache/mihomo-tui` for runtime artifacts only.

Resource bundles (`Country.mmdb`, `geoip.dat`, `geosite.dat`) mirror clash-verge-rev behaviour and live under `~/.config/mihomo-tui/resources/`; refresh logic should stay compatible with upstream URLs.

## Build, Test, and Development Commands
- `cargo build` compiles the entire workspace. Use `cargo build -p core` or `-p tui` for crate-specific checks.
- `cargo run -p tui` launches the TUI with mock data or the current config files.
- `cargo test -p core` executes unit tests, especially the merge logic.
- `cargo fmt` and `cargo clippy --all-targets --all-features` enforce formatting and linting before review.

## Coding Style & Naming Conventions
Stick to Rust 2021 idioms with 4-space indentation and `snake_case` for modules, functions, and fields. Prefer descriptive struct names (`Subscription`, `FileDeployer`). Use `rustfmt` defaults; never hand-edit generated formatting. Keep public APIs documented with `///` comments when behaviour is non-trivial. Log actionable events through `tracing` with structured fields.

## Testing Guidelines
Unit tests live beside implementation files in `crates/core/src`. Cover merge behaviour (ports, proxies, proxy groups) and subscriptions parsing edge cases. Name tests with `test_merge_ports`-style clarity. For TUI components, isolate pure logic behind structs so it can be tested without terminal I/O. Run the full suite via `cargo test` before submitting.

## Commit & Pull Request Guidelines
Adopt Conventional Commits (`feat:`, `fix:`, `refactor:`, `chore:`) to describe intent. Scope commits narrowly—configuration paths and merge logic should land separately. Pull requests must summarise changes, note affected config directories (`~/.config/mihomo-tui`, `~/.cache/mihomo-tui`), and call out manual verification (e.g., `cargo run -p tui`). Attach screenshots only when UI layout changes; otherwise paste terminal output. Link related issues and describe follow-ups if work is partial.

## Configuration Tips
Ensure code auto-creates paths such as `~/.config/mihomo-tui/templates/` and `~/.config/mihomo-tui/output/config.yaml`. Never commit user-specific credentials or cached subscription files. Document any new environment variables or feature flags in `SPEC.md` or an adjacent README update.
