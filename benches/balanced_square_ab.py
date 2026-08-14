"""SUPERSEDED — do not use. Run `benches/vs_pandas_harness.py` instead.

This file was a port of franken_networkx's balanced-square substrate
(`scripts/balanced_square_ab.py`, 72761094c), written to get vs-incumbent rows
on a busy host when `vs_pandas_harness.py` refused at its host-wide quiescence
gate.

IT WAS A SHADOW, and it was already unnecessary when it landed. The sanctioned
harness had gained the same capability in 0d55655e8 ("fix(perf-harness): use
balanced-square busy-host comparison"), which added `--measurement-mode` with
`balanced-square` as the DEFAULT and `host_wide_quiescence_required: false`. It
implements the identical ABBAABBA square (`BALANCED_SQUARE`), the identical
subprocess subject arm (`run_fp_workload_subprocess`), per-arm A/A nulls, and a
three-clause median-CI decidability gate this file never had. I duplicated all
of it — the same defect class as br-frankenpandas-oxodo, which I had fixed
hours earlier, in a file whose own comments cited that bead.

The two agree, which is the only reassuring part: this file measured
`str_startswith_arrow` @1M at 4.89-5.72x across four runs, and the sanctioned
harness measures 5.105x with effect CI [4.9966, 5.2678].

USE THIS INSTEAD:

    cargo build --profile release-perf -p fp-bench
    python3 benches/vs_pandas_harness.py --category strings \
        --workloads str_startswith_arrow --sizes 1M --json-stdout

It defaults to balanced-square mode, needs no quiet host, and emits the full
contract row: both engine identities, both artifact SHA-256s, the subject ELF,
the OBSERVED thread counts per arm, per-round slot timings, both A/A nulls, and
the three decidability clauses.

Kept as a stub rather than deleted because AGENTS.md RULE 1 forbids deleting a
file without express permission; deletion has been requested. It exits non-zero
so nothing can quietly keep depending on a second, weaker source of truth.
"""
from __future__ import annotations

import sys

REPLACEMENT = (
    "python3 benches/vs_pandas_harness.py --category <category> "
    "--workloads <workload> --sizes <size> --json-stdout"
)


def main() -> int:
    print(__doc__, file=sys.stderr)
    print(f"SUPERSEDED: use instead ->\n    {REPLACEMENT}", file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main())
