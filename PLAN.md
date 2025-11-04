# Roadmap / TODOs (next iterations)

This document tracks concrete improvements to pick up next time. Items are ordered for incremental delivery, each with clear goals, tasks, and acceptance criteria.

## 1) CLI Runtime Options (mimic CVR)

- Goal
  - Expose common CVR runtime settings as CLI options so users can tweak output without hand‑editing templates/base-config.
  - Priority keys: `mixed-port`, `mode`, `allow-lan`, `log-level`, `ipv6`, `unified-delay`, `external-controller`, `secret`, `external-controller-cors`, `external-controller-unix`, `tun.*`, `dns.*`, `profile.*`.

- Design
  - Precedence: CLI flags > base-config > template > subscription.
  - Scope (phase 1): top-level scalars + enable/disable blocks; (phase 2): nested DNS/TUN structured options.
  - Flag style: top-level direct flags (e.g., `--mixed-port 7897`, `--mode rule`, `--allow-lan`, `--ipv6`, `--unified-delay`, `--external-controller 127.0.0.1:9097`, `--secret ...`).
  - For structured blocks (dns/tun/profile/cors): either
    - (A) accept a YAML overlay file `--runtime-overlay path.yaml`, or
    - (B) provide minimal toggles (e.g., `--dns-enable`, `--tun-enable`) and rely on templates for full shape.

- Tasks
  1. Inventory CVR keys we already reproduce when using base-config; pick the phase‑1 subset for flags.
  2. Add flags to `merge` (CLI), map into `ClashConfig.extra` during merge preparation (before base‑config apply).
  3. Ensure base‑config precedence still applies for keys not provided via CLI; CLI should override base‑config.
  4. Add unit tests for precedence and for omitting nulls (`skip_serializing_if`).
  5. Update README/CLI_USAGE/AGENTS/SPEC with examples and precedence rules.

- Acceptance Criteria
  - Running `mihomo-cli merge --mixed-port 7897 --mode rule --allow-lan --ipv6 --unified-delay --external-controller 127.0.0.1:9097 --secret ''` produces those keys in output regardless of template content.
  - Base-config is still applied for other runtime fields when provided.
  - Formatting and tests pass in flake env.

- Nice‑to‑have (phase 2)
  - `--runtime-overlay path.yaml` to merge a YAML snippet into `extra` (documented merge strategy: deep merge, CLI wins).

## 2) Research CVR Runtime Change Mechanism (apply vs reload)

- Goal
  - Understand and replicate how CVR changes Mihomo runtime: via Mihomo external‑controller API or by writing YAML and asking Mihomo to reload.
  - Provide an opt‑in path in CLI to apply configs live.

- Background
  - SPEC already envisions an `HttpDeployer` (external‑controller). Current implementation only writes files via `FileDeployer`.

- Hypotheses to verify
  - CVR uses Mihomo’s external‑controller HTTP API to push config changes (e.g., `/configs`, `/reload`, or similar endpoints).
  - Alternatively, it rewrites YAML on disk and signals Mihomo to reload (file watch or explicit API call).

- Tasks
  1. Review CVR repo/docs for how it applies runtime changes (API endpoints and payloads). If offline, consult Mihomo/Clash API docs for:
     - POST/PUT `/configs` (apply full config), `PATCH` for partials, and `/configs?force=true` if available.
     - `/reload` or equivalent endpoint to trigger reload from file.
  2. Implement `HttpDeployer` in core (guided by SPEC):
     - Fields: `endpoint` (e.g., `http://127.0.0.1:9090/configs`), optional `secret`.
     - Method: `deploy(&self, yaml)` uploads config (or calls reload endpoint if we choose file‑based).
  3. Add CLI subcommand `deploy http` (or `merge --apply-http`):
     - `mihomo-cli deploy http --endpoint http://127.0.0.1:9090 --secret <...> --file ~/.config/mihomo-tui/output/config.yaml`
     - Or pipeline from merge: `mihomo-cli merge ... --apply-http --endpoint ... --secret ...`.
  4. Add dry‑run flag and timeouts; clear error messages and status output.
  5. Docs: how to enable Mihomo external‑controller and required `secret`, caveats on live edits.

- Acceptance Criteria
  - Able to send a merged config to a running Mihomo via HTTP endpoint with optional secret.
  - Alternatively, able to trigger reload when writing to file only.
  - Clear errors when endpoint unreachable; does not corrupt local files.

- Risks / Open Questions
  - Endpoint/contract differences between Clash/Mihomo variants.
  - Security: handling of `secret` and not logging sensitive data.
  - Partial vs full updates: start with full config apply for simplicity.

## 3) Operational / Dev Workflow

- Enforce `cargo fmt`, run `cargo test -p mihomo-core` in flake env on each change.
- Add a small `examples/e2e.sh` (optional) to run:
  - Local example merges
  - Real URL merge (placeholder; remind to set via env var)
  - Diff against CVR output when a base-config path is present

## References

- Clash/Mihomo external‑controller docs (configs/reload endpoints)
- CVR repository (runtime settings and updates)
- Current repo: `examples/cvr_template.yaml`, `crates/core/src/merge.rs`, `crates/cli/src/main.rs`

