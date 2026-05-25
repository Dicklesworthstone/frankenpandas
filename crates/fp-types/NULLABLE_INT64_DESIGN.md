# Nullable Int64 Extension Dtype Design

**Bead:** br-frankenpandas-rg8ys.6.1
**Status:** Design Phase
**Date:** 2026-05-25

## Problem Statement

FrankenPandas currently promotes `Int64` to `Float64` when null values are introduced (DISC-011/014). This differs from pandas' nullable `Int64` extension dtype which preserves integer semantics with `pd.NA`.

## Pandas Semantics

### Dtype Variants
| pandas dtype | Storage | Null handling | Example |
|--------------|---------|---------------|---------|
| `int64` (numpy) | 64-bit int | Promotes to float64 | `np.array([1, 2])` |
| `Int64` (extension) | 64-bit int + mask | Preserves Int64 with NA | `pd.array([1, pd.NA])` |

### Promotion Rules
| Operation | int64 result | Int64 result |
|-----------|--------------|--------------|
| Arithmetic with valid | int64 | Int64 |
| Arithmetic introducing NA | float64+NaN | Int64+NA |
| Concat with nulls | float64 | Int64 |
| Index alignment miss | float64 | Int64 |

## Current FrankenPandas Infrastructure

```rust
// fp-types: DType enum (no nullable distinction)
pub enum DType {
    Int64,      // Used for both nullable and non-nullable
    Float64,
    // ...
}

// fp-columnar: Column already has validity mask
pub struct Column {
    dtype: DType,
    values: Vec<Scalar>,
    validity: ValidityMask,  // Already tracks nulls!
}
```

## Design Options

### Option A: Distinct DType Variants (Recommended)
Add `Int64Nullable` (and potentially `BoolNullable`) variants:

```rust
pub enum DType {
    Int64,          // Non-nullable numpy int64
    Int64Nullable,  // Nullable extension Int64
    Float64,
    Bool,
    BoolNullable,   // Nullable extension boolean
    // ...
}
```

**Pros:**
- Clear separation of numpy vs extension semantics
- Dtype reporting matches pandas (`"Int64"` vs `"int64"`)
- Promotion logic is explicit in type

**Cons:**
- More variants to handle in match statements

### Option B: Nullable Flag on Column
Add a flag to Column indicating extension dtype:

```rust
pub struct Column {
    dtype: DType,
    values: Vec<Scalar>,
    validity: ValidityMask,
    is_extension: bool,  // If true, Int64 reports as "Int64"
}
```

**Pros:**
- Minimal DType changes

**Cons:**
- Dtype reporting requires runtime check
- Promotion logic depends on flag, not type

### Recommendation: Option A

Use distinct `DType::Int64Nullable` variant for cleaner type-level semantics.

## Implementation Plan

### Phase 1: fp-types (T6_CORE)
1. Add `DType::Int64Nullable` variant
2. Update `common_dtype()` with nullable promotion rules
3. Add `DType::is_nullable()` helper
4. Update `cast_scalar()` for Int64 ↔ Int64Nullable

### Phase 2: fp-columnar (T6_COL)
1. Update `Column::from_scalars()` to preserve nullable dtype
2. Update arithmetic kernels to preserve Int64Nullable
3. Add `Column::to_nullable()` / `Column::to_non_nullable()` conversions

### Phase 3: fp-frame (T6_WIRE)
1. Update Series/DataFrame constructors for nullable inference
2. Wire nullable through reductions (sum, mean skip NA)
3. Update concat/alignment to preserve nullable

### Phase 4: fp-io (T6_IO)
1. JSON/CSV round-trip preserving nullable dtype
2. Dtype reporting as "Int64" (capital I)

## Promotion Matrix

| Left | Right | Result (no nulls) | Result (with nulls) |
|------|-------|-------------------|---------------------|
| int64 | int64 | int64 | float64 |
| Int64 | Int64 | Int64 | Int64 |
| int64 | Int64 | Int64 | Int64 |
| Int64 | float64 | float64 | float64 |
| Int64 | NA | Int64 | Int64 |

## Migration Plan for ctmet Packets

Once Int64Nullable is implemented:
1. Regenerate fixtures that currently accept Float64 to expect Int64
2. Update tests to verify Int64 preservation
3. Remove DISC-011/014 waivers

## DoD Checklist
- [x] Design note committed
- [ ] Reviewed before T6_CORE implementation
- [ ] Promotion matrix verified against pandas 2.2.3
