# mihomo-cli Usage

`mihomo-cli` merges Mihomo/Clash-compatible subscriptions with a template to produce a final configuration. It reads repository defaults and follows the same merge semantics as clash-verge-rev.

## Install & Build

```bash
cargo build -p mihomo-cli
```

The binary will be at `target/debug/mihomo-cli`.

## Command Overview

### `merge`

Combine a template with configured subscriptions and optional ad-hoc sources.

```
mihomo-cli merge --template <template_path> [OPTIONS]
```

Key flags:
- `--template <PATH>`: Template YAML file. Relative paths resolve under `~/.config/mihomo-tui/templates/`.
- `--subscriptions-file <PATH>`: Custom subscriptions list (defaults to `~/.config/mihomo-tui/subscriptions.yaml`).
- `-s, --subscription <SRC>`: Extra source (URL or local YAML). Repeatable.
- `--output <PATH>`: Destination for merged config. Defaults to `~/.config/mihomo-tui/output/config.yaml`.
- `--stdout`: Print merged YAML to stdout instead of writing to disk.

## Configuration Files

Runtime directories (auto-created):
- Templates: `~/.config/mihomo-tui/templates/`
- Subscriptions list: `~/.config/mihomo-tui/subscriptions.yaml`
- Cache: `~/.cache/mihomo-tui/subscriptions/`
- Output: `~/.config/mihomo-tui/output/config.yaml`

## Examples

Merge using default subscriptions and save to the default output:

```bash
mihomo-cli merge --template default.yaml
```

Preview merged YAML without writing to disk, combining a local subscription and a remote URL:

```bash
mihomo-cli merge \
  --template examples/template.yaml \
  --subscription ~/.config/mihomo-tui/subscription-local.yaml \
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
