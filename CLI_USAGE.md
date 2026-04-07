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

If you use `direnv`/`nix-direnv` locally, keep your own allowlist and shell hook as preferred. The project no longer relies on a checked-in `.envrc`; the canonical entrypoint remains `nix develop -c ...`.

Windows note:

```powershell
cargo build --release -p mihomo-cli
.\target\release\mihomo-cli.exe doctor
.\target\release\mihomo-cli.exe refresh-clash-verge
```

Windows is expected to run without Nix. The CLI now prepares for that by:
- using native Windows config/cache roots for `mihomocli`
- probing Clash Verge under `%APPDATA%` / `%LOCALAPPDATA%`
- reading WinINET system proxy state from the registry in `doctor`

## Command Overview

Get top-level help and per-command details directly from the binary:

```bash
mihomo-cli --help
mihomo-cli merge --help
mihomo-cli init --help
mihomo-cli doctor --help
mihomo-cli refresh-clash-verge --help
mihomo-cli runtime --help
```

### `merge`

Combine a template with configured subscriptions and optional ad-hoc sources.

```
mihomo-cli merge --template <template_path> [OPTIONS]
```

Key flags:
- `--template <PATH>`: Optional template YAML file. Defaults to the bundled `cvr_template.yaml` under `~/.config/mihomocli/templates/`.
- `--base-config <PATH>`: Optional Clash config whose ports/dns/rules/group metadata should be inherited (e.g., `clash-verge.yaml`). If omitted, the CLI first checks `~/.config/mihomocli/base-config.yaml`, then auto-detects a local Clash Verge exported config.
- `--subscriptions-file <PATH>`: Custom subscriptions list (defaults to `~/.config/mihomocli/subscriptions.yaml`).
- `-s, --subscription <SRC>`: Extra source (URL or local YAML). Repeatable.
- `--output <PATH>`: Destination for merged config. Defaults to `~/.config/mihomocli/output/clash-verge.yaml`.
- `--mode <rule|global|direct>`: Final Clash mode. Defaults to `rule`.
- `--sniffer-preset <tun|off>`: Transparent traffic sniffer preset. Defaults to `tun`.
- `--stdout`: Print merged YAML to stdout instead of writing to disk.
- `--sync-to-clash-verge`: After writing the normal output file, auto-detect Clash Verge's local `config.yaml`, back it up, and replace it with the generated result.
- `--sync-to-clash-verge-sources`: Also update Clash Verge source files such as `dns_config.yaml` and `profiles/Merge.yaml` so future runtime regenerations keep the same DNS/tun settings.
- `--no-dev-rules`: Disable the default proxy-rule injection for common developer registries and slow infra endpoints (GitHub/GitLab, Go module proxies, npm/yarn/pnpm, PyPI, crates.io, Kubernetes/k3s/Vultr, Docker/GCR, `cache.nixos.org`, `channels.nixos.org`, `cachix.org`, mainstream AI agent APIs like OpenAI/Anthropic/Gemini/Cursor/OpenRouter, etc.).
- `--dev-rules-via <NAME>`: Proxy/group tag used by the generated dev rules (default: `Proxy`). If the default `Proxy` is not present, the CLI auto-falls back to a present group (preferring `🚀 节点选择`), then the first group, then the first proxy, and finally `DIRECT`.
- `--dev-rules-show`: Print the generated dev rule list (even without applying it).
- `--subscription-ua <STRING>`: HTTP User-Agent used when fetching subscriptions. Default: `clash-verge/v2.4.2`.
- `--subscription-allow-base64`: Enable decoding base64/share-link lists (trojan/vmess/ss). Disabled by default to prefer provider-native Clash YAML.
- `--use-last`: Reuse the cached last subscription URL when no `-s/--subscription` is provided.
 - `--external-controller-url <HOST>`: Host/IP for the external controller (e.g., `0.0.0.0`).
 - `--external-controller-port <PORT>`: Port for the external controller (e.g., `9090`).
 - `--external-controller-secret <SECRET>`: Secret for the external controller API.
