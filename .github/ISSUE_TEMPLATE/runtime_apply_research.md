---
name: Runtime apply mechanism (CVR parity)
about: Research and implement Mihomo runtime apply (HTTP API vs reload)
title: "feat(core/cli): implement runtime apply via external-controller or reload"
labels: ["enhancement", "research"]
assignees: []
---

## Goal
- Determine how CVR applies runtime changes to Mihomo (HTTP API vs YAML reload), and add an opt-in path in the CLI to apply configs live.

## Background
- SPEC envisions an HttpDeployer. Current implementation uses FileDeployer only.

## Tasks
- [ ] Review CVR repo/docs; verify API endpoints (e.g., `/configs`, `/reload`).
- [ ] Implement `HttpDeployer { endpoint, secret }` in core; `deploy(&self, yaml)` uploads or triggers reload.
- [ ] Add CLI: `mihomo-cli deploy http --endpoint ... --secret ... --file ...` or `merge --apply-http --endpoint ... --secret ...`.
- [ ] Add dry-run, timeouts, and clear errors. Do not log secrets.
- [ ] Docs for enabling external-controller and expected behavior.

## Acceptance
- Able to apply merged config to a running Mihomo via endpoint with optional secret.
- Or trigger reload for file-based deployment.
- Proper error handling; no corruption of local files.

## Risks
- API differences among Clash/Mihomo variants.
- Security considerations for secret handling.

