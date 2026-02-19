#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use fp_columnar::ComparisonOp;
use fp_frame::{self, FrameError, Series};
use fp_runtime::{EvidenceLedger, RuntimePolicy};
use fp_types::Scalar;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SeriesRef(pub String);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Expr {
    Series {
        name: SeriesRef,
    },
    Add {
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Sub {
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Mul {
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Div {
        left: Box<Expr>,
        right: Box<Expr>,
    },
    And {
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Or {
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Not {
        expr: Box<Expr>,
    },
    Compare {
        left: Box<Expr>,
        right: Box<Expr>,
        op: ComparisonOp,
    },
    Literal {
        value: Scalar,
    },
}

#[derive(Debug, Clone, Default)]
pub struct EvalContext {
    series: BTreeMap<String, Series>,
}

impl EvalContext {
    #[must_use]
    pub fn new() -> Self {
        Self {
            series: BTreeMap::new(),
        }
    }

    pub fn insert_series(&mut self, series: Series) {
        self.series.insert(series.name().to_owned(), series);
    }

    #[must_use]
    pub fn get_series(&self, name: &str) -> Option<&Series> {
        self.series.get(name)
    }
}

#[derive(Debug, Error)]
pub enum ExprError {
    #[error("unknown series reference: {0}")]
    UnknownSeries(String),
    #[error("cannot evaluate a pure literal expression without an index anchor")]
    UnanchoredLiteral,
    #[error(transparent)]
    Frame(#[from] FrameError),
}

pub fn evaluate(
    expr: &Expr,
    context: &EvalContext,
    policy: &RuntimePolicy,
    ledger: &mut EvidenceLedger,
) -> Result<Series, ExprError> {
    match expr {
        Expr::Series { name } => context
            .get_series(&name.0)
            .cloned()
            .ok_or_else(|| ExprError::UnknownSeries(name.0.clone())),
        Expr::Add { left, right } => {
            let lhs = evaluate(left, context, policy, ledger)?;
            let rhs = evaluate(right, context, policy, ledger)?;
            lhs.add_with_policy(&rhs, policy, ledger)
                .map_err(ExprError::from)
        }
        Expr::Sub { left, right } => {
            let lhs = evaluate(left, context, policy, ledger)?;
            let rhs = evaluate(right, context, policy, ledger)?;
            lhs.sub_with_policy(&rhs, policy, ledger)
                .map_err(ExprError::from)
        }
        Expr::Mul { left, right } => {
            let lhs = evaluate(left, context, policy, ledger)?;
            let rhs = evaluate(right, context, policy, ledger)?;
            lhs.mul_with_policy(&rhs, policy, ledger)
                .map_err(ExprError::from)
        }
        Expr::Div { left, right } => {
            let lhs = evaluate(left, context, policy, ledger)?;
            let rhs = evaluate(right, context, policy, ledger)?;
            lhs.div_with_policy(&rhs, policy, ledger)
                .map_err(ExprError::from)
        }
        Expr::And { left, right } => {
            let lhs = evaluate(left, context, policy, ledger)?;
            let rhs = evaluate(right, context, policy, ledger)?;
            lhs.and(&rhs).map_err(ExprError::from)
        }
        Expr::Or { left, right } => {
            let lhs = evaluate(left, context, policy, ledger)?;
            let rhs = evaluate(right, context, policy, ledger)?;
            lhs.or(&rhs).map_err(ExprError::from)
        }
        Expr::Not { expr } => {
            let input = evaluate(expr, context, policy, ledger)?;
            input.not().map_err(ExprError::from)
        }
        Expr::Compare { left, right, op } => {
            evaluate_comparison(left, right, *op, context, policy, ledger)
        }
        Expr::Literal { .. } => Err(ExprError::UnanchoredLiteral),
    }
}

fn apply_series_comparison(
    left: &Series,
    right: &Series,
    op: ComparisonOp,
) -> Result<Series, ExprError> {
    match op {
        ComparisonOp::Gt => left.gt(right),
        ComparisonOp::Lt => left.lt(right),
        ComparisonOp::Eq => left.eq_series(right),
        ComparisonOp::Ne => left.ne_series(right),
        ComparisonOp::Ge => left.ge(right),
        ComparisonOp::Le => left.le(right),
    }
    .map_err(ExprError::from)
}

fn reverse_comparison_op(op: ComparisonOp) -> ComparisonOp {
    match op {
        ComparisonOp::Gt => ComparisonOp::Lt,
        ComparisonOp::Lt => ComparisonOp::Gt,
        ComparisonOp::Eq => ComparisonOp::Eq,
        ComparisonOp::Ne => ComparisonOp::Ne,
        ComparisonOp::Ge => ComparisonOp::Le,
        ComparisonOp::Le => ComparisonOp::Ge,
    }
}

fn evaluate_comparison(
    left: &Expr,
    right: &Expr,
    op: ComparisonOp,
    context: &EvalContext,
    policy: &RuntimePolicy,
    ledger: &mut EvidenceLedger,
) -> Result<Series, ExprError> {
    match (left, right) {
        (Expr::Literal { .. }, Expr::Literal { .. }) => Err(ExprError::UnanchoredLiteral),
        (Expr::Literal { value }, right_expr) => {
            let rhs = evaluate(right_expr, context, policy, ledger)?;
            rhs.compare_scalar(value, reverse_comparison_op(op))
                .map_err(ExprError::from)
        }
        (left_expr, Expr::Literal { value }) => {
            let lhs = evaluate(left_expr, context, policy, ledger)?;
            lhs.compare_scalar(value, op).map_err(ExprError::from)
        }
        (left_expr, right_expr) => {
            let lhs = evaluate(left_expr, context, policy, ledger)?;
            let rhs = evaluate(right_expr, context, policy, ledger)?;
            apply_series_comparison(&lhs, &rhs, op)
        }
    }
}

// ── AG-15: Incremental View Maintenance ────────────────────────────────

/// A delta represents new rows appended to a base series.
#[derive(Debug, Clone)]
pub struct Delta {
    pub series_name: String,
    pub new_labels: Vec<fp_index::IndexLabel>,
    pub new_values: Vec<Scalar>,
}

/// Cached result of a previous full evaluation, used as base for incremental updates.
#[derive(Debug, Clone)]
pub struct MaterializedView {
    pub expr: Expr,
    pub result: Series,
    pub base_snapshot: EvalContext,
}

impl MaterializedView {
    pub fn from_full_eval(
        expr: &Expr,
        context: &EvalContext,
        policy: &RuntimePolicy,
        ledger: &mut EvidenceLedger,
    ) -> Result<Self, ExprError> {
        let result = evaluate(expr, context, policy, ledger)?;
        Ok(Self {
            expr: expr.clone(),
            result,
            base_snapshot: context.clone(),
        })
    }

    /// Apply a delta (appended rows) incrementally.
    ///
    /// For linear expressions (series refs, arithmetic, anchored comparisons), only the new rows
    /// are computed and concatenated to the existing result. Falls back to
    /// full re-evaluation for expressions that cannot be incrementally maintained.
    pub fn apply_delta(
        &mut self,
        delta: &Delta,
        context: &EvalContext,
        policy: &RuntimePolicy,
        ledger: &mut EvidenceLedger,
    ) -> Result<&Series, ExprError> {
        // Build a context containing only the delta rows
        let delta_series = Series::from_values(
            &delta.series_name,
            delta.new_labels.clone(),
            delta.new_values.clone(),
        )
        .map_err(ExprError::from)?;

        let mut delta_ctx = context.clone();
        delta_ctx.insert_series(delta_series);

        // Check if the expression can be incrementally maintained
        if Self::is_linear(&self.expr) {
            // Evaluate only the delta portion
            let delta_result = evaluate_delta(&self.expr, &delta_ctx, delta, policy, ledger)?;

            // Concatenate: old result + delta result
            let combined =
                fp_frame::concat_series(&[&self.result, &delta_result]).map_err(ExprError::from)?;
            self.result = combined;
            self.base_snapshot = context.clone();
        } else {
            // Fallback: full re-evaluation
            self.result = evaluate(&self.expr, context, policy, ledger)?;
            self.base_snapshot = context.clone();
        }

        Ok(&self.result)
    }

    fn is_linear(expr: &Expr) -> bool {
        match expr {
            Expr::Series { .. } => true,
            Expr::Add { left, right }
            | Expr::Sub { left, right }
            | Expr::Mul { left, right }
            | Expr::Div { left, right }
            | Expr::And { left, right }
            | Expr::Or { left, right } => Self::is_linear(left) && Self::is_linear(right),
            Expr::Not { expr } => Self::is_linear(expr),
            Expr::Compare { left, right, .. } => {
                Self::is_linear_or_literal(left)
                    && Self::is_linear_or_literal(right)
                    && Self::comparison_is_anchored(left, right)
            }
            Expr::Literal { .. } => false,
        }
    }

    fn is_linear_or_literal(expr: &Expr) -> bool {
        matches!(expr, Expr::Literal { .. }) || Self::is_linear(expr)
    }

    fn comparison_is_anchored(left: &Expr, right: &Expr) -> bool {
        !(matches!(left, Expr::Literal { .. }) && matches!(right, Expr::Literal { .. }))
    }
}

/// Evaluate only the delta rows of an expression.
///
/// For `Expr::Series`, returns the delta rows for the named series.
/// For arithmetic/logical/comparison operators, recursively evaluates delta
/// operands and applies the operation only on appended rows.
fn evaluate_delta(
    expr: &Expr,
    delta_ctx: &EvalContext,
    delta: &Delta,
    policy: &RuntimePolicy,
    ledger: &mut EvidenceLedger,
) -> Result<Series, ExprError> {
    match expr {
        Expr::Series { name } => {
            if name.0 == delta.series_name {
                // This is the series that has the delta
                Series::from_values(&name.0, delta.new_labels.clone(), delta.new_values.clone())
                    .map_err(ExprError::from)
            } else {
                // Other series: extract the corresponding labels from the full series
                let full = delta_ctx
                    .get_series(&name.0)
                    .ok_or_else(|| ExprError::UnknownSeries(name.0.clone()))?;
                // For non-delta series in an add, we need the values at the delta labels.
                // If the other series doesn't have those labels, alignment will fill nulls.
                let reindexed = full
                    .reindex(delta.new_labels.clone())
                    .map_err(ExprError::from)?;
                Ok(reindexed)
            }
        }
        Expr::Add { left, right } => {
            let lhs = evaluate_delta(left, delta_ctx, delta, policy, ledger)?;
            let rhs = evaluate_delta(right, delta_ctx, delta, policy, ledger)?;
            lhs.add_with_policy(&rhs, policy, ledger)
                .map_err(ExprError::from)
        }
        Expr::Sub { left, right } => {
            let lhs = evaluate_delta(left, delta_ctx, delta, policy, ledger)?;
            let rhs = evaluate_delta(right, delta_ctx, delta, policy, ledger)?;
            lhs.sub_with_policy(&rhs, policy, ledger)
                .map_err(ExprError::from)
        }
        Expr::Mul { left, right } => {
            let lhs = evaluate_delta(left, delta_ctx, delta, policy, ledger)?;
            let rhs = evaluate_delta(right, delta_ctx, delta, policy, ledger)?;
            lhs.mul_with_policy(&rhs, policy, ledger)
                .map_err(ExprError::from)
        }
        Expr::Div { left, right } => {
            let lhs = evaluate_delta(left, delta_ctx, delta, policy, ledger)?;
            let rhs = evaluate_delta(right, delta_ctx, delta, policy, ledger)?;
            lhs.div_with_policy(&rhs, policy, ledger)
                .map_err(ExprError::from)
        }
        Expr::And { left, right } => {
            let lhs = evaluate_delta(left, delta_ctx, delta, policy, ledger)?;
            let rhs = evaluate_delta(right, delta_ctx, delta, policy, ledger)?;
            lhs.and(&rhs).map_err(ExprError::from)
        }
        Expr::Or { left, right } => {
            let lhs = evaluate_delta(left, delta_ctx, delta, policy, ledger)?;
            let rhs = evaluate_delta(right, delta_ctx, delta, policy, ledger)?;
            lhs.or(&rhs).map_err(ExprError::from)
        }
        Expr::Not { expr } => {
            let input = evaluate_delta(expr, delta_ctx, delta, policy, ledger)?;
            input.not().map_err(ExprError::from)
        }
        Expr::Compare { left, right, op } => {
            evaluate_delta_comparison(left, right, *op, delta_ctx, delta, policy, ledger)
        }
        Expr::Literal { .. } => Err(ExprError::UnanchoredLiteral),
    }
}

fn evaluate_delta_comparison(
    left: &Expr,
    right: &Expr,
    op: ComparisonOp,
    delta_ctx: &EvalContext,
    delta: &Delta,
    policy: &RuntimePolicy,
    ledger: &mut EvidenceLedger,
) -> Result<Series, ExprError> {
    match (left, right) {
        (Expr::Literal { .. }, Expr::Literal { .. }) => Err(ExprError::UnanchoredLiteral),
        (Expr::Literal { value }, right_expr) => {
            let rhs = evaluate_delta(right_expr, delta_ctx, delta, policy, ledger)?;
            rhs.compare_scalar(value, reverse_comparison_op(op))
                .map_err(ExprError::from)
        }
        (left_expr, Expr::Literal { value }) => {
            let lhs = evaluate_delta(left_expr, delta_ctx, delta, policy, ledger)?;
            lhs.compare_scalar(value, op).map_err(ExprError::from)
        }
        (left_expr, right_expr) => {
            let lhs = evaluate_delta(left_expr, delta_ctx, delta, policy, ledger)?;
            let rhs = evaluate_delta(right_expr, delta_ctx, delta, policy, ledger)?;
            apply_series_comparison(&lhs, &rhs, op)
        }
    }
}

#[cfg(test)]
mod tests {
    use fp_columnar::ComparisonOp;
    use fp_runtime::{EvidenceLedger, RuntimePolicy};
    use fp_types::Scalar;

    use super::{EvalContext, Expr, SeriesRef, evaluate};
    use fp_frame::Series;

    use super::{Delta, MaterializedView};

    #[test]
    fn expression_add_works_through_series_refs() {
        let a = Series::from_values(
            "a",
            vec![1_i64.into(), 2_i64.into()],
            vec![Scalar::Int64(1), Scalar::Int64(2)],
        )
        .expect("a");
        let b = Series::from_values(
            "b",
            vec![2_i64.into(), 3_i64.into()],
            vec![Scalar::Int64(10), Scalar::Int64(20)],
        )
        .expect("b");

        let mut ctx = EvalContext::new();
        ctx.insert_series(a);
        ctx.insert_series(b);

        let expr = Expr::Add {
            left: Box::new(Expr::Series {
                name: SeriesRef("a".to_owned()),
            }),
            right: Box::new(Expr::Series {
                name: SeriesRef("b".to_owned()),
            }),
        };

        let mut ledger = EvidenceLedger::new();
        let out = evaluate(
            &expr,
            &ctx,
            &RuntimePolicy::hardened(Some(10_000)),
            &mut ledger,
        )
        .expect("eval");
        assert_eq!(out.values()[1], Scalar::Int64(12));
    }

    #[test]
    fn expression_sub_mul_div_work_through_series_refs() {
        let a = Series::from_values(
            "a",
            vec![1_i64.into(), 2_i64.into()],
            vec![Scalar::Int64(8), Scalar::Int64(6)],
        )
        .expect("a");
        let b = Series::from_values(
            "b",
            vec![1_i64.into(), 2_i64.into()],
            vec![Scalar::Int64(2), Scalar::Int64(3)],
        )
        .expect("b");

        let mut ctx = EvalContext::new();
        ctx.insert_series(a);
        ctx.insert_series(b);
        let policy = RuntimePolicy::hardened(Some(10_000));

        let mut ledger = EvidenceLedger::new();
        let sub_out = evaluate(
            &Expr::Sub {
                left: Box::new(Expr::Series {
                    name: SeriesRef("a".to_owned()),
                }),
                right: Box::new(Expr::Series {
                    name: SeriesRef("b".to_owned()),
                }),
            },
            &ctx,
            &policy,
            &mut ledger,
        )
        .expect("sub eval");
        assert_eq!(sub_out.values(), &[Scalar::Int64(6), Scalar::Int64(3)]);

        let mut ledger = EvidenceLedger::new();
        let mul_out = evaluate(
            &Expr::Mul {
                left: Box::new(Expr::Series {
                    name: SeriesRef("a".to_owned()),
                }),
                right: Box::new(Expr::Series {
                    name: SeriesRef("b".to_owned()),
                }),
            },
            &ctx,
            &policy,
            &mut ledger,
        )
        .expect("mul eval");
        assert_eq!(mul_out.values(), &[Scalar::Int64(16), Scalar::Int64(18)]);

        let mut ledger = EvidenceLedger::new();
        let div_out = evaluate(
            &Expr::Div {
                left: Box::new(Expr::Series {
                    name: SeriesRef("a".to_owned()),
                }),
                right: Box::new(Expr::Series {
                    name: SeriesRef("b".to_owned()),
                }),
            },
            &ctx,
            &policy,
            &mut ledger,
        )
        .expect("div eval");
        assert_eq!(
            div_out.values(),
            &[Scalar::Float64(4.0), Scalar::Float64(2.0)]
        );
    }

    #[test]
    fn expression_compare_work_through_series_refs() {
        let a = Series::from_values(
            "a",
            vec![1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![Scalar::Int64(1), Scalar::Int64(3), Scalar::Int64(2)],
        )
        .expect("a");
        let b = Series::from_values(
            "b",
            vec![1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![Scalar::Int64(2), Scalar::Int64(2), Scalar::Int64(2)],
        )
        .expect("b");

        let mut ctx = EvalContext::new();
        ctx.insert_series(a);
        ctx.insert_series(b);
        let policy = RuntimePolicy::hardened(Some(10_000));

        let mut ledger = EvidenceLedger::new();
        let gt_out = evaluate(
            &Expr::Compare {
                left: Box::new(Expr::Series {
                    name: SeriesRef("a".to_owned()),
                }),
                right: Box::new(Expr::Series {
                    name: SeriesRef("b".to_owned()),
                }),
                op: ComparisonOp::Gt,
            },
            &ctx,
            &policy,
            &mut ledger,
        )
        .expect("gt eval");
        assert_eq!(
            gt_out.values(),
            &[Scalar::Bool(false), Scalar::Bool(true), Scalar::Bool(false)]
        );

        let mut ledger = EvidenceLedger::new();
        let eq_out = evaluate(
            &Expr::Compare {
                left: Box::new(Expr::Series {
                    name: SeriesRef("a".to_owned()),
                }),
                right: Box::new(Expr::Series {
                    name: SeriesRef("b".to_owned()),
                }),
                op: ComparisonOp::Eq,
            },
            &ctx,
            &policy,
            &mut ledger,
        )
        .expect("eq eval");
        assert_eq!(
            eq_out.values(),
            &[Scalar::Bool(false), Scalar::Bool(false), Scalar::Bool(true)]
        );
    }

    #[test]
    fn expression_compare_supports_series_scalar_and_scalar_series() {
        let a = Series::from_values(
            "a",
            vec![1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)],
        )
        .expect("a");

        let mut ctx = EvalContext::new();
        ctx.insert_series(a);
        let policy = RuntimePolicy::hardened(Some(10_000));

        let mut ledger = EvidenceLedger::new();
        let series_gt_scalar = evaluate(
            &Expr::Compare {
                left: Box::new(Expr::Series {
                    name: SeriesRef("a".to_owned()),
                }),
                right: Box::new(Expr::Literal {
                    value: Scalar::Int64(2),
                }),
                op: ComparisonOp::Gt,
            },
            &ctx,
            &policy,
            &mut ledger,
        )
        .expect("series > scalar");
        assert_eq!(
            series_gt_scalar.values(),
            &[Scalar::Bool(false), Scalar::Bool(false), Scalar::Bool(true)]
        );

        let mut ledger = EvidenceLedger::new();
        let scalar_ge_series = evaluate(
            &Expr::Compare {
                left: Box::new(Expr::Literal {
                    value: Scalar::Int64(2),
                }),
                right: Box::new(Expr::Series {
                    name: SeriesRef("a".to_owned()),
                }),
                op: ComparisonOp::Ge,
            },
            &ctx,
            &policy,
            &mut ledger,
        )
        .expect("scalar >= series");
        assert_eq!(
            scalar_ge_series.values(),
            &[Scalar::Bool(true), Scalar::Bool(true), Scalar::Bool(false)]
        );
    }

    #[test]
    fn expression_logical_ops_support_boolean_masks() {
        let a = Series::from_values(
            "a",
            vec![1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Bool(true),
                Scalar::Bool(false),
                Scalar::Null(fp_types::NullKind::Null),
            ],
        )
        .expect("a");
        let b = Series::from_values(
            "b",
            vec![2_i64.into(), 3_i64.into(), 4_i64.into()],
            vec![
                Scalar::Bool(true),
                Scalar::Null(fp_types::NullKind::Null),
                Scalar::Bool(false),
            ],
        )
        .expect("b");

        let mut ctx = EvalContext::new();
        ctx.insert_series(a);
        ctx.insert_series(b);
        let policy = RuntimePolicy::hardened(Some(10_000));

        let and_expr = Expr::And {
            left: Box::new(Expr::Series {
                name: SeriesRef("a".to_owned()),
            }),
            right: Box::new(Expr::Series {
                name: SeriesRef("b".to_owned()),
            }),
        };
        let mut ledger = EvidenceLedger::new();
        let and_out = evaluate(&and_expr, &ctx, &policy, &mut ledger).expect("and eval");
        assert_eq!(
            and_out.values(),
            &[
                Scalar::Null(fp_types::NullKind::Null),
                Scalar::Bool(false),
                Scalar::Null(fp_types::NullKind::Null),
                Scalar::Bool(false)
            ]
        );

        let or_expr = Expr::Or {
            left: Box::new(Expr::Series {
                name: SeriesRef("a".to_owned()),
            }),
            right: Box::new(Expr::Series {
                name: SeriesRef("b".to_owned()),
            }),
        };
        let mut ledger = EvidenceLedger::new();
        let or_out = evaluate(&or_expr, &ctx, &policy, &mut ledger).expect("or eval");
        assert_eq!(
            or_out.values(),
            &[
                Scalar::Bool(true),
                Scalar::Bool(true),
                Scalar::Null(fp_types::NullKind::Null),
                Scalar::Null(fp_types::NullKind::Null)
            ]
        );

        let not_expr = Expr::Not {
            expr: Box::new(Expr::Series {
                name: SeriesRef("a".to_owned()),
            }),
        };
        let mut ledger = EvidenceLedger::new();
        let not_out = evaluate(&not_expr, &ctx, &policy, &mut ledger).expect("not eval");
        assert_eq!(
            not_out.values(),
            &[
                Scalar::Bool(false),
                Scalar::Bool(true),
                Scalar::Null(fp_types::NullKind::Null)
            ]
        );
    }

    // === AG-15: Incremental View Maintenance Tests ===

    fn make_series(name: &str, labels: Vec<i64>, values: Vec<Scalar>) -> Series {
        Series::from_values(
            name,
            labels.into_iter().map(fp_index::IndexLabel::from).collect(),
            values,
        )
        .expect("series")
    }

    #[test]
    fn materialized_view_from_full_eval() {
        let a = make_series("a", vec![0, 1], vec![Scalar::Int64(10), Scalar::Int64(20)]);
        let mut ctx = EvalContext::new();
        ctx.insert_series(a);

        let expr = Expr::Series {
            name: SeriesRef("a".into()),
        };
        let mut ledger = EvidenceLedger::new();
        let policy = RuntimePolicy::hardened(Some(10_000));

        let view =
            MaterializedView::from_full_eval(&expr, &ctx, &policy, &mut ledger).expect("full eval");
        assert_eq!(view.result.values().len(), 2);
    }

    #[test]
    fn ivm_append_delta_series_ref() {
        let a = make_series("a", vec![0, 1], vec![Scalar::Int64(10), Scalar::Int64(20)]);
        let mut ctx = EvalContext::new();
        ctx.insert_series(a);

        let expr = Expr::Series {
            name: SeriesRef("a".into()),
        };
        let mut ledger = EvidenceLedger::new();
        let policy = RuntimePolicy::hardened(Some(10_000));

        let mut view =
            MaterializedView::from_full_eval(&expr, &ctx, &policy, &mut ledger).expect("full eval");
        assert_eq!(view.result.values().len(), 2);

        // Append 2 new rows
        let delta = Delta {
            series_name: "a".into(),
            new_labels: vec![2_i64.into(), 3_i64.into()],
            new_values: vec![Scalar::Int64(30), Scalar::Int64(40)],
        };

        // Update context with full new series
        let a_full = make_series(
            "a",
            vec![0, 1, 2, 3],
            vec![
                Scalar::Int64(10),
                Scalar::Int64(20),
                Scalar::Int64(30),
                Scalar::Int64(40),
            ],
        );
        ctx.insert_series(a_full);

        view.apply_delta(&delta, &ctx, &policy, &mut ledger)
            .expect("delta");
        assert_eq!(view.result.values().len(), 4);
        assert_eq!(view.result.values()[2], Scalar::Int64(30));
        assert_eq!(view.result.values()[3], Scalar::Int64(40));
    }

    #[test]
    fn ivm_append_delta_add_expression() {
        let a = make_series("a", vec![0, 1], vec![Scalar::Int64(1), Scalar::Int64(2)]);
        let b = make_series("b", vec![0, 1], vec![Scalar::Int64(10), Scalar::Int64(20)]);
        let mut ctx = EvalContext::new();
        ctx.insert_series(a);
        ctx.insert_series(b.clone());

        let expr = Expr::Add {
            left: Box::new(Expr::Series {
                name: SeriesRef("a".into()),
            }),
            right: Box::new(Expr::Series {
                name: SeriesRef("b".into()),
            }),
        };
        let mut ledger = EvidenceLedger::new();
        let policy = RuntimePolicy::hardened(Some(10_000));

        let mut view =
            MaterializedView::from_full_eval(&expr, &ctx, &policy, &mut ledger).expect("full eval");
        assert_eq!(view.result.values().len(), 2);
        assert_eq!(view.result.values()[0], Scalar::Int64(11));
        assert_eq!(view.result.values()[1], Scalar::Int64(22));

        // Append rows to "a" — "b" needs corresponding rows at labels 2,3
        let delta = Delta {
            series_name: "a".into(),
            new_labels: vec![2_i64.into(), 3_i64.into()],
            new_values: vec![Scalar::Int64(3), Scalar::Int64(4)],
        };

        let a_full = make_series(
            "a",
            vec![0, 1, 2, 3],
            vec![
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Int64(3),
                Scalar::Int64(4),
            ],
        );
        let b_full = make_series(
            "b",
            vec![0, 1, 2, 3],
            vec![
                Scalar::Int64(10),
                Scalar::Int64(20),
                Scalar::Int64(30),
                Scalar::Int64(40),
            ],
        );
        ctx.insert_series(a_full);
        ctx.insert_series(b_full);

        view.apply_delta(&delta, &ctx, &policy, &mut ledger)
            .expect("delta");
        assert_eq!(view.result.values().len(), 4);
        // New rows: 3+30=33, 4+40=44
        assert_eq!(view.result.values()[2], Scalar::Int64(33));
        assert_eq!(view.result.values()[3], Scalar::Int64(44));
    }

    #[test]
    fn ivm_append_delta_mul_expression() {
        let a = make_series("a", vec![0, 1], vec![Scalar::Int64(2), Scalar::Int64(3)]);
        let b = make_series("b", vec![0, 1], vec![Scalar::Int64(4), Scalar::Int64(5)]);
        let mut ctx = EvalContext::new();
        ctx.insert_series(a);
        ctx.insert_series(b);

        let expr = Expr::Mul {
            left: Box::new(Expr::Series {
                name: SeriesRef("a".into()),
            }),
            right: Box::new(Expr::Series {
                name: SeriesRef("b".into()),
            }),
        };
        let mut ledger = EvidenceLedger::new();
        let policy = RuntimePolicy::hardened(Some(10_000));

        let mut view =
            MaterializedView::from_full_eval(&expr, &ctx, &policy, &mut ledger).expect("base");
        assert_eq!(view.result.values(), &[Scalar::Int64(8), Scalar::Int64(15)]);

        let delta = Delta {
            series_name: "a".into(),
            new_labels: vec![2_i64.into(), 3_i64.into()],
            new_values: vec![Scalar::Int64(4), Scalar::Int64(6)],
        };
        let a_full = make_series(
            "a",
            vec![0, 1, 2, 3],
            vec![
                Scalar::Int64(2),
                Scalar::Int64(3),
                Scalar::Int64(4),
                Scalar::Int64(6),
            ],
        );
        let b_full = make_series(
            "b",
            vec![0, 1, 2, 3],
            vec![
                Scalar::Int64(4),
                Scalar::Int64(5),
                Scalar::Int64(6),
                Scalar::Int64(7),
            ],
        );
        ctx.insert_series(a_full);
        ctx.insert_series(b_full);

        view.apply_delta(&delta, &ctx, &policy, &mut ledger)
            .expect("delta");
        assert_eq!(
            view.result.values(),
            &[
                Scalar::Int64(8),
                Scalar::Int64(15),
                Scalar::Int64(24),
                Scalar::Int64(42)
            ]
        );
    }

    #[test]
    fn ivm_append_delta_comparison_expression() {
        let a = make_series("a", vec![0, 1], vec![Scalar::Int64(1), Scalar::Int64(2)]);
        let mut ctx = EvalContext::new();
        ctx.insert_series(a);

        let expr = Expr::Compare {
            left: Box::new(Expr::Series {
                name: SeriesRef("a".into()),
            }),
            right: Box::new(Expr::Literal {
                value: Scalar::Int64(1),
            }),
            op: ComparisonOp::Gt,
        };
        let mut ledger = EvidenceLedger::new();
        let policy = RuntimePolicy::hardened(Some(10_000));

        let mut view =
            MaterializedView::from_full_eval(&expr, &ctx, &policy, &mut ledger).expect("base");
        assert_eq!(
            view.result.values(),
            &[Scalar::Bool(false), Scalar::Bool(true)]
        );

        let delta = Delta {
            series_name: "a".into(),
            new_labels: vec![2_i64.into(), 3_i64.into()],
            new_values: vec![Scalar::Int64(3), Scalar::Int64(0)],
        };
        ctx.insert_series(make_series(
            "a",
            vec![0, 1, 2, 3],
            vec![
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Int64(3),
                Scalar::Int64(0),
            ],
        ));

        view.apply_delta(&delta, &ctx, &policy, &mut ledger)
            .expect("delta");
        assert_eq!(
            view.result.values(),
            &[
                Scalar::Bool(false),
                Scalar::Bool(true),
                Scalar::Bool(true),
                Scalar::Bool(false)
            ]
        );
    }

    #[test]
    fn ivm_append_delta_logical_expression() {
        let a = make_series(
            "a",
            vec![0, 1],
            vec![Scalar::Bool(false), Scalar::Bool(true)],
        );
        let b = make_series(
            "b",
            vec![0, 1],
            vec![Scalar::Bool(true), Scalar::Bool(true)],
        );
        let mut ctx = EvalContext::new();
        ctx.insert_series(a);
        ctx.insert_series(b);

        let expr = Expr::And {
            left: Box::new(Expr::Series {
                name: SeriesRef("a".into()),
            }),
            right: Box::new(Expr::Series {
                name: SeriesRef("b".into()),
            }),
        };
        let mut ledger = EvidenceLedger::new();
        let policy = RuntimePolicy::hardened(Some(10_000));

        let mut view =
            MaterializedView::from_full_eval(&expr, &ctx, &policy, &mut ledger).expect("base");
        assert_eq!(
            view.result.values(),
            &[Scalar::Bool(false), Scalar::Bool(true)]
        );

        let delta = Delta {
            series_name: "a".into(),
            new_labels: vec![2_i64.into()],
            new_values: vec![Scalar::Bool(true)],
        };
        let a_full = make_series(
            "a",
            vec![0, 1, 2],
            vec![Scalar::Bool(false), Scalar::Bool(true), Scalar::Bool(true)],
        );
        let b_full = make_series(
            "b",
            vec![0, 1, 2],
            vec![Scalar::Bool(true), Scalar::Bool(true), Scalar::Bool(false)],
        );
        ctx.insert_series(a_full);
        ctx.insert_series(b_full);

        view.apply_delta(&delta, &ctx, &policy, &mut ledger)
            .expect("delta");
        assert_eq!(
            view.result.values(),
            &[Scalar::Bool(false), Scalar::Bool(true), Scalar::Bool(false)]
        );
    }

    #[test]
    fn ivm_isomorphism_incremental_matches_full() {
        // The key correctness property: incremental result must equal full re-eval.
        let a = make_series("a", vec![0, 1], vec![Scalar::Int64(5), Scalar::Int64(10)]);
        let b = make_series(
            "b",
            vec![0, 1],
            vec![Scalar::Int64(100), Scalar::Int64(200)],
        );
        let mut ctx = EvalContext::new();
        ctx.insert_series(a);
        ctx.insert_series(b);

        let expr = Expr::Add {
            left: Box::new(Expr::Series {
                name: SeriesRef("a".into()),
            }),
            right: Box::new(Expr::Series {
                name: SeriesRef("b".into()),
            }),
        };
        let mut ledger = EvidenceLedger::new();
        let policy = RuntimePolicy::hardened(Some(10_000));

        let mut view =
            MaterializedView::from_full_eval(&expr, &ctx, &policy, &mut ledger).expect("base");

        // Apply delta
        let delta = Delta {
            series_name: "a".into(),
            new_labels: vec![2_i64.into()],
            new_values: vec![Scalar::Int64(15)],
        };
        let a_full = make_series(
            "a",
            vec![0, 1, 2],
            vec![Scalar::Int64(5), Scalar::Int64(10), Scalar::Int64(15)],
        );
        let b_full = make_series(
            "b",
            vec![0, 1, 2],
            vec![Scalar::Int64(100), Scalar::Int64(200), Scalar::Int64(300)],
        );
        ctx.insert_series(a_full);
        ctx.insert_series(b_full);

        view.apply_delta(&delta, &ctx, &policy, &mut ledger)
            .expect("incremental");

        // Full re-evaluation for comparison
        let full_result = evaluate(&expr, &ctx, &policy, &mut ledger).expect("full");

        // Compare: incremental result must match full
        assert_eq!(view.result.values().len(), full_result.values().len());
        for (i, (inc, full)) in view
            .result
            .values()
            .iter()
            .zip(full_result.values().iter())
            .enumerate()
        {
            assert!(
                inc.semantic_eq(full),
                "mismatch at position {i}: incremental={inc:?} full={full:?}"
            );
        }
    }

    #[test]
    fn ivm_multiple_deltas() {
        let a = make_series("a", vec![0], vec![Scalar::Int64(1)]);
        let mut ctx = EvalContext::new();
        ctx.insert_series(a);

        let expr = Expr::Series {
            name: SeriesRef("a".into()),
        };
        let mut ledger = EvidenceLedger::new();
        let policy = RuntimePolicy::hardened(Some(10_000));

        let mut view =
            MaterializedView::from_full_eval(&expr, &ctx, &policy, &mut ledger).expect("base");

        // First delta
        let delta1 = Delta {
            series_name: "a".into(),
            new_labels: vec![1_i64.into()],
            new_values: vec![Scalar::Int64(2)],
        };
        ctx.insert_series(make_series(
            "a",
            vec![0, 1],
            vec![Scalar::Int64(1), Scalar::Int64(2)],
        ));
        view.apply_delta(&delta1, &ctx, &policy, &mut ledger)
            .expect("delta1");
        assert_eq!(view.result.values().len(), 2);

        // Second delta
        let delta2 = Delta {
            series_name: "a".into(),
            new_labels: vec![2_i64.into(), 3_i64.into()],
            new_values: vec![Scalar::Int64(3), Scalar::Int64(4)],
        };
        ctx.insert_series(make_series(
            "a",
            vec![0, 1, 2, 3],
            vec![
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Int64(3),
                Scalar::Int64(4),
            ],
        ));
        view.apply_delta(&delta2, &ctx, &policy, &mut ledger)
            .expect("delta2");
        assert_eq!(view.result.values().len(), 4);
        assert_eq!(view.result.values()[3], Scalar::Int64(4));
    }

    #[test]
    fn ivm_is_linear_detects_expressions() {
        assert!(MaterializedView::is_linear(&Expr::Series {
            name: SeriesRef("a".into()),
        }));
        assert!(MaterializedView::is_linear(&Expr::Add {
            left: Box::new(Expr::Series {
                name: SeriesRef("a".into()),
            }),
            right: Box::new(Expr::Series {
                name: SeriesRef("b".into()),
            }),
        }));
        assert!(MaterializedView::is_linear(&Expr::Sub {
            left: Box::new(Expr::Series {
                name: SeriesRef("a".into()),
            }),
            right: Box::new(Expr::Series {
                name: SeriesRef("b".into()),
            }),
        }));
        assert!(MaterializedView::is_linear(&Expr::Mul {
            left: Box::new(Expr::Series {
                name: SeriesRef("a".into()),
            }),
            right: Box::new(Expr::Series {
                name: SeriesRef("b".into()),
            }),
        }));
        assert!(MaterializedView::is_linear(&Expr::Div {
            left: Box::new(Expr::Series {
                name: SeriesRef("a".into()),
            }),
            right: Box::new(Expr::Series {
                name: SeriesRef("b".into()),
            }),
        }));
        assert!(MaterializedView::is_linear(&Expr::And {
            left: Box::new(Expr::Series {
                name: SeriesRef("a".into()),
            }),
            right: Box::new(Expr::Series {
                name: SeriesRef("b".into()),
            }),
        }));
        assert!(MaterializedView::is_linear(&Expr::Or {
            left: Box::new(Expr::Series {
                name: SeriesRef("a".into()),
            }),
            right: Box::new(Expr::Series {
                name: SeriesRef("b".into()),
            }),
        }));
        assert!(MaterializedView::is_linear(&Expr::Not {
            expr: Box::new(Expr::Series {
                name: SeriesRef("a".into()),
            }),
        }));
        assert!(MaterializedView::is_linear(&Expr::Compare {
            left: Box::new(Expr::Series {
                name: SeriesRef("a".into()),
            }),
            right: Box::new(Expr::Series {
                name: SeriesRef("b".into()),
            }),
            op: ComparisonOp::Ge,
        }));
        assert!(MaterializedView::is_linear(&Expr::Compare {
            left: Box::new(Expr::Series {
                name: SeriesRef("a".into()),
            }),
            right: Box::new(Expr::Literal {
                value: Scalar::Int64(1),
            }),
            op: ComparisonOp::Gt,
        }));
        assert!(!MaterializedView::is_linear(&Expr::Compare {
            left: Box::new(Expr::Literal {
                value: Scalar::Int64(1),
            }),
            right: Box::new(Expr::Literal {
                value: Scalar::Int64(2),
            }),
            op: ComparisonOp::Lt,
        }));
        assert!(!MaterializedView::is_linear(&Expr::Literal {
            value: Scalar::Int64(42),
        }));
    }
}
