# Resources

- `base-config.example.yaml`: Skeleton file describing the expected structure of a base configuration. Copy your actual clash config (e.g., `clash-verge.yaml`) here or supply it via `--base-config` when running `mihomo-cli merge` to inherit ports/DNS/rules/proxy-groups.
- Runtime assets (`Country.mmdb`, `geoip.dat`, `geosite.dat`) are downloaded automatically to `~/.config/mihomo-tui/resources/` when the CLI runs; keep this directory documented if relocating assets.
