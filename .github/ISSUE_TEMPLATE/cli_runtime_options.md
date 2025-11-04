---
name: CLI runtime options (mimic CVR)
about: Expose CVR-like runtime settings via CLI flags
title: "feat(cli): runtime options (mimic CVR)"
labels: ["enhancement", "cli"]
assignees: []
---

## Goal
- Expose common CVR runtime settings as CLI options so users can tweak output without editing templates/base-config.
- Priority keys: mixed-port, mode, allow-lan, log-level, ipv6, unified-delay, external-controller, secret, external-controller-cors, external-controller-unix, tun.*, dns.*, profile.*

## Design
- Precedence: CLI flags > base-config > template > subscription.
- Phase 1: top-level scalars + enable/disable toggles; Phase 2: nested DNS/TUN or overlay file.

## Tasks
- [ ] Inventory CVR keys reproduced by base-config; pick phase‑1 subset for flags.
- [ ] Add flags to `merge` and map into `ClashConfig.extra` before base apply.
- [ ] Ensure base precedence for keys not provided via CLI; CLI overrides when set.
- [ ] Unit tests for precedence and omission of nulls.
- [ ] Docs (README/CLI_USAGE/AGENTS/SPEC) and examples.

## Acceptance
- Running `mihomo-cli merge --mixed-port 7897 --mode rule --allow-lan --ipv6 --unified-delay --external-controller 127.0.0.1:9097 --secret ''` produces those keys.
- Base-config applied for other fields.
- fmt + tests pass in flake env.

