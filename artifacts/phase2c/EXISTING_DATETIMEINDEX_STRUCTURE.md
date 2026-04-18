# EXISTING_DATETIMEINDEX_STRUCTURE.md

## 1. Overview

`pd.DatetimeIndex` is an Index of datetime64[ns] values. Used for time-series indexing and date-based operations.

Key pandas objects:
- `pd.DatetimeIndex` - index of datetime values
- `pd.date_range()` - range constructor for DatetimeIndex
- `pd.Timestamp` - scalar datetime type

## 2. DatetimeIndex Constructor

### 2.1 Signature

```python
pd.DatetimeIndex(
    data=None,           # array-like of datetime-like
    freq=None,           # offset alias or DateOffset
    tz=None,             # timezone string or tzinfo
    normalize=False,     # normalize to midnight
    closed=None,         # 'left', 'right', or None
    ambiguous='raise',   # how to handle DST ambiguity
    dayfirst=False,      # parse day-first format
    yearfirst=False,     # parse year-first format
    dtype=None,          # always datetime64[ns] or datetime64[ns, tz]
    copy=False,
    name=None,
)
```

### 2.2 Construction Patterns

```python
# From strings
pd.DatetimeIndex(['2024-01-01', '2024-01-02', '2024-01-03'])

# From Timestamp objects
pd.DatetimeIndex([pd.Timestamp('2024-01-01'), pd.Timestamp('2024-01-02')])

# From numpy datetime64
pd.DatetimeIndex(np.array(['2024-01-01', '2024-01-02'], dtype='datetime64[D]'))

# With name
pd.DatetimeIndex(['2024-01-01', '2024-01-02'], name='date')

# With timezone
pd.DatetimeIndex(['2024-01-01', '2024-01-02'], tz='UTC')
```

## 3. date_range() Function

### 3.1 Signature

```python
pd.date_range(
    start=None,       # str or datetime-like
    end=None,         # str or datetime-like
    periods=None,     # int, number of periods
    freq=None,        # str or DateOffset, default 'D'
    tz=None,          # timezone
    normalize=False,  # normalize to midnight
    name=None,        # name for resulting index
    closed=None,      # 'left', 'right', or None (deprecated)
    inclusive='both', # 'both', 'neither', 'left', 'right'
)
```

### 3.2 Examples

```python
# By start and periods
pd.date_range(start='2024-01-01', periods=5)
# DatetimeIndex(['2024-01-01', '2024-01-02', '2024-01-03', '2024-01-04', '2024-01-05'])

# By start and end
pd.date_range(start='2024-01-01', end='2024-01-05')
# DatetimeIndex(['2024-01-01', '2024-01-02', '2024-01-03', '2024-01-04', '2024-01-05'])

# With frequency
pd.date_range(start='2024-01-01', periods=5, freq='h')
# DatetimeIndex(['2024-01-01 00:00', '2024-01-01 01:00', ...])

# Business days
pd.date_range(start='2024-01-01', periods=5, freq='B')

# With timezone
pd.date_range(start='2024-01-01', periods=3, tz='US/Eastern')
```

### 3.3 Frequency Aliases

| Alias | Meaning |
|-------|---------|
| D | calendar day |
| B | business day |
| W | weekly |
| M | month end |
| MS | month start |
| Q | quarter end |
| QS | quarter start |
| Y, A | year end |
| YS, AS | year start |
| h, H | hour |
| min, T | minute |
| s, S | second |
| ms, L | millisecond |
| us, U | microsecond |
| ns, N | nanosecond |

Numeric prefix: '2D', '6h', '30min', etc.

## 4. Properties

### 4.1 Component Properties

```python
dti = pd.date_range('2024-01-15 10:30:45', periods=3)

dti.year           # Int64Index([2024, 2024, 2024])
dti.month          # Int64Index([1, 1, 1])
dti.day            # Int64Index([15, 16, 17])
dti.hour           # Int64Index([10, 10, 10])
dti.minute         # Int64Index([30, 30, 30])
dti.second         # Int64Index([45, 45, 45])
dti.microsecond    # Int64Index([0, 0, 0])
dti.nanosecond     # Int64Index([0, 0, 0])
dti.dayofweek      # Int64Index([0, 1, 2]) (Monday=0)
dti.dayofyear      # Int64Index([15, 16, 17])
dti.quarter        # Int64Index([1, 1, 1])
dti.is_month_start # array([False, False, False])
dti.is_month_end   # array([False, False, False])
dti.is_quarter_start
dti.is_quarter_end
dti.is_year_start
dti.is_year_end
dti.is_leap_year
dti.date           # array of datetime.date
dti.time           # array of datetime.time
```

