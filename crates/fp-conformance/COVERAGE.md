# FrankenPandas Conformance Coverage

> Coverage accounting for pandas API parity testing.
> Generated: 2026-04-15

## Coverage Summary

| Category | Operations | Fixtures | Coverage |
|----------|:----------:|:--------:|:--------:|
| Series Arithmetic | 5 | 13 | ✅ Core |
| Series Aggregation | 6 | 24 | ✅ Core |
| Series Selection | 6 | 26 | ✅ Core |
| Series Transform | 4 | 12 | ✅ Core |
| DataFrame Constructor | 6 | 84 | ✅ Core |
| DataFrame Selection | 4 | 35 | ✅ Core |
| DataFrame Merge/Join | 3 | 48 | ✅ Core |
| DataFrame Transform | 8 | 32 | ✅ Core |
| DataFrame Reshape | 1 | 4 | ⚠️ Started |
| DataFrame Aggregation | 4 | 10 | ⚠️ Light |
| GroupBy | 10 | 24 | ⚠️ Light |
| Index Operations | 5 | 14 | ✅ Core |
| NaN Aggregations | 7 | 17 | ✅ Core |
| IO (CSV) | 1 | 16 | ✅ Core |
| **Total** | **70** | **467** | **Partial** |

## Operation Coverage Detail

### Series Operations (Fixtures: 75)

| Operation | Fixtures | Status | Notes |
|-----------|:--------:|:------:|-------|
| series_add | 5 | ✅ | Alignment, dtype promotion |
| series_sub | 2 | ⚠️ | Needs more edge cases |
| series_mul | 2 | ⚠️ | Needs more edge cases |
| series_div | 2 | ⚠️ | Needs division by zero |
| series_constructor | 5 | ✅ | Various dtypes |
| series_all | 4 | ✅ | Bool aggregation |
| series_any | 4 | ✅ | Bool aggregation |
| series_count | 4 | ✅ | Null handling |
| series_dropna | 4 | ✅ | NA filtering |
| series_fillna | 4 | ✅ | Fill methods |
| series_filter | 4 | ✅ | Boolean masking |
| series_head | 1 | ⚠️ | Needs negative n |
| series_tail | 6 | ✅ | Negative n covered |
| series_iloc | 6 | ✅ | Integer indexing |
| series_loc | 4 | ✅ | Label indexing |
| series_isna/isnull | 6 | ✅ | Null detection |
| series_notna/notnull | 6 | ✅ | Null detection |
| series_sort_values | 4 | ✅ | Ascending/descending |
| series_sort_index | 2 | ⚠️ | Needs more cases |
| series_value_counts | 4 | ✅ | Frequency counts |
| series_join | 4 | ✅ | Join types |
| series_concat | 2 | ⚠️ | Needs more cases |

### DataFrame Operations (Fixtures: 274)

| Operation | Fixtures | Status | Notes |
|-----------|:--------:|:------:|-------|
| dataframe_constructor_list_like | 41 | ✅ | Comprehensive |
| dataframe_constructor_kwargs | 15 | ✅ | Index/columns |
| dataframe_constructor_scalar | 8 | ✅ | Broadcast |
| dataframe_constructor_dict_of_series | 8 | ✅ | Series composition |
| dataframe_from_dict | 11 | ✅ | Dict orient |
| dataframe_from_records | 13 | ✅ | Record orient |
| dataframe_from_series | 8 | ✅ | Single series |
| dataframe_concat | 53 | ✅ | Axis/join options |
| dataframe_merge | 35 | ✅ | All join types |
| dataframe_merge_asof | 9 | ✅ | Time-series join |
| dataframe_merge_index | 4 | ⚠️ | Needs more cases |
| dataframe_head | 10 | ✅ | Positive/negative |
| dataframe_tail | 10 | ✅ | Positive/negative |
| dataframe_iloc | 11 | ✅ | Integer indexing |
| dataframe_loc | 9 | ✅ | Label indexing |
| dataframe_sort_values | 5 | ✅ | Multi-column |
| dataframe_sort_index | 5 | ✅ | Index sort |
| dataframe_rank | 5 | ✅ | Ranking methods |
| dataframe_dropna | 4 | ⚠️ | Axis options |
| dataframe_dropna_columns | 4 | ✅ | Column-wise |
| dataframe_fillna | 4 | ⚠️ | Fill methods |
| dataframe_count | 4 | ⚠️ | Axis options |
| dataframe_diff | 4 | ⚠️ | Period options |
| dataframe_pct_change | 3 | ⚠️ | Period options |
| dataframe_melt | 4 | ⚠️ | Basic reshape parity started |
| dataframe_duplicated | 3 | ⚠️ | Subset options |
| dataframe_drop_duplicates | 3 | ⚠️ | Keep options |
| dataframe_set_index | 3 | ⚠️ | Column to index |
| dataframe_reset_index | 3 | ⚠️ | Index to column |
| dataframe_mode | 2 | ⚠️ | Modal values |
| dataframe_isna/isnull | 4 | ✅ | Null detection |
| dataframe_notna/notnull | 4 | ✅ | Null detection |

### GroupBy Operations (Fixtures: 24)

| Operation | Fixtures | Status | Notes |
|-----------|:--------:|:------:|-------|
| groupby_sum | 3 | ⚠️ | Needs multi-key |
| groupby_mean | 2 | ⚠️ | Needs edge cases |
| groupby_median | 3 | ⚠️ | Needs edge cases |
| groupby_min | 2 | ⚠️ | Needs edge cases |
| groupby_max | 2 | ⚠️ | Needs edge cases |
| groupby_count | 2 | ⚠️ | Needs edge cases |
| groupby_std | 3 | ⚠️ | Needs ddof |
| groupby_var | 3 | ⚠️ | Needs ddof |
| groupby_first | 2 | ⚠️ | Needs edge cases |
| groupby_last | 2 | ⚠️ | Needs edge cases |

