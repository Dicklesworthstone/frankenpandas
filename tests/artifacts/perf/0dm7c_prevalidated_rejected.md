# br-frankenpandas-0dm7c prevalidated-ranks candidate

## Verdict

Rejected. The candidate made the complete Kendall matrix prevalidate order/rank
bounds once, then call unchecked/internal ordered-rank helpers per pair. It kept
the output byte-identical but did not improve the measured wall time.

No production source from this candidate is staged for the closeout commit.

## Proof

Golden hashes stayed byte-identical:

```text
df_kendall 2000:  acf366c266b66f8497fb55734ed1b4ec40952a7c014f43db262c7c1e625e15e1
df_kendall 5000:  031978ba431260b942dd36d9be055064a7453118a7c00888c35747644d33d99e
df_kendall 20000: f164fa86300fbea93e46aa99a5e0f7413fa8c0baf2ee49c6a689ed92652a4c3b
```

Paired timing:

```text
df_kendall 50000 1:
  baseline:  167.6 ms +/- 3.4 ms
  candidate: 168.7 ms +/- 1.2 ms
  result: baseline 1.01x faster

df_kendall 200000 1:
  baseline:  695.8 ms +/- 21.5 ms
  candidate: 716.3 ms +/- 5.9 ms
  result: baseline 1.03x faster
```

Score: Impact 0 x Confidence 4 / Effort 1 = 0.0.

## Interpretation

The hot wall is not per-pair validation overhead. Removing checked option and
bounds plumbing adds a one-time scan and does not reduce the dominant Fenwick
inversion work. The next useful Kendall bead remains
`br-frankenpandas-uza04.87`: exact cross-pair work sharing or all-pairs
rank-signature/offline dominance counting.
