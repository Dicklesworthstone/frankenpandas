#!/usr/bin/env python3
"""Differential: FP DataFrame.corr() vs pandas (br-frankenpandas-fgy9g).
Regenerates the identical splitmix input in numpy, computes pandas corr, and
compares against FP's corr matrix read from a file holding the example's
"FPCORR v0 v1 ..." stdout line. Prints max abs/rel divergence vs the 1e-10
conformance tolerance.

Usage: corr_parity_check.py <n> <m> <fp_stdout_file>"""
import sys
import numpy as np
import pandas as pd

n, m, fpfile = int(sys.argv[1]), int(sys.argv[2]), sys.argv[3]

# Replicate perf_profile::build_corr_frame splitmix exactly (u64 wrapping).
U64 = np.uint64
def col(c):
    i = np.arange(n, dtype=U64)
    z = i * U64(0x9e3779b97f4a7c15) + U64(c) * U64(0xbf58476d1ce4e5b9)
    z = (z ^ (z >> U64(30))) * U64(0xbf58476d1ce4e5b9)
    z = (z ^ (z >> U64(27))) * U64(0x94d049bb133111eb)
    z = z ^ (z >> U64(31))
    unit = (z >> U64(11)).astype(np.float64) / float(1 << 53)
    return unit * 2.0 - 1.0

with np.errstate(over="ignore"):
    df = pd.DataFrame({f"c{c}": col(c) for c in range(m)})
pd_corr = df.corr().to_numpy()

fp_vals = None
for line in open(fpfile):
    if line.startswith("FPCORR"):
        fp_vals = np.array([float(x) for x in line.split()[1:]], dtype=np.float64)
        break
assert fp_vals is not None and fp_vals.size == m * m, (None if fp_vals is None else fp_vals.size, m * m)
fp = fp_vals.reshape(m, m)

diff = np.abs(fp - pd_corr)
rel = diff / np.maximum(np.abs(pd_corr), 1e-300)
max_abs = float(np.nanmax(diff))
max_rel = float(np.nanmax(rel))
print(f"pandas {pd.__version__}  n={n} m={m}")
print(f"max_abs_diff = {max_abs:.3e}")
print(f"max_rel_diff = {max_rel:.3e}")
print(f"exceeds_1e-10 = {max_abs > 1e-10}")
print(f"exceeds_1e-12 = {max_abs > 1e-12}")
fi, fj = np.unravel_index(np.nanargmax(diff), diff.shape)
print(f"worst [{fi},{fj}]: fp={fp[fi,fj]:.17e} pandas={pd_corr[fi,fj]:.17e}")
