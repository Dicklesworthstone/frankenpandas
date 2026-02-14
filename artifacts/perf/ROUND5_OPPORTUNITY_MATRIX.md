# ROUND5 Opportunity Matrix

| Hotspot | Impact | Confidence | Effort | Score |
|---|---:|---:|---:|---:|
| Repeated duplicate detection hashing in `Index::has_duplicates` | 5 | 5 | 1 | 25.00 |
| Repeated index equality checks in fast-path guard | 4 | 4 | 3 | 5.33 |
| Remaining scalar conversion overhead (`Scalar::to_f64`) | 3 | 4 | 3 | 4.00 |

Round-5 selected lever:
- Lazy memoization of `Index::has_duplicates` using `OnceCell<bool>`.

Reason:
- Highest EV, minimal compatibility risk, directly targets measured hotspot.