- `--fake-ip-filter-add <PATTERN>`: Append entries to `dns.fake-ip-filter` (useful to bypass DNS hijacking when `dns.enhanced-mode: fake-ip`). Repeatable. Examples: `--fake-ip-filter-add '+.example.com' --fake-ip-filter-add 'hs.example.com'`.
- `--fake-ip-filter-mode <MODE>`: Set `dns.fake-ip-filter-mode` to `blacklist` or `whitelist`.
- `--fake-ip-bypass <PATTERN>`: Clearer shorthand for exemptions. Appends to `dns.fake-ip-filter` and ensures `fake-ip-filter-mode: blacklist`. Repeatable. Use this when you want specified domains not to use fake‑ip, e.g., `--fake-ip-bypass '+.example.com'`.
- `--k8s-cidr-exclude <CIDR>`: Append CIDRs to `tun.route-exclude-address` (repeatable). Use this for Kubernetes Pod/Service CIDRs to avoid tun-mode hijacking. Defaults include `10.42.0.0/16` and `10.43.0.0/16`.
- `--route-exclude-address-add <CIDR>`: Append arbitrary CIDRs to `tun.route-exclude-address` (repeatable). Use this for specific remote IPs/subnets that must not go through mihomo TUN, such as a self-hosted DERP IP.
- `--tailscale-compatible`: Keep fake-ip and tun compatible with Tailscale by moving unsafe fake-ip ranges off `198.18.0.0/16`, bypassing Tailscale domains from fake-ip, and excluding tailnet CIDRs from tun routing.
- `--tailscale-tailnet-suffix <SUFFIX>`: Add a custom tailnet suffix so `tail.<suffix>` is also bypassed from fake-ip and forced `DIRECT`. Repeatable.
- `--tailscale-direct-domain <DOMAIN>`: Add extra domains or suffixes that should bypass fake-ip and be forced `DIRECT` under `--tailscale-compatible`. Repeatable. Examples: `--tailscale-direct-domain derp.example.com` or `--tailscale-direct-domain +.corp.example.com`.
 - `--dry-run`: Do not write output; print a concise summary (proxies/groups/rules counts, fake‑ip mode + number of bypass entries requested, dev‑rules via and count, external-controller presence).

### `init`

Create runtime directories under `~/.config/mihomocli` and seed the bundled CVR‑aligned template if missing.

```
mihomo-cli init
```

What it does:
- Ensures: `~/.config/mihomocli/`, `templates/`, `resources/`, `output/`, and cache dirs exist
- Seeds: `~/.config/mihomocli/templates/cvr_template.yaml` if not present
- Does not download resources to avoid first-run network stalls

### `doctor`

Inspect the current local desktop state without changing anything.

```bash
mihomo-cli doctor
```

What it reports:
- local Clash Verge runtime file state (`mode`, `tun`, `sniffer`, fake-ip range, route excludes)
- whether `config.yaml` and `clash-verge.yaml` currently agree on the important runtime fields
- system proxy status via `scutil --proxy` on macOS or WinINET registry keys on Windows
- Tailscale CLI status and health warnings when available
- controller connectivity and a live connection sample when the Mihomo controller is reachable

Useful flags:
- `--show-connections`: Include a short live controller connection sample. Defaults to on.
- `--focus-domain <DOMAIN>`: Highlight specific domains in the live connection sample. Repeatable.

Example:

```bash
mihomo-cli doctor \
  --focus-domain chatgpt.com \
  --focus-domain api.anthropic.com
```

This is especially useful when you want to answer:
- Is the machine currently in `rule + tun + sniffer`?
- Are system proxies still enabled?
- Is the current app session entering Mihomo via `DEFAULT-MIXED` or via TUN?

### `runtime`

Operate on the locally detected Mihomo / Clash Verge runtime without opening the GUI.

```bash
mihomo-cli runtime status
```

Available actions:
- `mihomo-cli runtime status`: Show the detected runtime files, current `mode`, `tun`, `sniffer`, whether `config.yaml` and `clash-verge.yaml` are aligned, the source `profiles/Merge.yaml` mode, and the live controller summary when reachable.
- `mihomo-cli runtime reload`: Ask the controller to reload from the detected runtime file, using the same `config.yaml`/`clash-verge.yaml` that `refresh-clash-verge` writes.
- `mihomo-cli runtime mode <rule|global|direct>`: Update the detected runtime files to the target mode, also keep `profiles/Merge.yaml` aligned where available, then reload the controller.

On Windows, `runtime` is designed to work without a unix socket. It will use the local HTTP `external-controller` that Clash Verge exports.

Examples:

```bash
# See the current runtime/controller state
mihomo-cli runtime status

# Switch the local desktop runtime to rule mode without opening Clash Verge
mihomo-cli runtime mode rule

# Reload the currently synced runtime after manual edits
mihomo-cli runtime reload
```

### `refresh-clash-verge`

Refresh the local Clash Verge setup using the currently active remote subscription from Clash Verge itself.

```bash
mihomo-cli refresh-clash-verge
```

What it does:
- detects Clash Verge's `profiles.yaml`
- resolves the active remote subscription URL
- runs the native CLI refresh flow
- enables `--sync-to-clash-verge` and `--sync-to-clash-verge-sources`
- applies the usual desktop defaults of `--mode rule` and `--sniffer-preset tun`
- explicitly restores `tun.enable: true` in the generated/runtime config so a Clash Verge reinstall or GUI drift can be pulled back to the expected desktop state in one command

Useful flags:
- `mihomo-cli refresh-clash-verge "https://example.com/sub.yaml"`: override the detected subscription URL explicitly
- `--mode <rule|global|direct>`: override the final mode
- `--sniffer-preset <tun|off>`: override the sniffer preset
- `--tailscale-tailnet-suffix <SUFFIX>`: repeatable
- `--tailscale-direct-domain <DOMAIN>`: repeatable
- `--route-exclude-address-add <CIDR>`: repeatable
- `--no-tailscale-compatible`: disable the Tailscale compatibility patch set
- `--dry-run`: preview the merge without writing files