### 4.2 Index Properties

```python
dti.dtype        # dtype('datetime64[ns]') or datetime64[ns, tz]
dti.freq         # frequency if regular, else None
dti.inferred_freq # inferred frequency
dti.name         # index name
dti.tz           # timezone or None
```

## 5. Arithmetic Operations

### 5.1 DatetimeIndex + Timedelta

```python
dti = pd.date_range('2024-01-01', periods=3)
dti + pd.Timedelta('1 day')   # Shift forward 1 day
dti - pd.Timedelta('6 hours') # Shift back 6 hours
```

### 5.2 DatetimeIndex - DatetimeIndex

```python
dti1 = pd.date_range('2024-01-01', periods=3)
dti2 = pd.date_range('2024-01-05', periods=3)
dti2 - dti1  # TimedeltaIndex(['4 days', '4 days', '4 days'])
```

### 5.3 DatetimeIndex + DateOffset

```python
dti = pd.date_range('2024-01-31', periods=3, freq='M')
dti + pd.DateOffset(months=1)  # Add 1 month
```

## 6. Rounding Methods

```python
dti = pd.date_range('2024-01-15 10:30:45', periods=3, freq='h')

dti.round('h')    # Round to nearest hour
dti.floor('D')    # Floor to day
dti.ceil('h')     # Ceiling to hour
```

## 7. Timezone Operations

```python
dti = pd.date_range('2024-01-01', periods=3)

dti.tz_localize('UTC')           # Localize naive to UTC
dti.tz_convert('US/Eastern')     # Convert between timezones
dti.tz_localize(None)            # Remove timezone
```

## 8. Slicing and Selection

```python
dti = pd.date_range('2024-01-01', periods=10)

dti[0]           # Timestamp('2024-01-01')
dti[1:5]         # DatetimeIndex slice
dti[dti.month == 1]  # Boolean mask
```

## 9. Set Operations

```python
dti1 = pd.date_range('2024-01-01', periods=5)
dti2 = pd.date_range('2024-01-03', periods=5)

dti1.union(dti2)        # Union
dti1.intersection(dti2) # Intersection
dti1.difference(dti2)   # Difference
```

## 10. Conversion Methods

```python
dti.to_pydatetime()    # array of datetime.datetime
dti.to_numpy()         # numpy datetime64 array
dti.to_series()        # Series with DatetimeIndex
dti.to_frame(name)     # DataFrame with single column
dti.strftime('%Y-%m-%d') # Format as strings
```

## 11. Internal Representation

- Stored as int64 nanoseconds since Unix epoch (1970-01-01 00:00:00 UTC)
- NaT represented as int64 minimum value (same as Timedelta)
- dtype is `datetime64[ns]` (naive) or `datetime64[ns, tz]` (tz-aware)
- Range: 1677-09-21 to 2262-04-11 (limited by int64 nanoseconds)

## 12. Edge Cases

### 12.1 Empty Index

```python
pd.DatetimeIndex([])  # Empty DatetimeIndex with datetime64[ns] dtype
```

### 12.2 NaT Values

```python
pd.DatetimeIndex(['2024-01-01', pd.NaT, '2024-01-03'])  # Contains NaT
```

### 12.3 Frequency Inference

```python
# Regular spacing → freq is inferred
pd.DatetimeIndex(['2024-01-01', '2024-01-02', '2024-01-03']).freq  # 'D'

# Irregular → freq is None
pd.DatetimeIndex(['2024-01-01', '2024-01-03', '2024-01-07']).freq  # None
```

## 13. Priority Implementation Targets

### Phase 1: Core DatetimeIndex
1. `IndexLabel::Datetime64(i64)` variant (nanoseconds since epoch)
2. `Index::from_datetime64()` constructor
3. `date_range(start, end, periods, freq, name)` function
4. Component properties (year, month, day, hour, etc.)

### Phase 2: Arithmetic
1. DatetimeIndex + Timedelta → DatetimeIndex
2. DatetimeIndex - Timedelta → DatetimeIndex
3. DatetimeIndex - DatetimeIndex → TimedeltaIndex
4. Rounding: round, floor, ceil

### Phase 3: Advanced
1. Timezone support (tz_localize, tz_convert)
2. Set operations (union, intersection, difference)
3. Integration with existing to_datetime()
