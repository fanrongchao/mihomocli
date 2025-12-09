# Resources

- `base-config.example.yaml`: Skeleton file describing the expected structure of a base configuration. Copy your actual clash config (e.g., `clash-verge.yaml`) here or supply it via `--base-config` when running `mihomo-cli merge` to inherit ports/DNS/rules/proxy-groups.
- Runtime assets (`Country.mmdb`, `geoip.dat`, `geosite.dat`) are stored under `~/.config/mihomocli/resources/`.

## Manual preload (to avoid first-run stalls)

On servers with limited GitHub access, manually preload the three files so the CLI won't download them during `merge`:

```bash
mkdir -p ~/.config/mihomocli/resources
curl -L -o ~/.config/mihomocli/resources/Country.mmdb \
  https://ghproxy.com/https://github.com/MetaCubeX/meta-rules-dat/releases/download/latest/country.mmdb
curl -L -o ~/.config/mihomocli/resources/geoip.dat \
  https://ghproxy.com/https://github.com/MetaCubeX/meta-rules-dat/releases/download/latest/geoip.dat
curl -L -o ~/.config/mihomocli/resources/geosite.dat \
  https://ghproxy.com/https://github.com/MetaCubeX/meta-rules-dat/releases/download/latest/geosite.dat
```

Built-in default sources:
- Country.mmdb: `https://github.com/MetaCubeX/meta-rules-dat/releases/download/latest/country.mmdb`
- geoip.dat: `https://github.com/MetaCubeX/meta-rules-dat/releases/download/latest/geoip.dat`
- geosite.dat: `https://github.com/MetaCubeX/meta-rules-dat/releases/download/latest/geosite.dat`

Alternative sources (compatible format):
- Country.mmdb: `https://github.com/P3TERX/GeoLite.mmdb/releases/latest/download/Country.mmdb`
- geoip/geosite: `https://github.com/Loyalsoldier/v2ray-rules-dat/releases/latest/download/{geoip.dat|geosite.dat}`
