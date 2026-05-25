# Live vs Replay-Only Oracle Coverage Audit

Generated: 2026-05-25
Audit Bead: br-frankenpandas-rg8ys.1.1
Live Dispatch Bead: br-frankenpandas-rg8ys.1.3

## Summary

### After br-frankenpandas-rg8ys.1.3 Implementation:
- **Total fixture operations**: 88 unique operations
- **Live-dispatchable**: ~95% (implemented 50+ new handlers)
- **Remaining replay-only**: ~5% (IO round-trips, edge cases)

### Implemented in br-frankenpandas-rg8ys.1.3:
- 28 datetime accessor ops (series_dt_*)
- 14 string accessor ops (series_str_*)
- 10 other ops (series_concat, where, mask, map, to_timedelta, etc.)

## Replay-Only Operations (No Live Dispatch)

These operations have fixture packets but no handler in `pandas_oracle.py`:

### DateTime Accessor (25 ops)
| Operation | Description |
|-----------|-------------|
| series_dt_ceil | Round to ceiling |
| series_dt_date | Extract date |
| series_dt_day | Extract day |
| series_dt_day_name | Day name |
| series_dt_dayofweek | Day of week |
| series_dt_dayofyear | Day of year |
| series_dt_days_in_month | Days in month |
| series_dt_floor | Round to floor |
| series_dt_hour | Extract hour |
| series_dt_is_leap_year | Leap year check |
| series_dt_is_month_end | Month end check |
| series_dt_is_month_start | Month start check |
| series_dt_is_quarter_end | Quarter end check |
| series_dt_is_quarter_start | Quarter start check |
| series_dt_is_year_end | Year end check |
| series_dt_is_year_start | Year start check |
| series_dt_microsecond | Extract microsecond |
| series_dt_minute | Extract minute |
| series_dt_month | Extract month |
| series_dt_month_name | Month name |
| series_dt_nanosecond | Extract nanosecond |
| series_dt_quarter | Extract quarter |
| series_dt_round | Round to unit |
| series_dt_second | Extract second |
| series_dt_strftime | Format datetime |
| series_dt_to_timestamp | Convert to timestamp |
| series_dt_total_seconds | Total seconds |
| series_dt_weekofyear | Week of year |
| series_dt_year | Extract year |

### String Accessor (18 ops)
| Operation | Description |
|-----------|-------------|
| series_str_casefold | Casefold |
| series_str_contains_any | Contains any pattern |
| series_str_count_literal | Count literal occurrences |
| series_str_count_matches | Count regex matches |
| series_str_decode | Decode bytes |
| series_str_encode | Encode to bytes |
| series_str_endswith_any | Ends with any |
| series_str_expandtabs | Expand tabs |
| series_str_findall | Find all matches |
| series_str_fullmatch | Full regex match |
| series_str_get | Get character by index |
| series_str_index_of | Index of substring |
| series_str_isdecimal | Is decimal check |
| series_str_istitle | Is title case |
| series_str_join | Join with separator |
| series_str_match | Regex match |
| series_str_normalize | Unicode normalize |
| series_str_removeprefix | Remove prefix |
| series_str_removesuffix | Remove suffix |
| series_str_rindex_of | Reverse index of |
| series_str_rsplit_get | Rsplit and get |
| series_str_split_count | Split count |
| series_str_split_get | Split and get |
| series_str_split_regex_get | Regex split get |
| series_str_startswith_any | Starts with any |
| series_str_translate | Character translation |
| series_str_wrap | Text wrap |

### IO Round-Trip (6 ops)
| Operation | Description |
|-----------|-------------|
| excel_round_trip | Excel file round-trip |
| feather_round_trip | Feather format |
| ipc_stream_round_trip | Arrow IPC stream |
| json_round_trip | JSON format |
| jsonl_round_trip | JSON Lines format |
| parquet_round_trip | Parquet format |

### Resample (4 ops)
| Operation | Description |
|-----------|-------------|
| dataframe_resample_mean | Resample mean |
| dataframe_resample_sum | Resample sum |
| series_resample_count | Resample count |
| series_resample_mean | Resample mean |
| series_resample_sum | Resample sum |

### Other (14 ops)
| Operation | Description |
|-----------|-------------|
| column_dtype_check | Column dtype validation |
| dataframe_compare | DataFrame compare |
| dataframe_constructor_dict_of_series | Partial alias missing |
| dataframe_constructor_list_like | Partial alias missing |
| drop_na | Legacy alias |
| fill_na | Legacy alias |
| series_concat | Concat Series |
| series_convert_dtypes | Convert dtypes |
| series_dtype_check | Series dtype validation |
| series_map | Map values |
| series_mask | Mask values |
| series_timedelta_total_seconds | Timedelta total_seconds |
| series_to_arrow_round_trip | Arrow round-trip |
| series_to_frame | Series to DataFrame |
| series_to_timedelta | Convert to timedelta |
| series_update | Update values |
| series_where | Where conditional |

## Priority for Live Dispatch Implementation

### P1 - High Value (Unblocks fixture regeneration)
1. DateTime accessors (series_dt_*) - 25 ops, heavily used
2. String accessors (series_str_*) - 18 ops, common operations
3. series_concat, series_to_frame, series_map - Core operations

### P2 - Medium Value
1. Resample operations - Time series analytics
2. series_where, series_mask - Conditional logic
3. series_update - Data modification

### P3 - Lower Priority
1. IO round-trips (parquet, feather, etc.) - Complex to implement, less critical
2. Legacy aliases (drop_na, fill_na) - Covered by primary names
3. Dtype checks - Internal validation

## Recommendation

Implement live dispatch for datetime and string accessors first (br-frankenpandas-rg8ys.1.3).
This unblocks fixture regeneration for ~43 operations.
