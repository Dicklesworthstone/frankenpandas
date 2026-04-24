# Conformance Fixtures

This folder stores normalized oracle-vs-target fixtures for fp-conformance.

## Structure

- `smoke_case.json`: minimal bootstrap fixture ensuring harness wiring works.
- `packets/*.json`: Phase-2C packet fixtures consumed by `run_packet_suite`.

## Provenance

**Generator:** `../oracle/pandas_oracle.py`
**Reference implementation:** pandas `2.2.3` (pinned in `../oracle/requirements.txt`)
**Generation workflow:**
```bash
# Generate fixture from oracle
python3 ../oracle/pandas_oracle.py \
  --legacy-root ../../legacy_pandas_code/pandas \
  < request.json > response.json
```

Every packet fixture now carries a top-level `fixture_provenance` object with:
- `pandas_version`
- `oracle_script_sha256`
- `generated_at`

Oracle error responses also carry the same provenance whenever pandas has been
loaded successfully, so failed regeneration attempts can still be tied to the
exact pandas version and oracle script revision that produced them.

## Fixture Format

Each packet fixture is a JSON object with:
- `packet_id`: Unique identifier (e.g., "FP-P2C-001")
- `case_id`: Human-readable test case name
- `mode`: "strict" or "hardened"
- `operation`: The pandas operation being tested
- `fixture_provenance`: Oracle/version/timestamp metadata for stale-fixture detection
- `left`, `right`, `frame`: Input data
- `expected_*`: Expected output from pandas oracle

## Regenerating Fixtures

When pandas behavior changes or new operations are added:

1. Update the oracle script to handle new operations
2. If pandas behavior changed, bump `../oracle/requirements.txt` first
3. Generate new fixtures via the oracle
4. Run `../../scripts/check_fixture_freshness.sh`
5. Review diff: `git diff fixtures/`
6. Document any divergences in `../DISCREPANCIES.md`
7. Commit with message referencing the change

## Current Coverage

See `../COVERAGE.md` for detailed coverage matrix.
