#!/usr/bin/env bash
# Install the perf-ledger preflight as an enforced pre-commit hook.
#
# `.git/hooks/` is not version-controlled, so the ratchet does not survive a fresh
# clone on its own. Run this once per checkout. Idempotent.
#
# Installs additively into the mcp-agent-mail chain-runner directory when present
# (hooks.d/pre-commit/), so it composes with the agent-mail file-reservation guard
# instead of replacing it. Falls back to a direct hook when the chain-runner is absent.
set -euo pipefail

REPO="$(git rev-parse --show-toplevel)"
SRC_DOC="$REPO/scripts/perf_candidate_preflight.py"
CHAIN_DIR="$REPO/.git/hooks/hooks.d/pre-commit"
HOOK_NAME="60-perf-ledger-preflight.sh"

[ -f "$SRC_DOC" ] || { echo "missing $SRC_DOC" >&2; exit 1; }
chmod +x "$SRC_DOC"

read -r -d '' HOOK_BODY <<'EOF' || true
#!/usr/bin/env bash
# Perf-ledger preflight (campaign perf-campaign-20260725, Meta-Lever #1 ratchet).
# Refuses an inadmissible verdict in either performance-ledger document.
set -uo pipefail
REPO="$(git rev-parse --show-toplevel)"
PREFLIGHT="$REPO/scripts/perf_candidate_preflight.py"
[ -x "$PREFLIGHT" ] || {
  echo "pre-commit: BLOCKED — required perf-ledger preflight is missing or non-executable: $PREFLIGHT" >&2
  exit 1
}
STAGED_PATHS="$(git diff --cached --name-only)" || exit 1
if printf '%s\n' "$STAGED_PATHS" |
  grep -Eq '^scripts/perf_candidate_preflight\.py$'; then
  python3 "$PREFLIGHT" --self-test || {
    echo "pre-commit: BLOCKED — perf-ledger preflight self-test failed."
    exit 1
  }
fi
if ! printf '%s\n' "$STAGED_PATHS" |
  grep -Eq '^docs/(NEGATIVE_EVIDENCE|LEDGER_RESURRECTION)\.md$'; then
  exit 0
fi
python3 "$PREFLIGHT" --check-new-rows --cached --base HEAD
rc=$?
if [ "$rc" -ne 0 ]; then
  echo ""
  echo "pre-commit: BLOCKED by perf-ledger preflight."
  echo "REJECT requires A/A or a counted mechanism; kept rows require executing-ELF/A/A/median-CI provenance and an exact maintenance-self-speedup/incumbent-win class."
  exit 1
fi
exit 0
EOF

if [ -d "$CHAIN_DIR" ]; then
  printf '%s\n' "$HOOK_BODY" > "$CHAIN_DIR/$HOOK_NAME"
  chmod +x "$CHAIN_DIR/$HOOK_NAME"
  echo "installed: $CHAIN_DIR/$HOOK_NAME (chained after the agent-mail guard)"
else
  HOOK="$REPO/.git/hooks/pre-commit"
  if [ -e "$HOOK" ] && ! grep -q 'perf-ledger preflight' "$HOOK"; then
    echo "refusing to overwrite an existing $HOOK that is not ours." >&2
    echo "move it to .git/hooks/hooks.d/pre-commit/ or chain manually." >&2
    exit 1
  fi
  printf '%s\n' "$HOOK_BODY" > "$HOOK"
  chmod +x "$HOOK"
  echo "installed: $HOOK"
fi

echo "verify with: python3 scripts/perf_candidate_preflight.py --self-test"
