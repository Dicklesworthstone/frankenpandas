# Affine Int64 inner merge: 10M rows on trj

Decision: **KEEP**

Classification: directional same-driver measurement. The host-wide admission
window was unavailable, so these numbers are not a validated benchmark claim.

Workload: `joins/join_inner/10M/float64`, with candidate, pre-change binary,
and live pandas executed by one driver using the harness's paired timing
implementation.

| Arm | ELF / version | Median | Threads observed |
|---|---|---:|---:|
| FrankenPandas candidate | `9e7676b94d40adab69597b3ca958d90472eea621ebe9810f6e528f60b577b8d9` | 3.523 ms | 11 |
| FrankenPandas pre-change | `727ff81d81aa5e396101de1479669bbe0d94966a80e0019a37b9ca0a4125026a` | 59.822 ms | 3 |
| pandas 2.2.3 | Python `efb29ce53d36ebaeee80e3aa44fd6c7f9d71bbded5fe1665240b2ed8ecaeee0e` | 138.081 ms | 1 |

- Candidate / pre-change speedup: **16.98x**
- Candidate / live pandas speedup: **39.19x**
- Candidate and pre-change checksums: `4957dea0fe3e2ed1`
- Harness SHA-256: `b9affde33c6e14a19ad118d6d3f0252c72fd38ee4cc9983517f5d244ebf859d4`
- Host: `threadripperje`, AMD Threadripper PRO 5995WX, 64 physical cores,
  128 logical threads, AVX2/FMA, performance governor

The candidate replaces serial ordered-key certification and two-pointer
intersection with up to 64 independent proof morsels plus an analytic CRT
intersection. Float64 payloads remain strided views, and a unit-stride shared
key is now an immutable zero-copy chunk view.