### Index Operations (Fixtures: 14)

| Operation | Fixtures | Status | Notes |
|-----------|:--------:|:------:|-------|
| index_align_union | 4 | ✅ | Core alignment |
| index_first_positions | 2 | ⚠️ | Position lookup |
| index_has_duplicates | 2 | ⚠️ | Duplicate detection |
| index_is_monotonic_increasing | 2 | ⚠️ | Monotonicity |
| index_is_monotonic_decreasing | 2 | ⚠️ | Monotonicity |

### NaN-Aware Aggregations (Fixtures: 17)

| Operation | Fixtures | Status | Notes |
|-----------|:--------:|:------:|-------|
| nan_sum | 3 | ✅ | Skip NaN sum |
| nan_mean | 2 | ⚠️ | Skip NaN mean |
| nan_min | 2 | ⚠️ | Skip NaN min |
| nan_max | 2 | ⚠️ | Skip NaN max |
| nan_count | 2 | ⚠️ | Non-null count |
| nan_std | 2 | ⚠️ | Skip NaN std |
| nan_var | 2 | ⚠️ | Skip NaN var |
| fill_na | 1 | ⚠️ | NA fill |
| drop_na | 1 | ⚠️ | NA drop |

### IO Operations (Fixtures: 16)

| Operation | Fixtures | Status | Notes |
|-----------|:--------:|:------:|-------|
| csv_round_trip | 16 | ✅ | Comprehensive |

## NOT YET TESTED

The following pandas operations are NOT yet covered by conformance fixtures:

### Series
- `series_diff` - Difference between elements
- `series_pct_change` - Percentage change
- `series_rank` - Ranking (covered in DataFrame)
- `series_shift` - Shift index
- `series_cumsum/cummax/cummin/cumprod` - Cumulative operations
- `series_rolling` - Rolling window
- `series_expanding` - Expanding window
- `series_apply` - User-defined functions
- `series_map` - Element-wise mapping
- `series_replace` - Value replacement
- `series_clip` - Value clipping
- `series_abs` - Absolute value
- `series_round` - Rounding
- `series_astype` - Type conversion
- `series_to_frame` - Convert to DataFrame

### DataFrame
- `dataframe_apply` - Row/column apply
- `dataframe_applymap` - Element-wise apply
- `dataframe_transform` - Transform with function
- `dataframe_agg/aggregate` - Multiple aggregations
- `dataframe_rolling` - Rolling window
- `dataframe_expanding` - Expanding window
- `dataframe_shift` - Shift index
- `dataframe_cumsum/cummax/cummin/cumprod` - Cumulative
- `dataframe_clip` - Value clipping
- `dataframe_round` - Rounding
- `dataframe_abs` - Absolute value
- `dataframe_astype` - Type conversion
- `dataframe_transpose/T` - Transpose
- `dataframe_pivot/pivot_table` - Pivoting
- `dataframe_melt` - Unpivot
- `dataframe_stack/unstack` - Reshaping
- `dataframe_explode` - List expansion
- `dataframe_assign` - Add columns
- `dataframe_insert` - Insert column
- `dataframe_pop` - Remove column
- `dataframe_rename` - Rename columns/index
- `dataframe_reindex` - Reindex
- `dataframe_sample` - Random sampling
- `dataframe_nlargest/nsmallest` - Top/bottom N

### GroupBy
- `groupby_agg` - Multiple aggregations (oracle exists, no fixtures)
- `groupby_transform` - Transform
- `groupby_filter` - Filter groups
- `groupby_apply` - Apply function
- `groupby_rank` - Group-wise ranking
- `groupby_shift` - Group-wise shift
- `groupby_cumsum/cummax/cummin/cumprod` - Cumulative
- `groupby_rolling` - Group-wise rolling

### Index
- `index_union/intersection/difference` - Set operations
- `index_drop_duplicates` - Remove duplicates
- `index_rename` - Rename index
- `index_reindex` - Reindex
- `index_sort_values` - Sort
- `index_unique` - Unique values
- `index_map` - Element-wise mapping

### IO (Not Yet Implemented)
- JSON read/write
- Parquet read/write
- Excel read/write
- SQL read/write
- HTML read/write
- Pickle read/write

## Fixture Provenance

**Generator:** `crates/fp-conformance/oracle/pandas_oracle.py`
**pandas version:** System pandas (typically 2.x)
**Generation command:**
```bash
python3 crates/fp-conformance/oracle/pandas_oracle.py \
  --legacy-root legacy_pandas_code/pandas \
  < fixture_request.json > fixture_response.json
```

## Coverage Goals

| Priority | Target | Current | Gap |
|----------|:------:|:-------:|:---:|
| P0: Core arithmetic/selection | 95% | ~85% | Add edge cases |
| P1: Merge/Join/Concat | 90% | ~80% | Add validation modes |
| P2: GroupBy | 80% | ~40% | Major expansion needed |
| P3: Reshape (pivot/melt) | 70% | ~10% | Melt started; pivot/stack/unstack still open |
| P4: Window functions | 60% | 0% | Not started |
| P5: IO formats | 50% | ~15% | CSV only |

## Next Actions

1. **High priority:** Add more GroupBy edge case fixtures
2. **High priority:** Add Series diff/pct_change fixtures
3. **Medium:** Expand reshape coverage beyond melt into pivot/pivot_table/stack/unstack
4. **Medium:** Add rolling/expanding window fixtures
5. **Low:** Add additional IO format support (JSON, Parquet)