Environment fallbacks kept for compatibility with earlier shell-based usage:
- `MIHOMOCLI_TAILSCALE_SUFFIXES` or `MIHOMOCLI_TAILSCALE_SUFFIX`
- `MIHOMOCLI_TAILSCALE_DIRECT_DOMAINS`
- `MIHOMOCLI_TAILSCALE_DIRECT_CIDRS`
- `MIHOMOCLI_MODE`
- `MIHOMOCLI_SNIFFER_PRESET`

Recommended long-term config source:

```yaml
# ~/.config/mihomocli/app.yaml
tailscale_compat_defaults:
  tailnet_suffixes:
    - example.com
  direct_domains:
    - hs.example.com
    - derp.example.com
  route_exclude_address:
    - 203.0.113.10/32
```

With that in place, `refresh-clash-verge` can stay zero-argument while tailnet
and DERP values remain site-owned rather than hardcoded in scripts.

Self-recovery command after Clash Verge reinstall or runtime drift:

```bash
cd ~/code/mihomocli
nix develop -c cargo run -p mihomo-cli -- refresh-clash-verge
```

That command is expected to restore:
- the active subscription content
- desktop defaults like `mode=rule` and `sniffer=tun`
- `tun.enable=true`
- Clash Verge runtime/source file alignment

## Recommended One-Command Refresh

After refreshing the subscription inside Clash Verge itself, run:

```bash
cd ~/code/mihomocli
nix develop -c cargo run -p mihomo-cli -- refresh-clash-verge
```

By default the command reads the current remote subscription URL from Clash Verge's
local `profiles.yaml` and then runs the equivalent of:

```bash
nix develop -c cargo run -p mihomo-cli -- merge --subscription "<current-url>" --mode rule --sniffer-preset tun --tailscale-compatible --sync-to-clash-verge --sync-to-clash-verge-sources
```

For a Tailscale-safe refresh that also updates Clash Verge source files:

```bash
nix develop -c cargo run -p mihomo-cli -- refresh-clash-verge --tailscale-tailnet-suffix example.com --tailscale-direct-domain derp.example.com --route-exclude-address-add 203.0.113.10/32
```

To explicitly force the common desktop setup of `rule + tun sniffer`:

```bash
nix develop -c cargo run -p mihomo-cli -- refresh-clash-verge --mode rule --sniffer-preset tun
```

After refresh, use `runtime` for GUI-free day-to-day operations:

```bash
# Inspect runtime state
nix develop -c cargo run -p mihomo-cli -- runtime status

# Flip modes without touching the GUI
nix develop -c cargo run -p mihomo-cli -- runtime mode global
nix develop -c cargo run -p mihomo-cli -- runtime mode rule
```

You can also override the subscription URL explicitly:

```bash
nix develop -c cargo run -p mihomo-cli -- refresh-clash-verge "https://example.com/sub.yaml"
```

If you still have old automation calling `./scripts/refresh_clash_verge.sh`, it now only forwards to this CLI command and prints a deprecation notice.

## Configuration Files

Runtime directories (auto-created):
- Templates: `~/.config/mihomocli/templates/` (auto-populated with `cvr_template.yaml` on first run)
- Subscriptions list: `~/.config/mihomocli/subscriptions.yaml`
- Cache: `~/.cache/mihomocli/subscriptions/`
- Output: `~/.config/mihomocli/output/clash-verge.yaml`
- Resources (Country.mmdb, geoip.dat, geosite.dat): `~/.config/mihomocli/resources/` (use `mihomo -d ~/.config/mihomocli/resources ...`)

### Resource mirrors and manual preload

If your environment has trouble reaching GitHub, you can preload the three resource files and the CLI will skip downloading them:

```bash
mkdir -p ~/.config/mihomocli/resources
curl -L -o ~/.config/mihomocli/resources/Country.mmdb \
  https://ghproxy.com/https://github.com/MetaCubeX/meta-rules-dat/releases/download/latest/country.mmdb
curl -L -o ~/.config/mihomocli/resources/geoip.dat \
  https://ghproxy.com/https://github.com/MetaCubeX/meta-rules-dat/releases/download/latest/geoip.dat
curl -L -o ~/.config/mihomocli/resources/geosite.dat \
  https://ghproxy.com/https://github.com/MetaCubeX/meta-rules-dat/releases/download/latest/geosite.dat
```

Built-in sources:
- Country.mmdb: `https://github.com/MetaCubeX/meta-rules-dat/releases/download/latest/country.mmdb`
- geoip.dat: `https://github.com/MetaCubeX/meta-rules-dat/releases/download/latest/geoip.dat`
- geosite.dat: `https://github.com/MetaCubeX/meta-rules-dat/releases/download/latest/geosite.dat`

Mirrors (prefix any of the above): `https://ghproxy.com/`, `https://mirror.ghproxy.com/`, `https://github.moeyy.xyz/`

## Validate with mihomo

You can validate the generated config with the real mihomo binary:

```
mihomo-cli test \
  --mihomo-bin mihomo \
  --mihomo-dir ~/.config/mihomocli \
  --config ~/.config/mihomocli/output/clash-verge.yaml
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
