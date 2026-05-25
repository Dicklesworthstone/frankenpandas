#!/usr/bin/env bash
# gauntlet_conformance_delta.sh - Conformance gate delta checker
# Runs conformance tests, aggregates results, diffs against baseline
# Exit non-zero if any packet flips red

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
ARTIFACTS_DIR="$PROJECT_ROOT/artifacts/phase2c"
REPORTS_DIR="$PROJECT_ROOT/reports"
BASELINE_FILE="$REPORTS_DIR/conformance_baseline_set.json"
TIMESTAMP=$(date -u +%Y%m%dT%H%M%SZ)
DELTA_DIR="$REPORTS_DIR/delta"
DELTA_FILE="$DELTA_DIR/$TIMESTAMP.json"

log_info() { echo "[$(date -u +%H:%M:%S)] INFO: $*"; }
log_warn() { echo "[$(date -u +%H:%M:%S)] WARN: $*" >&2; }
log_fail() { echo "[$(date -u +%H:%M:%S)] FAIL: $*" >&2; }

mkdir -p "$DELTA_DIR"

# Step 1: Run conformance tests
log_info "Running conformance gate tests..."
if ! cargo test -p fp-conformance differential_all_packets_green --release 2>&1 | tee /tmp/conformance_run.log; then
    log_fail "Conformance tests failed to complete"
    exit 1
fi

# Step 2: Aggregate all parity_gate_result.json files
log_info "Aggregating parity gate results..."

CURRENT_RESULTS=$(mktemp)
find "$ARTIFACTS_DIR" -name "parity_gate_result.json" -exec cat {} \; 2>/dev/null | \
    jq -s '
        {
            timestamp: "'"$TIMESTAMP"'",
            total_packets: length,
            passed: [.[] | select(.pass == true)] | length,
            failed: [.[] | select(.pass == false)] | length,
            red_packets: [.[] | select(.pass == false) | .packet_id] | sort,
            green_packets: [.[] | select(.pass == true) | .packet_id] | sort,
            live_validated: [.[] | select(.validation_source == "live")] | length,
            replay_only: [.[] | select(.validation_source == "replay" or .validation_source == null)] | length,
            live_validated_pct: (([.[] | select(.validation_source == "live")] | length) / (length | if . == 0 then 1 else . end) * 100 | floor),
            details: (. | sort_by(.packet_id) | map({(.packet_id): {pass, reasons, validation_source}})) | add
        }
    ' > "$CURRENT_RESULTS"

TOTAL=$(jq -r '.total_packets' "$CURRENT_RESULTS")
PASSED=$(jq -r '.passed' "$CURRENT_RESULTS")
FAILED=$(jq -r '.failed' "$CURRENT_RESULTS")

log_info "Results: $PASSED/$TOTAL passed, $FAILED failed"

# Step 3: Compare against baseline
if [[ -f "$BASELINE_FILE" ]]; then
    log_info "Comparing against baseline..."

    BASELINE_RED=$(jq -r '.red_packets | sort | .[]' "$BASELINE_FILE" 2>/dev/null | sort)
    CURRENT_RED=$(jq -r '.red_packets | sort | .[]' "$CURRENT_RESULTS" | sort)

    BASELINE_GREEN=$(jq -r '.green_packets | sort | .[]' "$BASELINE_FILE" 2>/dev/null | sort)
    CURRENT_GREEN=$(jq -r '.green_packets | sort | .[]' "$CURRENT_RESULTS" | sort)

    # Find packets that flipped
    FLIPPED_GREEN=$(comm -23 <(echo "$BASELINE_RED") <(echo "$CURRENT_RED") | tr '\n' ' ')
    FLIPPED_RED=$(comm -13 <(echo "$BASELINE_RED") <(echo "$CURRENT_RED") | tr '\n' ' ')

    # Build delta JSON
    jq -n \
        --arg timestamp "$TIMESTAMP" \
        --argjson current "$(cat "$CURRENT_RESULTS")" \
        --arg flipped_green "$FLIPPED_GREEN" \
        --arg flipped_red "$FLIPPED_RED" \
        '{
            timestamp: $timestamp,
            packets_flipped_green: ($flipped_green | split(" ") | map(select(. != ""))),
            packets_flipped_red: ($flipped_red | split(" ") | map(select(. != ""))),
            still_red: $current.red_packets,
            counts: {
                total: $current.total_packets,
                passed: $current.passed,
                failed: $current.failed,
                pass_rate: (($current.passed / $current.total_packets) * 100 | floor / 100)
            }
        }' > "$DELTA_FILE"

    FLIPPED_GREEN_COUNT=$(echo "$FLIPPED_GREEN" | wc -w | tr -d ' ')
    FLIPPED_RED_COUNT=$(echo "$FLIPPED_RED" | wc -w | tr -d ' ')

    log_info "Flipped green: $FLIPPED_GREEN_COUNT packets"
    log_info "Flipped red: $FLIPPED_RED_COUNT packets"

    if [[ -n "$FLIPPED_GREEN" && "$FLIPPED_GREEN" != " " ]]; then
        log_info "Packets now passing: $FLIPPED_GREEN"
    fi

    if [[ -n "$FLIPPED_RED" && "$FLIPPED_RED" != " " ]]; then
        log_fail "REGRESSION: Packets flipped RED: $FLIPPED_RED"
        log_fail "Delta report: $DELTA_FILE"
        rm -f "$CURRENT_RESULTS"
        exit 1
    fi
else
    log_warn "No baseline file found at $BASELINE_FILE"
    log_info "Creating initial baseline..."
    mkdir -p "$REPORTS_DIR"
    cp "$CURRENT_RESULTS" "$BASELINE_FILE"

    jq -n \
        --arg timestamp "$TIMESTAMP" \
        --argjson current "$(cat "$CURRENT_RESULTS")" \
        '{
            timestamp: $timestamp,
            packets_flipped_green: [],
            packets_flipped_red: [],
            still_red: $current.red_packets,
            counts: {
                total: $current.total_packets,
                passed: $current.passed,
                failed: $current.failed,
                pass_rate: (($current.passed / $current.total_packets) * 100 | floor / 100)
            },
            note: "Initial baseline created"
        }' > "$DELTA_FILE"
fi

rm -f "$CURRENT_RESULTS"

# Summary
log_info "========================================="
log_info "CONFORMANCE DELTA SUMMARY"
log_info "========================================="
log_info "Timestamp: $TIMESTAMP"
log_info "Total packets: $TOTAL"
log_info "Passed: $PASSED ($((PASSED * 100 / TOTAL))%)"
log_info "Failed: $FAILED"
log_info "Delta report: $DELTA_FILE"
log_info "========================================="

if [[ "$FAILED" -eq 0 ]]; then
    log_info "All conformance packets PASSED"
else
    log_warn "$FAILED packets still failing (see delta report)"
fi

exit 0
