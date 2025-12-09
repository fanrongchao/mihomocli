# mihomo-cli Usage

`mihomo-cli` merges Mihomo/Clash-compatible subscriptions with a template to produce a final configuration. It reads repository defaults and follows the same merge semantics as clash-verge-rev.

## Install & Build

```bash
cargo build -p mihomo-cli
```

The binary will be at `target/debug/mihomo-cli`.

Tip (Nix dev shell): for a reproducible toolchain with `cargo`, `rustfmt`, and `clippy`, run commands inside the flake dev shell:

```bash
nix develop -c cargo build -p mihomo-cli
nix develop -c cargo fmt
nix develop -c cargo clippy --all-targets --all-features
```

## Command Overview

Get top-level help and per-command details directly from the binary:

```bash
mihomo-cli --help
mihomo-cli merge --help
```

### `merge`

Combine a template with configured subscriptions and optional ad-hoc sources.

```
mihomo-cli merge --template <template_path> [OPTIONS]
```

Key flags:
- `--template <PATH>`: Optional template YAML file. Defaults to the bundled `cvr_template.yaml` under `~/.config/mihomocli/templates/`.
- `--base-config <PATH>`: Optional Clash config whose ports/dns/rules/group metadata should be inherited (e.g., `clash-verge.yaml`). If omitted, the CLI auto-loads `~/.config/mihomocli/base-config.yaml` when present.
- `--subscriptions-file <PATH>`: Custom subscriptions list (defaults to `~/.config/mihomocli/subscriptions.yaml`).
- `-s, --subscription <SRC>`: Extra source (URL or local YAML). Repeatable.
- `--output <PATH>`: Destination for merged config. Defaults to `~/.config/mihomocli/output/config.yaml`.
- `--stdout`: Print merged YAML to stdout instead of writing to disk.
- `--no-dev-rules`: Disable the default proxy-rule injection for common developer registries (GitHub/GitLab, Go module proxies, npm/yarn/pnpm, PyPI, crates.io, Kubernetes/k3s registries, Docker/GCR, cache.nixos.org, mainstream AI agent APIs like OpenAI/Anthropic/Gemini/Cursor/OpenCode, etc.).
- `--dev-rules-via <NAME>`: Proxy/group tag used by the generated dev rules (default: `Proxy`). If the default `Proxy` is not present, the CLI auto-falls back to a present group (preferring `🚀 节点选择`), then the first group, then the first proxy, and finally `DIRECT`.
- `--dev-rules-show`: Print the generated dev rule list (even without applying it).
- `--subscription-ua <STRING>`: HTTP User-Agent used when fetching subscriptions. Default: `clash-verge/v2.4.2`.
- `--subscription-allow-base64`: Enable decoding base64/share-link lists (trojan/vmess/ss). Disabled by default to prefer provider-native Clash YAML.
- `--use-last`: Reuse the cached last subscription URL when no `-s/--subscription` is provided.
 - `--external-controller-url <HOST>`: Host/IP for the external controller (e.g., `0.0.0.0`).
 - `--external-controller-port <PORT>`: Port for the external controller (e.g., `9090`).
 - `--external-controller-secret <SECRET>`: Secret for the external controller API.

## Configuration Files

Runtime directories (auto-created):
- Templates: `~/.config/mihomocli/templates/` (auto-populated with `cvr_template.yaml` on first run)
- Subscriptions list: `~/.config/mihomocli/subscriptions.yaml`
- Cache: `~/.cache/mihomocli/subscriptions/`
- Output: `~/.config/mihomocli/output/config.yaml`
- Resources (Country.mmdb, geoip.dat, geosite.dat): `~/.config/mihomocli/resources/` (use `mihomo -d ~/.config/mihomocli/resources ...`)

## Validate with mihomo

You can validate the generated config with the real mihomo binary:

```
mihomo-cli test \
  --mihomo-bin mihomo \
  --mihomo-dir ~/.config/mihomocli \
  --config ~/.config/mihomocli/output/config.yaml
```

By default, `mihomo-cli test` uses `mihomo` from `PATH`, `~/.config/mihomocli` as `-d`, and the default output config path.

The CLI accepts Clash YAML subscriptions directly, but it can also decode common
link-based feeds (trojan/vmess/shadowsocks) even when they are delivered via
base64-wrapped subscription URLs.

## Examples

Merge using default subscriptions and save to the default output:

```bash
mihomo-cli merge --template default.yaml
```

Preview merged YAML without writing to disk, combining a local subscription and a remote URL:

```bash
mihomo-cli merge \
  --template examples/template.yaml \
  --subscription ~/.config/mihomocli/subscription-local.yaml \
  --subscription https://example.com/sub.yaml \
  --stdout
```

Use a custom subscriptions file and custom output path:

```bash
mihomo-cli merge \
  --template production.yaml \
  --subscriptions-file ./fixtures/subscriptions.yaml \
  --output ./dist/mihomo.yaml
```

After a successful merge, the CLI updates the subscriptions metadata (ETag, last-modified, last-updated) in the chosen subscriptions file.

### UA-sensitive provider example

Some providers return full Clash YAML (including large DOMAIN-SUFFIX rule sets) only for specific User-Agents. The CLI defaults to a clash-verge UA to coax compatible outputs.

```bash
mihomo-cli merge \
  --template examples/default.yaml \
  --subscription "https://example.com/sub.yaml"

# Override UA if needed
mihomo-cli merge \
  --template examples/default.yaml \
  --subscription "https://example.com/sub.yaml" \
  --subscription-ua "clash-verge/v2.4.2"
```

If your provider only serves share-link/base64 lists, opt in explicitly:

```bash
mihomo-cli merge \
  --template examples/default.yaml \
  --subscription "https://example.com/base64" \
  --subscription-allow-base64
```

### CVR‑aligned template (no base-config required)

Use the provided template that mirrors Clash Verge Rev runtime to align outputs without `--base-config`:

```bash
mihomo-cli merge \
  --template examples/cvr_template.yaml \
  --subscription "https://example.com/sub.yaml"
```

Customize `secret` or controller settings by copying the template to `~/.config/mihomocli/templates/` and editing as needed.
### Cached last URL

Reuse the last successful subscription URL without retyping it:

```bash
mihomo-cli merge \
  --template examples/cvr_template.yaml \
  --use-last
```
