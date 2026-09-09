#!/usr/bin/env bash
# scripts/commit_only_mine.sh — br-frankenpandas-lz1yy
#
# Safe commit wrapper for shared multi-agent checkouts.
# Verifies that only intended files and hunks are being committed, checks that
# deletion count does not exceed expected limits (preventing silent accidental
# reversions of peer work), verifies HEAD has not advanced mid-check, and
# commits atomically.

set -euo pipefail

usage() {
    cat <<'EOF'
Usage: scripts/commit_only_mine.sh [OPTIONS] -m "COMMIT_MESSAGE" [-- PATHS...]

Options:
  -m, --message <MSG>        Commit message (required)
  --max-deletions <NUM>      Maximum permitted deletions across all files (default: 50)
  --allow-deletions          Bypass the maximum deletion threshold check
  --dry-run                  Perform all safety preflight checks without committing
  -h, --help                 Show this help message

Paths:
  Optional list of files/directories. If provided, verifies that ONLY files
  matching these paths are staged.

Examples:
  scripts/commit_only_mine.sh -m "fix(frame): correct null reduction" crates/fp-frame/src/lib.rs
  scripts/commit_only_mine.sh -m "refactor: clean up test utils" --max-deletions 200 -- tests/
EOF
    exit 0
}

MESSAGE=""
MAX_DELETIONS=50
ALLOW_DELETIONS=false
DRY_RUN=false
TARGET_PATHS=()

while [[ $# -gt 0 ]]; do
    case "$1" in
        -m|--message)
            MESSAGE="$2"
            shift 2
            ;;
        --max-deletions)
            MAX_DELETIONS="$2"
            shift 2
            ;;
        --allow-deletions)
            ALLOW_DELETIONS=true
            shift
            ;;
        --dry-run)
            DRY_RUN=true
            shift
            ;;
        -h|--help)
            usage
            ;;
        --)
            shift
            while [[ $# -gt 0 ]]; do
                TARGET_PATHS+=("$1")
                shift
            done
            break
            ;;
        -*)
            echo "error: unknown option: $1" >&2
            echo "run '$0 --help' for usage." >&2
            exit 1
            ;;
        *)
            TARGET_PATHS+=("$1")
            shift
            ;;
    esac
done

if [[ -z "$MESSAGE" && "$DRY_RUN" = false ]]; then
    echo "error: -m/--message is required unless running with --dry-run" >&2
    exit 1
fi

PARENT_HEAD="$(git rev-parse HEAD)"

# Check if anything is staged
STAGED_FILES="$(git diff --cached --name-only)"
if [[ -z "$STAGED_FILES" ]]; then
    echo "error: no staged changes in the index. Run 'git add <paths>' first." >&2
    exit 1
fi

# If target paths are specified, verify no unstaged/unexpected files are staged
if [[ ${#TARGET_PATHS[@]} -gt 0 ]]; then
    while IFS= read -r file; do
        matched=false
        for target in "${TARGET_PATHS[@]}"; do
            target_clean="${target%/}"
            if [[ "$file" == "$target_clean"* || "$file" == "$target_clean" ]]; then
                matched=true
                break
            fi
        done
        if [[ "$matched" = false ]]; then
            echo "ABORT: Staged file '$file' is outside specified paths: ${TARGET_PATHS[*]}" >&2
            echo "Unstage with: git restore --staged '$file'" >&2
            exit 1
        fi
    done <<< "$STAGED_FILES"
fi

# Inspect insertions and deletions
TOTAL_INS=0
TOTAL_DEL=0
while IFS=$'\t' read -r ins del file; do
    # Skip binary files where numstat prints '-'
    if [[ "$ins" != "-" && "$del" != "-" ]]; then
        TOTAL_INS=$((TOTAL_INS + ins))
        TOTAL_DEL=$((TOTAL_DEL + del))
    fi
done < <(git diff --cached --numstat)

echo "Preflight check for commit:"
echo "  Parent commit : $PARENT_HEAD"
echo "  Staged files  : $(echo "$STAGED_FILES" | wc -l)"
echo "  Insertions    : +$TOTAL_INS"
echo "  Deletions     : -$TOTAL_DEL"

# Safety check on deletions
if [[ "$ALLOW_DELETIONS" = false && "$TOTAL_DEL" -gt "$MAX_DELETIONS" ]]; then
    echo "ABORT: Deletion count ($TOTAL_DEL) exceeds threshold ($MAX_DELETIONS)." >&2
    echo "In shared multi-agent checkouts, high deletion counts often indicate" >&2
    echo "accidental reversal of a concurrent peer commit." >&2
    echo "If this deletion count is intentional, rerun with --max-deletions $TOTAL_DEL or --allow-deletions." >&2
    exit 1
fi

# Verify HEAD hasn't moved between preflight start and commit execution
CURRENT_HEAD="$(git rev-parse HEAD)"
if [[ "$CURRENT_HEAD" != "$PARENT_HEAD" ]]; then
    echo "ABORT: HEAD moved during preflight ($PARENT_HEAD -> $CURRENT_HEAD)." >&2
    echo "Another agent committed in the background. Re-verify your staged diff against current HEAD." >&2
    exit 1
fi

if [[ "$DRY_RUN" = true ]]; then
    echo "Dry-run check PASSED. No commit created."
    exit 0
fi

# Execute commit
git commit -m "$MESSAGE"
echo "Commit created successfully: $(git rev-parse --short HEAD)"
