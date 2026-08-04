#!/usr/bin/env python3
"""Compare FP Series.quantile (from quantile_parity_dump stdout) vs pandas 2.2.3
across 5 interpolation modes. Prints any cell diverging > 1e-12. Usage:
  quantile_parity_check.py <fp_dump_file>"""
import sys
import pandas as pd

datasets = [
    [1.0, 2.0, 3.0, 4.0],
    [1.0, 2.0, 3.0, 4.0, 5.0],
    [10.0, 7.0, 3.0, 1.0, 9.0, 2.0],
    [-3.0, -1.0, 0.0, 2.5, 4.0],
    [5.0, 5.0, 5.0],
    [1.0, 1.0, 2.0, 8.0],
    [1.0, 2.0],
    [42.0],
]
# Parse FP dump
fp = {}
for line in open(sys.argv[1]):
    if not line.startswith("QP "):
        continue
    _, di, qi, mode, val = line.split()
    fp[(int(di[1:]), float(qi[1:]), mode)] = float(val)

mism = 0
checked = 0
for (i, q, m), fv in sorted(fp.items()):
    s = pd.Series(datasets[i])
    pv = float(s.quantile(q, interpolation=m))
    checked += 1
    if abs(fv - pv) > 1e-12 and not (fv != fv and pv != pv):
        print(f"DIVERGE d{i}={datasets[i]} q={q} {m}: fp={fv!r} pandas={pv!r}")
        mism += 1
print(f"pandas {pd.__version__}  checked={checked} divergences={mism}")
