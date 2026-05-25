#!/usr/bin/env bash
# gauntlet_regression_e2e.sh - Full regression harness for FrankenPandas
# Runs fmt, clippy, tests, conformance delta, and perf baselines
# JSON-line logging for each step

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
TIMESTAMP=$(date -u +%Y%m%dT%H%M%SZ)
ARTIFACTS_DIR="$PROJECT_ROOT/tests/artifacts/regression/$TIMESTAMP"

# Crates to test (in dependency order)
CRATES=(
    fp-types
    fp-runtime
    fp-columnar
    fp-index
    fp-expr
    fp-groupby
    fp-join
    fp-frame
    fp-io
    fp-conformance
    fp-frankentui
    frankenpandas
)

# Parse arguments
SCOPE_CRATE=""
SKIP_PERF=false
while [[ $# -gt 0 ]]; do
    case $1 in
        --crate)
            SCOPE_CRATE="$2"
            shift 2
            ;;
        --skip-perf)
            SKIP_PERF=true
            shift
            ;;
        *)
            echo "Usage: $0 [--crate <name>] [--skip-perf]"
            exit 1
            ;;
    esac
done

mkdir -p "$ARTIFACTS_DIR"
LOG_FILE="$ARTIFACTS_DIR/regression.jsonl"

# JSON logging helper
log_json() {
    local step="$1"
    local status="$2"
    local duration="$3"
    local message="${4:-}"
    echo "{\"timestamp\":\"$(date -u +%Y-%m-%dT%H:%M:%SZ)\",\"step\":\"$step\",\"status\":\"$status\",\"duration_ms\":$duration,\"message\":\"$message\"}" >> "$LOG_FILE"
}

log_info() { echo "[$(date -u +%H:%M:%S)] INFO: $*"; }
log_fail() { echo "[$(date -u +%H:%M:%S)] FAIL: $*" >&2; }
log_pass() { echo "[$(date -u +%H:%M:%S)] PASS: $*"; }

TOTAL_STEPS=0
PASSED_STEPS=0
FAILED_STEPS=0

run_step() {
    local name="$1"
    shift
    local cmd="$*"

    TOTAL_STEPS=$((TOTAL_STEPS + 1))
    log_info "Starting: $name"
    local start_ms=$(date +%s%3N)

    if eval "$cmd" > "$ARTIFACTS_DIR/${name//[ \/]/_}.log" 2>&1; then
        local end_ms=$(date +%s%3N)
        local duration=$((end_ms - start_ms))
        log_json "$name" "pass" "$duration"
        log_pass "$name (${duration}ms)"
        PASSED_STEPS=$((PASSED_STEPS + 1))
        return 0
    else
        local end_ms=$(date +%s%3N)
        local duration=$((end_ms - start_ms))
        log_json "$name" "fail" "$duration" "See ${name//[ \/]/_}.log"
        log_fail "$name (${duration}ms) - see log"
        FAILED_STEPS=$((FAILED_STEPS + 1))
        return 1
    fi
}

log_info "========================================="
log_info "GAUNTLET REGRESSION E2E"
log_info "========================================="
log_info "Timestamp: $TIMESTAMP"
log_info "Artifacts: $ARTIFACTS_DIR"
if [[ -n "$SCOPE_CRATE" ]]; then
    log_info "Scope: $SCOPE_CRATE only"
fi
log_info "========================================="

# Step 1: Format check
run_step "fmt-check" "cargo fmt --check" || true

# Step 2-3: Clippy and Test per crate
if [[ -n "$SCOPE_CRATE" ]]; then
    CRATES=("$SCOPE_CRATE")
fi

for crate in "${CRATES[@]}"; do
    run_step "clippy-$crate" "cargo clippy -p $crate -- -D warnings" || true
    run_step "test-$crate" "cargo test -p $crate" || true
done

# Step 4: Conformance delta
run_step "conformance-delta" "$SCRIPT_DIR/gauntlet_conformance_delta.sh" || true

# Step 5: Perf baselines (optional)
if [[ "$SKIP_PERF" == false ]]; then
    run_step "perf-baselines" "cargo test -p fp-conformance --test perf_baselines -- --ignored" || true
fi

# Summary
log_info "========================================="
log_info "REGRESSION SUMMARY"
log_info "========================================="
log_info "Total steps: $TOTAL_STEPS"
log_info "Passed: $PASSED_STEPS"
log_info "Failed: $FAILED_STEPS"
log_info "Log: $LOG_FILE"
log_info "========================================="

if [[ $FAILED_STEPS -gt 0 ]]; then
    log_fail "REGRESSION: $FAILED_STEPS step(s) failed"
    exit 1
else
    log_pass "All steps passed"
    exit 0
fi
