#![forbid(unsafe_code)]

use std::collections::HashMap;

use fp_columnar::{Column, ColumnError};
use fp_frame::{FrameError, Series};
use fp_index::{Index, IndexError, IndexLabel, align_union, validate_alignment_plan};
use fp_runtime::{EvidenceLedger, RuntimePolicy};
use fp_types::{NullKind, Scalar};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroupByOptions {
    pub dropna: bool,
}

impl Default for GroupByOptions {
    fn default() -> Self {
        Self { dropna: true }
    }
}

#[derive(Debug, Error)]
pub enum GroupByError {
    #[error(transparent)]
    Frame(#[from] FrameError),
    #[error(transparent)]
    Index(#[from] IndexError),
    #[error(transparent)]
    Column(#[from] ColumnError),
}

pub fn groupby_sum(
    keys: &Series,
    values: &Series,
    options: GroupByOptions,
    _policy: &RuntimePolicy,
    _ledger: &mut EvidenceLedger,
) -> Result<Series, GroupByError> {
    let plan = align_union(keys.index(), values.index());
    validate_alignment_plan(&plan)?;

    let aligned_keys = keys.column().reindex_by_positions(&plan.left_positions)?;
    let aligned_values = values
        .column()
        .reindex_by_positions(&plan.right_positions)?;

    let mut ordering = Vec::<GroupKeyRef<'_>>::new();
    let mut slot = HashMap::<GroupKeyRef<'_>, (Scalar, f64)>::new();

    for (key, value) in aligned_keys.values().iter().zip(aligned_values.values()) {
        if options.dropna && key.is_missing() {
            continue;
        }

        let key_id = GroupKeyRef::from_scalar(key);
        let entry = slot.entry(key_id.clone()).or_insert_with(|| {
            ordering.push(key_id.clone());
            (key.clone(), 0.0)
        });

        if value.is_missing() {
            continue;
        }

        if let Ok(v) = value.to_f64() {
            entry.1 += v;
        }
    }

    let mut out_index = Vec::with_capacity(ordering.len());
    let mut out_values = Vec::with_capacity(ordering.len());

    for key in ordering {
        let (label, sum) = slot
            .remove(&key)
            .expect("ordering references only inserted keys");
        out_index.push(match label {
            Scalar::Int64(v) => IndexLabel::Int64(v),
            Scalar::Utf8(v) => IndexLabel::Utf8(v),
            Scalar::Bool(v) => IndexLabel::Utf8(v.to_string()),
            Scalar::Null(NullKind::NaN)
            | Scalar::Null(NullKind::NaT)
            | Scalar::Null(NullKind::Null) => IndexLabel::Utf8("<null>".to_owned()),
            Scalar::Float64(v) => IndexLabel::Utf8(v.to_string()),
        });
        out_values.push(Scalar::Float64(sum));
    }

    let out_column = Column::from_values(out_values)?;
    Ok(Series::new("sum", Index::new(out_index), out_column)?)
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
enum GroupKeyRef<'a> {
    Bool(bool),
    Int64(i64),
    FloatBits(u64),
    Utf8(&'a str),
    Null(NullKind),
}

impl<'a> GroupKeyRef<'a> {
    fn from_scalar(key: &'a Scalar) -> Self {
        match key {
            Scalar::Bool(v) => Self::Bool(*v),
            Scalar::Int64(v) => Self::Int64(*v),
            Scalar::Float64(v) => Self::FloatBits(if v.is_nan() {
                f64::NAN.to_bits()
            } else {
                v.to_bits()
            }),
            Scalar::Utf8(v) => Self::Utf8(v.as_str()),
            Scalar::Null(kind) => Self::Null(*kind),
        }
    }
}

#[cfg(test)]
mod tests {
    use fp_runtime::{EvidenceLedger, RuntimePolicy};
    use fp_types::Scalar;

    use super::{GroupByOptions, groupby_sum};
    use fp_frame::Series;

    #[test]
    fn groupby_sum_respects_first_seen_key_order() {
        let keys = Series::from_values(
            "key",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Utf8("b".to_owned()),
                Scalar::Utf8("a".to_owned()),
                Scalar::Utf8("b".to_owned()),
                Scalar::Utf8("a".to_owned()),
            ],
        )
        .expect("keys");

        let values = Series::from_values(
            "value",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Int64(3),
                Scalar::Int64(4),
            ],
        )
        .expect("values");

        let mut ledger = EvidenceLedger::new();
        let out = groupby_sum(
            &keys,
            &values,
            GroupByOptions::default(),
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .expect("groupby");

        assert_eq!(out.index().labels(), &["b".into(), "a".into()]);
        assert_eq!(out.values(), &[Scalar::Float64(4.0), Scalar::Float64(6.0)]);
    }
}
