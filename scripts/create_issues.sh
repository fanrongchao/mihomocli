#!/usr/bin/env bash
set -euo pipefail

# Requires GitHub CLI (`gh`) authenticated for this repo.

TITLE1="feat(cli): runtime options (mimic CVR)"
BODY1=$(sed -n '1,$p' .github/ISSUE_TEMPLATE/cli_runtime_options.md | sed '1,/^---$/d')
LABELS1="enhancement,cli"

TITLE2="feat(core/cli): implement runtime apply via external-controller or reload"
BODY2=$(sed -n '1,$p' .github/ISSUE_TEMPLATE/runtime_apply_research.md | sed '1,/^---$/d')
LABELS2="enhancement,research"

gh issue create --title "$TITLE1" --label "$LABELS1" --body "$BODY1"
gh issue create --title "$TITLE2" --label "$LABELS2" --body "$BODY2"

echo "Created two issues via gh CLI."

