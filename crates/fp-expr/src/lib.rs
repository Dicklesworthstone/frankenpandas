#![forbid(unsafe_code)]
#![warn(rustdoc::broken_intra_doc_links)]

//! Expression engine for **frankenpandas** — implements pandas
//! `DataFrame.eval()` / `DataFrame.query()` semantics for Rust
//! callers and provides the underlying [`Expr`] AST + parser /
//! evaluator that the rest of the workspace can build on.
//!
//! ## Why
//!
//! Pandas users write things like:
//! ```python
//! df.query("price > 100 and category == 'A'")
//! df.eval("margin = revenue - cost")
//! ```
//!
//! This crate gives Rust callers the equivalent: `df.query(...)`,
//! `df.eval(...)`, with a parsed [`Expr`] AST you can build
//! programmatically or via the string parser.
//!
//! ## Public surface
//!
//! - [`Expr`]: the expression AST. Built either by hand
//!   ([`Expr::Series`], [`Expr::Add`], [`Expr::Sub`],
//!   [`Expr::Local`], etc.) or by parsing a pandas-style string
//!   with [`parse_expr`].
//! - [`SeriesRef`]: a typed column-name reference. Used by `Expr`
//!   variants and by callers building expressions programmatically.
//! - [`EvalContext`]: the evaluation environment — bindings for
//!   local variables (the `@local_var` syntax in pandas eval) plus
//!   the [`RuntimePolicy`] / [`EvidenceLedger`] from fp-runtime
//!   for decision recording.
//! - [`ExprError`]: failure modes (parse error, unknown column,
//!   type mismatch, division by zero, ...).
//!
//! ## Evaluation entry points
//!
//! Direct AST eval:
//! - [`evaluate`]: evaluate an [`Expr`] against a Series-like input.
//! - [`evaluate_on_dataframe`]: bind columns from a DataFrame as
//!   identifiers and evaluate.
//! - [`evaluate_on_dataframe_with_locals`]: same, plus an external
//!   `@local` binding map.
//! - [`filter_dataframe_on_expr`] /
//!   [`filter_dataframe_on_expr_with_locals`]: shortcut for
//!   `df[df.eval(expr)]` — the boolean-mask filter pattern.
//!
//! String entry points (parse-then-eval):
//! - [`eval_str`] / [`eval_str_with_locals`]: pandas
//!   `df.eval(string)` — returns a new Series / DataFrame column.
//! - [`query_str`] / [`query_str_with_locals`]: pandas
//!   `df.query(string)` — returns the row-filtered DataFrame.
//! - [`parse_expr`]: the standalone parser if you only want the
//!   AST.
//!
//! ## DataFrame extension trait
//!
//! [`DataFrameExprExt`] adds `df.eval(expr)` / `df.query(expr)`
//! method-style entry points on `DataFrame` so users can call them
//! fluently after `use fp_expr::DataFrameExprExt;`.
//!
//! ## Incremental views
//!
//! [`MaterializedView`] + [`Delta`]: the foundation for incremental
//! `eval`-derived columns. A `MaterializedView` caches the last
//! result and the input fingerprint; on next call, only re-evaluates
//! when inputs change. `Delta` records what changed for downstream
//! consumers.
//!
//! ## Cross-crate relationships
//!
//! - **fp-types** (`Scalar`), **fp-columnar** (`ComparisonOp`),
//!   **fp-index** (`Index`), **fp-frame** (`Series`, `DataFrame`,
//!   `FrameError`) are all consumed.
//! - **fp-runtime** (`RuntimePolicy`, `EvidenceLedger`) provides
//!   the optional decision-policy hook threaded through
//!   `EvalContext`.

use std::collections::BTreeMap;

use fp_columnar::ComparisonOp;
use fp_frame::{self, FrameError, Series};
use fp_index::{Index, IndexLabel};
use fp_runtime::{EvidenceLedger, RuntimePolicy};
use fp_types::Scalar;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SeriesRef(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BetweenInclusive {
    Both,
    Left,
    Right,
    Neither,
}

impl BetweenInclusive {
    fn parse(value: &str) -> Result<Self, ExprError> {
        match value {
            "both" => Ok(Self::Both),
            "left" => Ok(Self::Left),
            "right" => Ok(Self::Right),
            "neither" => Ok(Self::Neither),
            other => Err(ExprError::ParseError(format!(
                "between() inclusive must be one of 'both', 'left', 'right', or 'neither', got {other:?}"
            ))),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Both => "both",
            Self::Left => "left",
            Self::Right => "right",
            Self::Neither => "neither",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Expr {
    Series {
        name: SeriesRef,
    },
    Local {
        name: String,
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
    Modulo {
        left: Box<Expr>,
        right: Box<Expr>,
    },
    FloorDiv {
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Pow {
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
    Abs {
        expr: Box<Expr>,
    },
    Round {
        expr: Box<Expr>,
        decimals: i32,
    },
    IsNull {
        expr: Box<Expr>,
        negated: bool,
    },
    Between {
        expr: Box<Expr>,
        left: Scalar,
        right: Scalar,
        inclusive: BetweenInclusive,
    },
    Clip {
        expr: Box<Expr>,
        lower: f64,
        upper: f64,
    },
    Compare {
        left: Box<Expr>,
        right: Box<Expr>,
        op: ComparisonOp,
    },
    IsIn {
        left: Box<Expr>,
        values: Vec<Scalar>,
        negated: bool,
    },
    Literal {
        value: Scalar,
    },
}

#[derive(Debug, Clone, Default)]
pub struct EvalContext {
    series: BTreeMap<String, Series>,
    locals: BTreeMap<String, Scalar>,
    anchor_index: Option<Index>,
}

impl EvalContext {
    #[must_use]
    pub fn new() -> Self {
        Self {
            series: BTreeMap::new(),
            locals: BTreeMap::new(),
            anchor_index: None,
        }
    }

    pub fn insert_series(&mut self, series: Series) {
        if self.anchor_index.is_none() {
            self.anchor_index = Some(series.index().clone());
        }
        self.series.insert(series.name().to_owned(), series);
    }

    pub fn from_dataframe(frame: &fp_frame::DataFrame) -> Result<Self, ExprError> {
        Self::from_dataframe_with_locals(frame, &BTreeMap::new())
    }

    pub fn from_dataframe_with_locals(
        frame: &fp_frame::DataFrame,
        locals: &BTreeMap<String, Scalar>,
    ) -> Result<Self, ExprError> {
        let mut context = Self {
            series: BTreeMap::new(),
            locals: locals.clone(),
            anchor_index: Some(frame.index().clone()),
        };
        context.insert_index_series("index", frame.index())?;
        context.insert_index_series("ilevel_0", frame.index())?;
        if let Some(name) = frame.index().name() {
            context.insert_index_series(name, frame.index())?;
        }
        for (name, column) in frame.columns() {
            let series = Series::new(name.clone(), frame.index().clone(), column.clone())?;
            context.insert_series(series);
        }
        Ok(context)
    }

    #[must_use]
    pub fn get_series(&self, name: &str) -> Option<&Series> {
        self.series.get(name)
    }

    pub fn insert_local(&mut self, name: impl Into<String>, value: Scalar) {
        self.locals.insert(name.into(), value);
    }

    fn insert_index_series(&mut self, name: &str, index: &Index) -> Result<(), ExprError> {
        let values = index.labels().iter().map(index_label_to_scalar).collect();
        let series = Series::from_values(name, index.labels().to_vec(), values)?;
        self.insert_series(series);
        Ok(())
    }

    #[must_use]
    pub fn get_local(&self, name: &str) -> Option<&Scalar> {
        self.locals.get(name)
    }

    fn broadcast_local(&self, name: &str, value: &Scalar) -> Result<Series, ExprError> {
        let index = self
            .anchor_index
            .as_ref()
            .ok_or_else(|| ExprError::UnanchoredLocal(name.to_owned()))?;
        Series::broadcast(name, value.clone(), index.labels().to_vec()).map_err(ExprError::from)
    }
}

fn index_label_to_scalar(label: &IndexLabel) -> Scalar {
    match label {
        IndexLabel::Int64(value) => Scalar::Int64(*value),
        IndexLabel::Utf8(value) => Scalar::Utf8(value.clone()),
        IndexLabel::Timedelta64(value) => Scalar::Timedelta64(*value),
        IndexLabel::Datetime64(value) => Scalar::Datetime64(*value),
    }
}

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ExprError {
    #[error("unknown series reference: {0}")]
    UnknownSeries(String),
    #[error("unknown local reference: @{0}")]
    UnknownLocal(String),
    #[error("cannot evaluate a pure literal expression without an index anchor")]
    UnanchoredLiteral,
    #[error("cannot evaluate local reference @{0} without an index anchor")]
    UnanchoredLocal(String),
    #[error("parse error: {0}")]
    ParseError(String),
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
        Expr::Local { name } => {
            let value = context
                .get_local(name)
                .ok_or_else(|| ExprError::UnknownLocal(name.clone()))?;
            context.broadcast_local(name, value)
        }
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
        Expr::Modulo { left, right } => {
            let lhs = evaluate(left, context, policy, ledger)?;
            let rhs = evaluate(right, context, policy, ledger)?;
            lhs.modulo_with_policy(&rhs, policy, ledger)
                .map_err(ExprError::from)
        }
        Expr::FloorDiv { left, right } => {
            let lhs = evaluate(left, context, policy, ledger)?;
            let rhs = evaluate(right, context, policy, ledger)?;
            lhs.floordiv_with_policy(&rhs, policy, ledger)
                .map_err(ExprError::from)
        }
        Expr::Pow { left, right } => {
            let lhs = evaluate(left, context, policy, ledger)?;
            let rhs = evaluate(right, context, policy, ledger)?;
            lhs.pow_with_policy(&rhs, policy, ledger)
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
        Expr::Abs { expr } => {
            let input = evaluate(expr, context, policy, ledger)?;
            input.abs().map_err(ExprError::from)
        }
        Expr::Round { expr, decimals } => {
            let input = evaluate(expr, context, policy, ledger)?;
            input.round(*decimals).map_err(ExprError::from)
        }
        Expr::IsNull { expr, negated } => {
            let input = evaluate(expr, context, policy, ledger)?;
            if *negated {
                input.notna().map_err(ExprError::from)
            } else {
                input.isna().map_err(ExprError::from)
            }
        }
        Expr::Between {
            expr,
            left,
            right,
            inclusive,
        } => {
            let input = evaluate(expr, context, policy, ledger)?;
            input
                .between(left, right, inclusive.as_str())
                .map_err(ExprError::from)
        }
        Expr::Clip { expr, lower, upper } => {
            let input = evaluate(expr, context, policy, ledger)?;
            input
                .clip(Some(*lower), Some(*upper))
                .map_err(ExprError::from)
        }
        Expr::Compare { left, right, op } => {
            evaluate_comparison(left, right, *op, context, policy, ledger)
        }
        Expr::IsIn {
            left,
            values,
            negated,
        } => {
            let out = evaluate(left, context, policy, ledger)?
                .isin(values)
                .map_err(ExprError::from)?;
            if *negated {
                out.not().map_err(ExprError::from)
            } else {
                Ok(out)
            }
        }
        Expr::Literal { value } => {
            let index = context
                .anchor_index
                .as_ref()
                .ok_or(ExprError::UnanchoredLiteral)?;
            Series::broadcast("_literal", value.clone(), index.labels().to_vec())
                .map_err(ExprError::from)
        }
    }
}

pub fn evaluate_on_dataframe(
    expr: &Expr,
    frame: &fp_frame::DataFrame,
    policy: &RuntimePolicy,
    ledger: &mut EvidenceLedger,
) -> Result<Series, ExprError> {
    let context = EvalContext::from_dataframe(frame)?;
    evaluate(expr, &context, policy, ledger)
}

pub fn evaluate_on_dataframe_with_locals(
    expr: &Expr,
    frame: &fp_frame::DataFrame,
    locals: &BTreeMap<String, Scalar>,
    policy: &RuntimePolicy,
    ledger: &mut EvidenceLedger,
) -> Result<Series, ExprError> {
    let context = EvalContext::from_dataframe_with_locals(frame, locals)?;
    evaluate(expr, &context, policy, ledger)
}

fn is_pure_boolean_literal_filter(expr: &Expr) -> bool {
    match expr {
        Expr::Literal {
            value: Scalar::Bool(_),
        } => true,
        Expr::And { left, right } | Expr::Or { left, right } => {
            is_pure_boolean_literal_filter(left) && is_pure_boolean_literal_filter(right)
        }
        Expr::Not { expr } => is_pure_boolean_literal_filter(expr),
        _ => false,
    }
}

pub fn filter_dataframe_on_expr(
    expr: &Expr,
    frame: &fp_frame::DataFrame,
    policy: &RuntimePolicy,
    ledger: &mut EvidenceLedger,
) -> Result<fp_frame::DataFrame, ExprError> {
    if is_pure_boolean_literal_filter(expr) {
        return Err(ExprError::Frame(FrameError::CompatibilityRejected(
            "scalar boolean query expressions are not valid row filters".to_string(),
        )));
    }
    let mask = evaluate_on_dataframe(expr, frame, policy, ledger)?;
    if let Some(offending) = mask
        .values()
        .iter()
        .find(|value| !matches!(value, Scalar::Bool(_) | Scalar::Null(_)))
    {
        return Err(ExprError::Frame(FrameError::CompatibilityRejected(
            format!(
                "boolean mask required for query-style filter; found dtype {:?}",
                offending.dtype()
            ),
        )));
    }
    frame.filter_rows(&mask).map_err(ExprError::from)
}

pub fn filter_dataframe_on_expr_with_locals(
    expr: &Expr,
    frame: &fp_frame::DataFrame,
    locals: &BTreeMap<String, Scalar>,
    policy: &RuntimePolicy,
    ledger: &mut EvidenceLedger,
) -> Result<fp_frame::DataFrame, ExprError> {
    if is_pure_boolean_literal_filter(expr) {
        return Err(ExprError::Frame(FrameError::CompatibilityRejected(
            "scalar boolean query expressions are not valid row filters".to_string(),
        )));
    }
    let mask = evaluate_on_dataframe_with_locals(expr, frame, locals, policy, ledger)?;
    if let Some(offending) = mask
        .values()
        .iter()
        .find(|value| !matches!(value, Scalar::Bool(_) | Scalar::Null(_)))
    {
        return Err(ExprError::Frame(FrameError::CompatibilityRejected(
            format!(
                "boolean mask required for query-style filter; found dtype {:?}",
                offending.dtype()
            ),
        )));
    }
    frame.filter_rows(&mask).map_err(ExprError::from)
}

/// Evaluate a string expression against a DataFrame and return a Series.
///
/// Analogous to `pandas.DataFrame.eval(expr_str)`. Parses the string
/// expression and evaluates it using the DataFrame's columns as variables.
pub fn eval_str(
    expr_str: &str,
    frame: &fp_frame::DataFrame,
    policy: &RuntimePolicy,
    ledger: &mut EvidenceLedger,
) -> Result<Series, ExprError> {
    eval_str_with_locals(expr_str, frame, &BTreeMap::new(), policy, ledger)
}

pub fn eval_str_with_locals(
    expr_str: &str,
    frame: &fp_frame::DataFrame,
    locals: &BTreeMap<String, Scalar>,
    policy: &RuntimePolicy,
    ledger: &mut EvidenceLedger,
) -> Result<Series, ExprError> {
    let expr = parse_expr(expr_str)?;
    evaluate_on_dataframe_with_locals(&expr, frame, locals, policy, ledger)
}

/// Filter a DataFrame using a string expression.
///
/// Analogous to `pandas.DataFrame.query(expr_str)`. Parses the string
/// expression, evaluates it to a boolean mask, then filters the DataFrame.
pub fn query_str(
    expr_str: &str,
    frame: &fp_frame::DataFrame,
    policy: &RuntimePolicy,
    ledger: &mut EvidenceLedger,
) -> Result<fp_frame::DataFrame, ExprError> {
    query_str_with_locals(expr_str, frame, &BTreeMap::new(), policy, ledger)
}

pub fn query_str_with_locals(
    expr_str: &str,
    frame: &fp_frame::DataFrame,
    locals: &BTreeMap<String, Scalar>,
    policy: &RuntimePolicy,
    ledger: &mut EvidenceLedger,
) -> Result<fp_frame::DataFrame, ExprError> {
    let expr = parse_expr(expr_str)?;
    filter_dataframe_on_expr_with_locals(&expr, frame, locals, policy, ledger)
}

// ── Extension trait for DataFrame eval/query convenience ─────────────

/// Extension trait that adds `eval()` and `query()` methods directly to DataFrame.
///
/// Import this trait to call `df.eval("a + b")` and `df.query("a > 1")` directly.
/// Uses a default hardened policy and fresh evidence ledger.
pub trait DataFrameExprExt {
    /// Evaluate an expression string in the context of this DataFrame.
    ///
    /// Matches `pd.DataFrame.eval(expr)`.
    fn eval(&self, expr_str: &str) -> Result<Series, ExprError>;

    /// Matches `pd.DataFrame.eval(expr)` with explicit `@local` scalar bindings.
    fn eval_with_locals(
        &self,
        expr_str: &str,
        locals: &BTreeMap<String, Scalar>,
    ) -> Result<Series, ExprError>;

    /// Filter rows by a boolean expression string.
    ///
    /// Matches `pd.DataFrame.query(expr)`.
    fn query(&self, expr_str: &str) -> Result<fp_frame::DataFrame, ExprError>;

    /// Matches `pd.DataFrame.query(expr)` with explicit `@local` scalar bindings.
    fn query_with_locals(
        &self,
        expr_str: &str,
        locals: &BTreeMap<String, Scalar>,
    ) -> Result<fp_frame::DataFrame, ExprError>;
}

impl DataFrameExprExt for fp_frame::DataFrame {
    fn eval(&self, expr_str: &str) -> Result<Series, ExprError> {
        self.eval_with_locals(expr_str, &BTreeMap::new())
    }

    fn eval_with_locals(
        &self,
        expr_str: &str,
        locals: &BTreeMap<String, Scalar>,
    ) -> Result<Series, ExprError> {
        let policy = RuntimePolicy::hardened(Some(100_000));
        let mut ledger = EvidenceLedger::new();
        eval_str_with_locals(expr_str, self, locals, &policy, &mut ledger)
    }

    fn query(&self, expr_str: &str) -> Result<fp_frame::DataFrame, ExprError> {
        self.query_with_locals(expr_str, &BTreeMap::new())
    }

    fn query_with_locals(
        &self,
        expr_str: &str,
        locals: &BTreeMap<String, Scalar>,
    ) -> Result<fp_frame::DataFrame, ExprError> {
        let policy = RuntimePolicy::hardened(Some(100_000));
        let mut ledger = EvidenceLedger::new();
        query_str_with_locals(expr_str, self, locals, &policy, &mut ledger)
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

    fn extract_series(expr: &Expr, series_set: &mut std::collections::BTreeSet<String>) {
        match expr {
            Expr::Series { name } => {
                series_set.insert(name.0.clone());
            }
            Expr::Local { .. } => {}
            Expr::Add { left, right }
            | Expr::Sub { left, right }
            | Expr::Mul { left, right }
            | Expr::Div { left, right }
            | Expr::Modulo { left, right }
            | Expr::FloorDiv { left, right }
            | Expr::Pow { left, right }
            | Expr::And { left, right }
            | Expr::Or { left, right }
            | Expr::Compare { left, right, .. } => {
                Self::extract_series(left, series_set);
                Self::extract_series(right, series_set);
            }
            Expr::IsIn { left, .. } => Self::extract_series(left, series_set),
            Expr::Not { expr } | Expr::Abs { expr } | Expr::Round { expr, .. } => {
                Self::extract_series(expr, series_set);
            }
            Expr::IsNull { expr, .. } => Self::extract_series(expr, series_set),
            Expr::Between { expr, .. } => Self::extract_series(expr, series_set),
            Expr::Clip { expr, .. } => Self::extract_series(expr, series_set),
            Expr::Literal { .. } => {}
        }
    }

    fn is_linear(expr: &Expr) -> bool {
        let mut series_set = std::collections::BTreeSet::new();
        Self::extract_series(expr, &mut series_set);
        series_set.len() == 1
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
        Expr::Local { name } => {
            let value = delta_ctx
                .get_local(name)
                .cloned()
                .ok_or_else(|| ExprError::UnknownLocal(name.clone()))?;
            Series::broadcast(name.as_str(), value, delta.new_labels.clone())
                .map_err(ExprError::from)
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
        Expr::Modulo { left, right } => {
            let lhs = evaluate_delta(left, delta_ctx, delta, policy, ledger)?;
            let rhs = evaluate_delta(right, delta_ctx, delta, policy, ledger)?;
            lhs.modulo_with_policy(&rhs, policy, ledger)
                .map_err(ExprError::from)
        }
        Expr::FloorDiv { left, right } => {
            let lhs = evaluate_delta(left, delta_ctx, delta, policy, ledger)?;
            let rhs = evaluate_delta(right, delta_ctx, delta, policy, ledger)?;
            lhs.floordiv_with_policy(&rhs, policy, ledger)
                .map_err(ExprError::from)
        }
        Expr::Pow { left, right } => {
            let lhs = evaluate_delta(left, delta_ctx, delta, policy, ledger)?;
            let rhs = evaluate_delta(right, delta_ctx, delta, policy, ledger)?;
            lhs.pow_with_policy(&rhs, policy, ledger)
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
        Expr::Abs { expr } => {
            let input = evaluate_delta(expr, delta_ctx, delta, policy, ledger)?;
            input.abs().map_err(ExprError::from)
        }
        Expr::Round { expr, decimals } => {
            let input = evaluate_delta(expr, delta_ctx, delta, policy, ledger)?;
            input.round(*decimals).map_err(ExprError::from)
        }
        Expr::IsNull { expr, negated } => {
            let input = evaluate_delta(expr, delta_ctx, delta, policy, ledger)?;
            if *negated {
                input.notna().map_err(ExprError::from)
            } else {
                input.isna().map_err(ExprError::from)
            }
        }
        Expr::Between {
            expr,
            left,
            right,
            inclusive,
        } => {
            let input = evaluate_delta(expr, delta_ctx, delta, policy, ledger)?;
            input
                .between(left, right, inclusive.as_str())
                .map_err(ExprError::from)
        }
        Expr::Clip { expr, lower, upper } => {
            let input = evaluate_delta(expr, delta_ctx, delta, policy, ledger)?;
            input
                .clip(Some(*lower), Some(*upper))
                .map_err(ExprError::from)
        }
        Expr::Compare { left, right, op } => {
            evaluate_delta_comparison(left, right, *op, delta_ctx, delta, policy, ledger)
        }
        Expr::IsIn {
            left,
            values,
            negated,
        } => {
            let out = evaluate_delta(left, delta_ctx, delta, policy, ledger)?
                .isin(values)
                .map_err(ExprError::from)?;
            if *negated {
                out.not().map_err(ExprError::from)
            } else {
                Ok(out)
            }
        }
        Expr::Literal { value } => {
            Series::broadcast("_literal", value.clone(), delta.new_labels.clone())
                .map_err(ExprError::from)
        }
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

// ── Expression Parser ───────────────────────────────────────────────────
//
// A simple recursive-descent parser for pandas-style query/eval expressions.
// Supports:
//   - Column references (identifiers)
//   - Numeric literals (integer and float)
//   - String literals ('...' or "...")
//   - Boolean literals (`True` / `False`)
//   - Comparison operators: ==, !=, >, >=, <, <=
//   - Membership operators: in, not in (with scalar list literals)
//   - Logical operators: and, or, not
//   - Unary function calls: abs(expr)
//   - Series method calls: .isin([...]), .between(left, right, inclusive=...),
//     .clip(lower, upper), .round(decimals), .isna(), .notna(), .isnull(),
//     .notnull()
//   - Arithmetic operators: +, -, *, /, //, %, **
//   - Parenthesized sub-expressions

/// Parse a string expression into an `Expr` AST.
///
/// Syntax:
///   expr       → or_expr
///   or_expr    → and_expr ( "or" and_expr )*
///   and_expr   → not_expr ( "and" not_expr )*
///   not_expr   → "not" not_expr | comparison
///   comparison → add_expr ( ("==" | "!=" | ">" | ">=" | "<" | "<=") add_expr | ("in" | "not" "in") list_literal )?
///   add_expr   → mul_expr ( ("+" | "-") mul_expr )*
///   mul_expr   → unary_expr ( ("*" | "/" | "//" | "%") unary_expr )*
///   unary_expr → ("+" | "-") unary_expr | pow_expr
///   pow_expr   → atom ( "**" unary_expr )?
///   atom       → primary ( "." METHOD_CALL )*
///   primary    → NUMBER | STRING | BOOL | IDENT | LOCAL | "abs" "(" expr ")" | "(" expr ")"
pub fn parse_expr(input: &str) -> Result<Expr, ExprError> {
    let tokens = tokenize(input)?;
    let mut pos = 0;
    let result = parse_or(&tokens, &mut pos)?;
    if pos < tokens.len() {
        return Err(ExprError::ParseError(format!(
            "unexpected token at position {pos}: {:?}",
            tokens[pos]
        )));
    }
    Ok(result)
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Ident(String),
    Local(String),
    Int(i64),
    Float(f64),
    Str(String),
    Bool(bool),
    // Comparison
    EqEq,
    NotEq,
    Gt,
    Ge,
    Lt,
    Le,
    Assign,
    // Arithmetic
    Plus,
    Minus,
    Star,
    Slash,
    FloorDiv,
    Percent,
    Pow,
    // Grouping
    LParen,
    RParen,
    LBracket,
    RBracket,
    Comma,
    Dot,
    // Logical (keywords)
    And,
    Or,
    Not,
    In,
}

fn tokenize(input: &str) -> Result<Vec<Token>, ExprError> {
    let chars: Vec<char> = input.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        match c {
            '+' => {
                tokens.push(Token::Plus);
                i += 1;
            }
            '-' => {
                // Check for negative number literal
                if i + 1 < chars.len()
                    && (chars[i + 1].is_ascii_digit() || chars[i + 1] == '.')
                    && (tokens.is_empty()
                        || matches!(
                            tokens.last(),
                            Some(
                                Token::LParen
                                    | Token::EqEq
                                    | Token::Assign
                                    | Token::NotEq
                                    | Token::Gt
                                    | Token::Ge
                                    | Token::Lt
                                    | Token::Le
                                    | Token::Plus
                                    | Token::Minus
                                    | Token::Star
                                    | Token::Slash
                                    | Token::FloorDiv
                                    | Token::Percent
                                    | Token::Pow
                                    | Token::And
                                    | Token::Or
                                    | Token::Not
                                    | Token::In
                                    | Token::LBracket
                                    | Token::Comma
                            )
                        ))
                {
                    let start = i;
                    let mut scan = i + 1;
                    let mut dot_count = 0;
                    while scan < chars.len() && (chars[scan].is_ascii_digit() || chars[scan] == '.')
                    {
                        if chars[scan] == '.' {
                            dot_count += 1;
                        }
                        scan += 1;
                    }
                    let mut after = scan;
                    while after < chars.len() && chars[after].is_whitespace() {
                        after += 1;
                    }
                    if after + 1 < chars.len() && chars[after] == '*' && chars[after + 1] == '*' {
                        tokens.push(Token::Minus);
                        i += 1;
                        continue;
                    }
                    i = scan;
                    let num_str: String = chars[start..i].iter().collect();
                    if dot_count > 1 {
                        return Err(ExprError::ParseError(format!(
                            "invalid number literal with multiple decimal points: {num_str}"
                        )));
                    }
                    if dot_count == 1 {
                        tokens.push(Token::Float(num_str.parse::<f64>().map_err(|_| {
                            ExprError::ParseError(format!("invalid float: {num_str}"))
                        })?));
                    } else {
                        tokens.push(Token::Int(num_str.parse::<i64>().map_err(|_| {
                            ExprError::ParseError(format!("invalid integer: {num_str}"))
                        })?));
                    }
                } else {
                    tokens.push(Token::Minus);
                    i += 1;
                }
            }
            '.' if i + 1 < chars.len() && chars[i + 1].is_ascii_digit() => {
                let start = i;
                i += 1;
                let mut extra_dots = 0;
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                    if chars[i] == '.' {
                        extra_dots += 1;
                    }
                    i += 1;
                }
                let num_str: String = chars[start..i].iter().collect();
                if extra_dots > 0 {
                    return Err(ExprError::ParseError(format!(
                        "invalid number literal with multiple decimal points: {num_str}"
                    )));
                }
                tokens.push(Token::Float(num_str.parse::<f64>().map_err(|_| {
                    ExprError::ParseError(format!("invalid float: {num_str}"))
                })?));
            }
            '.' => {
                tokens.push(Token::Dot);
                i += 1;
            }
            '*' => {
                if i + 1 < chars.len() && chars[i + 1] == '*' {
                    tokens.push(Token::Pow);
                    i += 2;
                } else {
                    tokens.push(Token::Star);
                    i += 1;
                }
            }
            '/' => {
                if i + 1 < chars.len() && chars[i + 1] == '/' {
                    tokens.push(Token::FloorDiv);
                    i += 2;
                } else {
                    tokens.push(Token::Slash);
                    i += 1;
                }
            }
            '%' => {
                tokens.push(Token::Percent);
                i += 1;
            }
            '(' => {
                tokens.push(Token::LParen);
                i += 1;
            }
            ')' => {
                tokens.push(Token::RParen);
                i += 1;
            }
            '[' => {
                tokens.push(Token::LBracket);
                i += 1;
            }
            ']' => {
                tokens.push(Token::RBracket);
                i += 1;
            }
            ',' => {
                tokens.push(Token::Comma);
                i += 1;
            }
            '=' => {
                if i + 1 < chars.len() && chars[i + 1] == '=' {
                    tokens.push(Token::EqEq);
                    i += 2;
                } else {
                    tokens.push(Token::Assign);
                    i += 1;
                }
            }
            '!' => {
                if i + 1 < chars.len() && chars[i + 1] == '=' {
                    tokens.push(Token::NotEq);
                    i += 2;
                } else {
                    return Err(ExprError::ParseError(
                        "expected '!=' but found single '!'".into(),
                    ));
                }
            }
            '>' => {
                if i + 1 < chars.len() && chars[i + 1] == '=' {
                    tokens.push(Token::Ge);
                    i += 2;
                } else {
                    tokens.push(Token::Gt);
                    i += 1;
                }
            }
            '<' => {
                if i + 1 < chars.len() && chars[i + 1] == '=' {
                    tokens.push(Token::Le);
                    i += 2;
                } else {
                    tokens.push(Token::Lt);
                    i += 1;
                }
            }
            '\'' | '"' => {
                let quote = c;
                i += 1;
                let mut s = String::new();
                while i < chars.len() && chars[i] != quote {
                    if chars[i] == '\\' && i + 1 < chars.len() {
                        i += 1;
                        match chars[i] {
                            'n' => s.push('\n'),
                            't' => s.push('\t'),
                            'r' => s.push('\r'),
                            '\\' => s.push('\\'),
                            c if c == quote => s.push(c),
                            other => {
                                s.push('\\');
                                s.push(other);
                            }
                        }
                    } else {
                        s.push(chars[i]);
                    }
                    i += 1;
                }
                if i >= chars.len() {
                    return Err(ExprError::ParseError("unterminated string literal".into()));
                }
                tokens.push(Token::Str(s));
                i += 1; // skip closing quote
            }
            '`' => {
                i += 1;
                let mut name = String::new();
                while i < chars.len() && chars[i] != '`' {
                    name.push(chars[i]);
                    i += 1;
                }
                if i >= chars.len() {
                    return Err(ExprError::ParseError(
                        "unterminated backtick identifier".into(),
                    ));
                }
                if name.is_empty() {
                    return Err(ExprError::ParseError("empty backtick identifier".into()));
                }
                tokens.push(Token::Ident(name));
                i += 1; // skip closing backtick
            }
            '@' => {
                i += 1;
                if i >= chars.len() || !(chars[i].is_alphabetic() || chars[i] == '_') {
                    return Err(ExprError::ParseError(
                        "expected identifier after '@'".into(),
                    ));
                }
                let start = i;
                while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                let word: String = chars[start..i].iter().collect();
                tokens.push(Token::Local(word));
            }
            '&' => {
                tokens.push(Token::And);
                i += 1;
            }
            '|' => {
                tokens.push(Token::Or);
                i += 1;
            }
            '~' => {
                tokens.push(Token::Not);
                i += 1;
            }
            _ if c.is_ascii_digit() => {
                let start = i;
                let mut dot_count = 0;
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                    if chars[i] == '.' {
                        dot_count += 1;
                    }
                    i += 1;
                }
                let num_str: String = chars[start..i].iter().collect();
                if dot_count > 1 {
                    return Err(ExprError::ParseError(format!(
                        "invalid number literal with multiple decimal points: {num_str}"
                    )));
                }
                if dot_count == 1 {
                    tokens.push(Token::Float(num_str.parse::<f64>().map_err(|_| {
                        ExprError::ParseError(format!("invalid float: {num_str}"))
                    })?));
                } else {
                    tokens.push(Token::Int(num_str.parse::<i64>().map_err(|_| {
                        ExprError::ParseError(format!("invalid integer: {num_str}"))
                    })?));
                }
            }
            _ if c.is_alphabetic() || c == '_' => {
                let start = i;
                while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                let word: String = chars[start..i].iter().collect();
                match word.as_str() {
                    "and" => tokens.push(Token::And),
                    "or" => tokens.push(Token::Or),
                    "not" => tokens.push(Token::Not),
                    "in" => tokens.push(Token::In),
                    "True" => tokens.push(Token::Bool(true)),
                    "False" => tokens.push(Token::Bool(false)),
                    _ => tokens.push(Token::Ident(word)),
                }
            }
            _ => {
                return Err(ExprError::ParseError(format!(
                    "unexpected character: '{c}'"
                )));
            }
        }
    }
    Ok(tokens)
}

fn parse_or(tokens: &[Token], pos: &mut usize) -> Result<Expr, ExprError> {
    let mut left = parse_and(tokens, pos)?;
    while *pos < tokens.len() && tokens[*pos] == Token::Or {
        *pos += 1;
        let right = parse_and(tokens, pos)?;
        left = Expr::Or {
            left: Box::new(left),
            right: Box::new(right),
        };
    }
    Ok(left)
}

fn parse_and(tokens: &[Token], pos: &mut usize) -> Result<Expr, ExprError> {
    let mut left = parse_not(tokens, pos)?;
    while *pos < tokens.len() && tokens[*pos] == Token::And {
        *pos += 1;
        let right = parse_not(tokens, pos)?;
        left = Expr::And {
            left: Box::new(left),
            right: Box::new(right),
        };
    }
    Ok(left)
}

fn parse_not(tokens: &[Token], pos: &mut usize) -> Result<Expr, ExprError> {
    if *pos < tokens.len() && tokens[*pos] == Token::Not {
        *pos += 1;
        let inner = parse_not(tokens, pos)?;
        return Ok(Expr::Not {
            expr: Box::new(inner),
        });
    }
    parse_comparison(tokens, pos)
}

fn parse_comparison(tokens: &[Token], pos: &mut usize) -> Result<Expr, ExprError> {
    let mut left = parse_add(tokens, pos)?;
    let mut chained = None;
    while *pos < tokens.len() {
        let membership = if tokens[*pos] == Token::In {
            *pos += 1;
            Some(false)
        } else if *pos + 1 < tokens.len()
            && tokens[*pos] == Token::Not
            && tokens[*pos + 1] == Token::In
        {
            *pos += 2;
            Some(true)
        } else {
            None
        };
        if let Some(negated) = membership {
            let comparison = Expr::IsIn {
                left: Box::new(left.clone()),
                values: parse_list_literal(tokens, pos)?,
                negated,
            };
            chained = Some(match chained {
                Some(previous) => Expr::And {
                    left: Box::new(previous),
                    right: Box::new(comparison),
                },
                None => comparison,
            });
            break;
        }

        let op = match &tokens[*pos] {
            Token::EqEq => Some(ComparisonOp::Eq),
            Token::NotEq => Some(ComparisonOp::Ne),
            Token::Gt => Some(ComparisonOp::Gt),
            Token::Ge => Some(ComparisonOp::Ge),
            Token::Lt => Some(ComparisonOp::Lt),
            Token::Le => Some(ComparisonOp::Le),
            _ => None,
        };
        if let Some(op) = op {
            *pos += 1;
            let right = parse_add(tokens, pos)?;
            let comparison = Expr::Compare {
                left: Box::new(left.clone()),
                right: Box::new(right.clone()),
                op,
            };
            chained = Some(match chained {
                Some(previous) => Expr::And {
                    left: Box::new(previous),
                    right: Box::new(comparison),
                },
                None => comparison,
            });
            left = right;
        } else {
            break;
        }
    }
    Ok(chained.unwrap_or(left))
}

fn parse_list_literal(tokens: &[Token], pos: &mut usize) -> Result<Vec<Scalar>, ExprError> {
    if *pos >= tokens.len() || tokens[*pos] != Token::LBracket {
        return Err(ExprError::ParseError(
            "expected list literal after membership operator".into(),
        ));
    }
    *pos += 1;

    let mut values = Vec::new();
    if *pos < tokens.len() && tokens[*pos] == Token::RBracket {
        *pos += 1;
        return Ok(values);
    }

    loop {
        if *pos >= tokens.len() {
            return Err(ExprError::ParseError("unterminated list literal".into()));
        }

        let value = match &tokens[*pos] {
            Token::Int(value) => Scalar::Int64(*value),
            Token::Float(value) => Scalar::Float64(*value),
            Token::Str(value) => Scalar::Utf8(value.clone()),
            Token::Bool(value) => Scalar::Bool(*value),
            other => {
                return Err(ExprError::ParseError(format!(
                    "membership list values must be scalar literals, got {other:?}"
                )));
            }
        };
        values.push(value);
        *pos += 1;

        if *pos >= tokens.len() {
            return Err(ExprError::ParseError("unterminated list literal".into()));
        }
        match tokens[*pos] {
            Token::Comma => {
                *pos += 1;
                if *pos < tokens.len() && tokens[*pos] == Token::RBracket {
                    *pos += 1;
                    return Ok(values);
                }
            }
            Token::RBracket => {
                *pos += 1;
                return Ok(values);
            }
            ref other => {
                return Err(ExprError::ParseError(format!(
                    "expected ',' or ']' in list literal, got {other:?}"
                )));
            }
        }
    }
}

fn parse_scalar_literal(tokens: &[Token], pos: &mut usize) -> Result<Scalar, ExprError> {
    if *pos >= tokens.len() {
        return Err(ExprError::ParseError("expected scalar literal".into()));
    }
    let scalar = match &tokens[*pos] {
        Token::Int(value) => Scalar::Int64(*value),
        Token::Float(value) => Scalar::Float64(*value),
        Token::Str(value) => Scalar::Utf8(value.clone()),
        Token::Bool(value) => Scalar::Bool(*value),
        Token::Minus => match tokens.get(*pos + 1) {
            Some(Token::Int(value)) => {
                *pos += 1;
                Scalar::Int64(value.saturating_neg())
            }
            Some(Token::Float(value)) => {
                *pos += 1;
                Scalar::Float64(-value)
            }
            other => {
                return Err(ExprError::ParseError(format!(
                    "expected numeric literal after '-', got {other:?}"
                )));
            }
        },
        other => {
            return Err(ExprError::ParseError(format!(
                "expected scalar literal, got {other:?}"
            )));
        }
    };
    *pos += 1;
    Ok(scalar)
}

fn parse_numeric_literal(tokens: &[Token], pos: &mut usize) -> Result<f64, ExprError> {
    match parse_scalar_literal(tokens, pos)? {
        Scalar::Int64(value) => Ok(value as f64),
        Scalar::Float64(value) => Ok(value),
        other => Err(ExprError::ParseError(format!(
            "expected numeric literal, got {other:?}"
        ))),
    }
}

fn parse_i32_literal(tokens: &[Token], pos: &mut usize, context: &str) -> Result<i32, ExprError> {
    let value = parse_scalar_literal(tokens, pos)?;
    let Scalar::Int64(value) = value else {
        return Err(ExprError::ParseError(format!(
            "{context} must be an integer literal, got {value:?}"
        )));
    };
    i32::try_from(value)
        .map_err(|_| ExprError::ParseError(format!("{context} is outside the i32 range: {value}")))
}

fn parse_add(tokens: &[Token], pos: &mut usize) -> Result<Expr, ExprError> {
    let mut left = parse_mul(tokens, pos)?;
    while *pos < tokens.len() {
        match &tokens[*pos] {
            Token::Plus => {
                *pos += 1;
                let right = parse_mul(tokens, pos)?;
                left = Expr::Add {
                    left: Box::new(left),
                    right: Box::new(right),
                };
            }
            Token::Minus => {
                *pos += 1;
                let right = parse_mul(tokens, pos)?;
                left = Expr::Sub {
                    left: Box::new(left),
                    right: Box::new(right),
                };
            }
            _ => break,
        }
    }
    Ok(left)
}

fn parse_mul(tokens: &[Token], pos: &mut usize) -> Result<Expr, ExprError> {
    let mut left = parse_unary(tokens, pos)?;
    while *pos < tokens.len() {
        match &tokens[*pos] {
            Token::Star => {
                *pos += 1;
                let right = parse_unary(tokens, pos)?;
                left = Expr::Mul {
                    left: Box::new(left),
                    right: Box::new(right),
                };
            }
            Token::Slash => {
                *pos += 1;
                let right = parse_unary(tokens, pos)?;
                left = Expr::Div {
                    left: Box::new(left),
                    right: Box::new(right),
                };
            }
            Token::FloorDiv => {
                *pos += 1;
                let right = parse_unary(tokens, pos)?;
                left = Expr::FloorDiv {
                    left: Box::new(left),
                    right: Box::new(right),
                };
            }
            Token::Percent => {
                *pos += 1;
                let right = parse_unary(tokens, pos)?;
                left = Expr::Modulo {
                    left: Box::new(left),
                    right: Box::new(right),
                };
            }
            _ => break,
        }
    }
    Ok(left)
}

fn parse_unary(tokens: &[Token], pos: &mut usize) -> Result<Expr, ExprError> {
    if *pos < tokens.len() {
        match tokens[*pos] {
            Token::Plus => {
                *pos += 1;
                return parse_unary(tokens, pos);
            }
            Token::Minus => {
                *pos += 1;
                let inner = parse_unary(tokens, pos)?;
                return Ok(Expr::Sub {
                    left: Box::new(Expr::Literal {
                        value: Scalar::Int64(0),
                    }),
                    right: Box::new(inner),
                });
            }
            _ => {}
        }
    }
    parse_pow(tokens, pos)
}

fn parse_pow(tokens: &[Token], pos: &mut usize) -> Result<Expr, ExprError> {
    let left = parse_atom(tokens, pos)?;
    if *pos < tokens.len() && tokens[*pos] == Token::Pow {
        *pos += 1;
        let right = parse_unary(tokens, pos)?;
        Ok(Expr::Pow {
            left: Box::new(left),
            right: Box::new(right),
        })
    } else {
        Ok(left)
    }
}

fn parse_atom(tokens: &[Token], pos: &mut usize) -> Result<Expr, ExprError> {
    if *pos >= tokens.len() {
        return Err(ExprError::ParseError("unexpected end of expression".into()));
    }
    let expr = match &tokens[*pos] {
        Token::Int(n) => {
            let val = *n;
            *pos += 1;
            Ok(Expr::Literal {
                value: Scalar::Int64(val),
            })
        }
        Token::Float(f) => {
            let val = *f;
            *pos += 1;
            Ok(Expr::Literal {
                value: Scalar::Float64(val),
            })
        }
        Token::Str(s) => {
            let val = s.clone();
            *pos += 1;
            Ok(Expr::Literal {
                value: Scalar::Utf8(val),
            })
        }
        Token::Bool(value) => {
            let val = *value;
            *pos += 1;
            Ok(Expr::Literal {
                value: Scalar::Bool(val),
            })
        }
        Token::Ident(name)
            if name == "abs" && *pos + 1 < tokens.len() && tokens[*pos + 1] == Token::LParen =>
        {
            *pos += 2; // skip function name and opening '('
            let inner = parse_or(tokens, pos)?;
            if *pos >= tokens.len() || tokens[*pos] != Token::RParen {
                return Err(ExprError::ParseError(
                    "expected closing ')' after abs argument".into(),
                ));
            }
            *pos += 1; // skip ')'
            Ok(Expr::Abs {
                expr: Box::new(inner),
            })
        }
        Token::Ident(name) => {
            let name = name.clone();
            *pos += 1;
            Ok(Expr::Series {
                name: SeriesRef(name),
            })
        }
        Token::Local(name) => {
            let name = name.clone();
            *pos += 1;
            Ok(Expr::Local { name })
        }
        Token::LParen => {
            *pos += 1; // skip '('
            let inner = parse_or(tokens, pos)?;
            if *pos >= tokens.len() || tokens[*pos] != Token::RParen {
                return Err(ExprError::ParseError("expected closing ')'".into()));
            }
            *pos += 1; // skip ')'
            Ok(inner)
        }
        other => Err(ExprError::ParseError(format!(
            "unexpected token: {other:?}"
        ))),
    }?;
    parse_postfix(expr, tokens, pos)
}

fn parse_postfix(mut expr: Expr, tokens: &[Token], pos: &mut usize) -> Result<Expr, ExprError> {
    while *pos < tokens.len() && tokens[*pos] == Token::Dot {
        let Some(Token::Ident(method)) = tokens.get(*pos + 1) else {
            return Err(ExprError::ParseError(
                "expected method name after '.'".into(),
            ));
        };
        if tokens.get(*pos + 2) != Some(&Token::LParen) {
            return Err(ExprError::ParseError(format!(
                "expected '(' after method name {method}"
            )));
        }

        match method.as_str() {
            "isna" | "isnull" | "notna" | "notnull" => {
                if tokens.get(*pos + 3) != Some(&Token::RParen) {
                    return Err(ExprError::ParseError(format!(
                        "method {method} does not accept arguments in expressions"
                    )));
                }
                expr = Expr::IsNull {
                    expr: Box::new(expr),
                    negated: matches!(method.as_str(), "notna" | "notnull"),
                };
                *pos += 4;
            }
            "isin" => {
                let mut arg_pos = *pos + 3;
                let values = parse_list_literal(tokens, &mut arg_pos)?;
                if tokens.get(arg_pos) != Some(&Token::RParen) {
                    return Err(ExprError::ParseError(
                        "expected ')' after isin list literal".into(),
                    ));
                }
                expr = Expr::IsIn {
                    left: Box::new(expr),
                    values,
                    negated: false,
                };
                *pos = arg_pos + 1;
            }
            "between" => {
                let mut arg_pos = *pos + 3;
                let left = parse_scalar_literal(tokens, &mut arg_pos)?;
                if tokens.get(arg_pos) != Some(&Token::Comma) {
                    return Err(ExprError::ParseError(
                        "expected ',' between between() bounds".into(),
                    ));
                }
                arg_pos += 1;
                let right = parse_scalar_literal(tokens, &mut arg_pos)?;
                let mut inclusive = BetweenInclusive::Both;
                if tokens.get(arg_pos) == Some(&Token::Comma) {
                    arg_pos += 1;
                    if let Some(Token::Ident(keyword)) = tokens.get(arg_pos)
                        && tokens.get(arg_pos + 1) == Some(&Token::Assign)
                    {
                        if keyword != "inclusive" {
                            return Err(ExprError::ParseError(format!(
                                "unexpected between() keyword argument: {keyword}"
                            )));
                        }
                        arg_pos += 2;
                    }
                    let inclusive_arg = parse_scalar_literal(tokens, &mut arg_pos)?;
                    let Scalar::Utf8(value) = inclusive_arg else {
                        return Err(ExprError::ParseError(format!(
                            "between() inclusive must be a string literal, got {inclusive_arg:?}"
                        )));
                    };
                    inclusive = BetweenInclusive::parse(&value)?;
                }
                if tokens.get(arg_pos) != Some(&Token::RParen) {
                    return Err(ExprError::ParseError(
                        "expected ')' after between() arguments".into(),
                    ));
                }
                expr = Expr::Between {
                    expr: Box::new(expr),
                    left,
                    right,
                    inclusive,
                };
                *pos = arg_pos + 1;
            }
            "clip" => {
                let mut arg_pos = *pos + 3;
                let lower = parse_numeric_literal(tokens, &mut arg_pos)?;
                if tokens.get(arg_pos) != Some(&Token::Comma) {
                    return Err(ExprError::ParseError(
                        "expected ',' between clip() bounds".into(),
                    ));
                }
                arg_pos += 1;
                let upper = parse_numeric_literal(tokens, &mut arg_pos)?;
                if tokens.get(arg_pos) != Some(&Token::RParen) {
                    return Err(ExprError::ParseError(
                        "expected ')' after clip() bounds".into(),
                    ));
                }
                expr = Expr::Clip {
                    expr: Box::new(expr),
                    lower,
                    upper,
                };
                *pos = arg_pos + 1;
            }
            "round" => {
                let mut arg_pos = *pos + 3;
                let decimals = if tokens.get(arg_pos) == Some(&Token::RParen) {
                    0
                } else {
                    if let Some(Token::Ident(keyword)) = tokens.get(arg_pos)
                        && tokens.get(arg_pos + 1) == Some(&Token::Assign)
                    {
                        if keyword != "decimals" {
                            return Err(ExprError::ParseError(format!(
                                "unexpected round() keyword argument: {keyword}"
                            )));
                        }
                        arg_pos += 2;
                    }
                    parse_i32_literal(tokens, &mut arg_pos, "round() decimals")?
                };
                if tokens.get(arg_pos) != Some(&Token::RParen) {
                    return Err(ExprError::ParseError(
                        "expected ')' after round() arguments".into(),
                    ));
                }
                expr = Expr::Round {
                    expr: Box::new(expr),
                    decimals,
                };
                *pos = arg_pos + 1;
            }
            _ => {
                return Err(ExprError::ParseError(format!(
                    "unsupported expression method: {method}"
                )));
            }
        };
    }
    Ok(expr)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use fp_columnar::ComparisonOp;
    use fp_frame::{FrameError, Series};
    use fp_runtime::{EvidenceLedger, RuntimePolicy};
    use fp_types::Scalar;

    use super::{
        BetweenInclusive, Delta, EvalContext, Expr, ExprError, MaterializedView, SeriesRef,
        evaluate,
    };

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
    fn expression_compare_supports_scalar_scalar_with_anchor() {
        let anchor = Series::from_values(
            "a",
            vec![1_i64.into(), 2_i64.into()],
            vec![Scalar::Int64(10), Scalar::Int64(20)],
        )
        .expect("anchor");

        let mut ctx = EvalContext::new();
        ctx.insert_series(anchor);
        let policy = RuntimePolicy::hardened(Some(10_000));

        let mut ledger = EvidenceLedger::new();
        let out = evaluate(
            &Expr::Compare {
                left: Box::new(Expr::Literal {
                    value: Scalar::Int64(1),
                }),
                right: Box::new(Expr::Literal {
                    value: Scalar::Int64(0),
                }),
                op: ComparisonOp::Gt,
            },
            &ctx,
            &policy,
            &mut ledger,
        )
        .expect("scalar > scalar");
        assert_eq!(out.values(), &[Scalar::Bool(true), Scalar::Bool(true)]);
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

    #[test]
    fn eval_context_from_dataframe_builds_series_bindings() {
        let frame = fp_frame::DataFrame::from_dict(
            &["a", "b"],
            vec![
                ("a", vec![Scalar::Int64(1), Scalar::Int64(2)]),
                ("b", vec![Scalar::Int64(10), Scalar::Int64(20)]),
            ],
        )
        .expect("frame");

        let context = EvalContext::from_dataframe(&frame).expect("context");
        let a = context.get_series("a").expect("a series present");
        let b = context.get_series("b").expect("b series present");

        assert_eq!(a.values(), &[Scalar::Int64(1), Scalar::Int64(2)]);
        assert_eq!(b.values(), &[Scalar::Int64(10), Scalar::Int64(20)]);
        assert_eq!(a.index().labels(), frame.index().labels());
        assert_eq!(b.index().labels(), frame.index().labels());
    }

    #[test]
    fn evaluate_on_dataframe_matches_manual_context_eval() {
        let frame = fp_frame::DataFrame::from_dict(
            &["a", "b"],
            vec![
                ("a", vec![Scalar::Int64(3), Scalar::Int64(4)]),
                ("b", vec![Scalar::Int64(30), Scalar::Int64(40)]),
            ],
        )
        .expect("frame");

        let expr = Expr::Add {
            left: Box::new(Expr::Series {
                name: SeriesRef("a".to_owned()),
            }),
            right: Box::new(Expr::Series {
                name: SeriesRef("b".to_owned()),
            }),
        };
        let policy = RuntimePolicy::hardened(Some(10_000));
        let mut ledger = EvidenceLedger::new();
        let via_frame =
            super::evaluate_on_dataframe(&expr, &frame, &policy, &mut ledger).expect("frame eval");

        let mut manual = EvalContext::new();
        manual.insert_series(
            Series::new(
                "a",
                frame.index().clone(),
                frame.column("a").expect("a column").clone(),
            )
            .expect("a series"),
        );
        manual.insert_series(
            Series::new(
                "b",
                frame.index().clone(),
                frame.column("b").expect("b column").clone(),
            )
            .expect("b series"),
        );
        let mut ledger = EvidenceLedger::new();
        let manual_out = evaluate(&expr, &manual, &policy, &mut ledger).expect("manual eval");

        assert_eq!(via_frame.values(), manual_out.values());
        assert_eq!(via_frame.index().labels(), manual_out.index().labels());
    }

    #[test]
    fn filter_dataframe_on_expr_applies_boolean_mask() {
        let frame = fp_frame::DataFrame::from_dict(
            &["a", "b"],
            vec![
                (
                    "a",
                    vec![
                        Scalar::Int64(1),
                        Scalar::Int64(2),
                        Scalar::Int64(3),
                        Scalar::Int64(4),
                    ],
                ),
                (
                    "b",
                    vec![
                        Scalar::Int64(10),
                        Scalar::Int64(20),
                        Scalar::Int64(30),
                        Scalar::Int64(40),
                    ],
                ),
            ],
        )
        .expect("frame");

        let expr = Expr::Compare {
            left: Box::new(Expr::Series {
                name: SeriesRef("a".to_owned()),
            }),
            right: Box::new(Expr::Literal {
                value: Scalar::Int64(2),
            }),
            op: ComparisonOp::Gt,
        };
        let policy = RuntimePolicy::hardened(Some(10_000));
        let mut ledger = EvidenceLedger::new();
        let filtered = super::filter_dataframe_on_expr(&expr, &frame, &policy, &mut ledger)
            .expect("filter via expr");

        assert_eq!(filtered.len(), 2);
        assert_eq!(
            filtered.column("a").expect("a").values(),
            &[Scalar::Int64(3), Scalar::Int64(4)]
        );
        assert_eq!(
            filtered.column("b").expect("b").values(),
            &[Scalar::Int64(30), Scalar::Int64(40)]
        );
    }

    #[test]
    fn filter_dataframe_on_expr_rejects_non_boolean_mask() {
        let frame = fp_frame::DataFrame::from_dict(
            &["a"],
            vec![("a", vec![Scalar::Int64(1), Scalar::Int64(2)])],
        )
        .expect("frame");

        let expr = Expr::Series {
            name: SeriesRef("a".to_owned()),
        };
        let policy = RuntimePolicy::hardened(Some(10_000));
        let mut ledger = EvidenceLedger::new();
        let err = super::filter_dataframe_on_expr(&expr, &frame, &policy, &mut ledger).unwrap_err();

        assert!(matches!(
            err,
            ExprError::Frame(FrameError::CompatibilityRejected(msg))
                if msg.contains("boolean mask required for query-style filter")
        ));
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
        assert!(!MaterializedView::is_linear(&Expr::Add {
            left: Box::new(Expr::Series {
                name: SeriesRef("a".into()),
            }),
            right: Box::new(Expr::Series {
                name: SeriesRef("b".into()),
            }),
        }));
        assert!(!MaterializedView::is_linear(&Expr::Sub {
            left: Box::new(Expr::Series {
                name: SeriesRef("a".into()),
            }),
            right: Box::new(Expr::Series {
                name: SeriesRef("b".into()),
            }),
        }));
        assert!(!MaterializedView::is_linear(&Expr::Mul {
            left: Box::new(Expr::Series {
                name: SeriesRef("a".into()),
            }),
            right: Box::new(Expr::Series {
                name: SeriesRef("b".into()),
            }),
        }));
        assert!(!MaterializedView::is_linear(&Expr::Div {
            left: Box::new(Expr::Series {
                name: SeriesRef("a".into()),
            }),
            right: Box::new(Expr::Series {
                name: SeriesRef("b".into()),
            }),
        }));
        assert!(!MaterializedView::is_linear(&Expr::Modulo {
            left: Box::new(Expr::Series {
                name: SeriesRef("a".into()),
            }),
            right: Box::new(Expr::Series {
                name: SeriesRef("b".into()),
            }),
        }));
        assert!(!MaterializedView::is_linear(&Expr::FloorDiv {
            left: Box::new(Expr::Series {
                name: SeriesRef("a".into()),
            }),
            right: Box::new(Expr::Series {
                name: SeriesRef("b".into()),
            }),
        }));
        assert!(!MaterializedView::is_linear(&Expr::Pow {
            left: Box::new(Expr::Series {
                name: SeriesRef("a".into()),
            }),
            right: Box::new(Expr::Series {
                name: SeriesRef("b".into()),
            }),
        }));
        assert!(!MaterializedView::is_linear(&Expr::And {
            left: Box::new(Expr::Series {
                name: SeriesRef("a".into()),
            }),
            right: Box::new(Expr::Series {
                name: SeriesRef("b".into()),
            }),
        }));
        assert!(!MaterializedView::is_linear(&Expr::Or {
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
        assert!(!MaterializedView::is_linear(&Expr::Compare {
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

    // ── Parser tests ──

    #[test]
    fn parse_simple_comparison() {
        let expr = super::parse_expr("col_a > 5").unwrap();
        assert_eq!(
            expr,
            Expr::Compare {
                left: Box::new(Expr::Series {
                    name: SeriesRef("col_a".into()),
                }),
                right: Box::new(Expr::Literal {
                    value: Scalar::Int64(5),
                }),
                op: ComparisonOp::Gt,
            }
        );
    }

    #[test]
    fn parse_chained_comparison_lowers_to_pairwise_and() {
        let expr = super::parse_expr("a < b <= c").unwrap();
        let Expr::And { left, right } = expr else {
            panic!("expected And, got {expr:?}");
        };

        assert!(matches!(
            left.as_ref(),
            Expr::Compare {
                op: ComparisonOp::Lt,
                ..
            }
        ));
        assert!(matches!(
            right.as_ref(),
            Expr::Compare {
                op: ComparisonOp::Le,
                ..
            }
        ));

        if let Expr::Compare { left, right, .. } = left.as_ref() {
            assert_eq!(
                left.as_ref(),
                &Expr::Series {
                    name: SeriesRef("a".into())
                }
            );
            assert_eq!(
                right.as_ref(),
                &Expr::Series {
                    name: SeriesRef("b".into())
                }
            );
        }
        if let Expr::Compare { left, right, .. } = right.as_ref() {
            assert_eq!(
                left.as_ref(),
                &Expr::Series {
                    name: SeriesRef("b".into())
                }
            );
            assert_eq!(
                right.as_ref(),
                &Expr::Series {
                    name: SeriesRef("c".into())
                }
            );
        }
    }

    #[test]
    fn parse_and_or_expression() {
        let expr = super::parse_expr("a > 1 and b < 2").unwrap();
        assert!(
            matches!(&expr, Expr::And { .. }),
            "expected And, got {expr:?}"
        );
        if let Expr::And { left, right } = expr {
            assert!(matches!(*left, Expr::Compare { .. }));
            assert!(matches!(*right, Expr::Compare { .. }));
        }
    }

    #[test]
    fn parse_not_expression() {
        let expr = super::parse_expr("not a == 1").unwrap();
        assert!(matches!(expr, Expr::Not { .. }));
    }

    #[test]
    fn parse_arithmetic() {
        let expr = super::parse_expr("a + b * 2").unwrap();
        // Should parse as a + (b * 2) due to precedence
        assert!(
            matches!(&expr, Expr::Add { .. }),
            "expected Add, got {expr:?}"
        );
        if let Expr::Add { left, right } = expr {
            assert!(matches!(*left, Expr::Series { .. }));
            assert!(matches!(*right, Expr::Mul { .. }));
        }
    }

    #[test]
    fn parse_arithmetic_advanced() {
        let expr = super::parse_expr("a ** b // 2 % 3").unwrap();
        assert!(
            matches!(&expr, Expr::Modulo { .. }),
            "expected Modulo, got {expr:?}"
        );
        if let Expr::Modulo { left, .. } = expr {
            assert!(
                matches!(*left, Expr::FloorDiv { .. }),
                "expected FloorDiv, got {left:?}"
            );
            if let Expr::FloorDiv {
                left: inner_left, ..
            } = *left
            {
                assert!(
                    matches!(*inner_left, Expr::Pow { .. }),
                    "expected Pow, got {inner_left:?}"
                );
            }
        }
    }

    #[test]
    fn parse_unary_minus_identifier() {
        let expr = super::parse_expr("-a").unwrap();
        assert!(
            matches!(&expr, Expr::Sub { .. }),
            "expected Sub, got {expr:?}"
        );
    }

    #[test]
    fn parse_unary_plus_identifier() {
        let expr = super::parse_expr("+a").unwrap();
        assert!(
            matches!(&expr, Expr::Series { .. }),
            "expected Series, got {expr:?}"
        );
    }

    #[test]
    fn parse_parentheses() {
        let expr = super::parse_expr("(a + b) * 2").unwrap();
        assert!(
            matches!(&expr, Expr::Mul { .. }),
            "expected Mul, got {expr:?}"
        );
        if let Expr::Mul { left, right } = expr {
            assert!(matches!(*left, Expr::Add { .. }));
            assert!(matches!(
                *right,
                Expr::Literal {
                    value: Scalar::Int64(2)
                }
            ));
        }
    }

    #[test]
    fn parse_string_literal() {
        let expr = super::parse_expr("name == 'alice'").unwrap();
        assert!(
            matches!(&expr, Expr::Compare { .. }),
            "expected Compare, got {expr:?}"
        );
        if let Expr::Compare { right, op, .. } = expr {
            assert_eq!(op, ComparisonOp::Eq);
            assert_eq!(
                *right,
                Expr::Literal {
                    value: Scalar::Utf8("alice".into())
                }
            );
        }
    }

    #[test]
    fn parse_pandas_boolean_literals() {
        let true_expr = super::parse_expr("flag == True").unwrap();
        let true_right = match &true_expr {
            Expr::Compare { right, .. } => Some(right.as_ref()),
            _ => None,
        };
        assert_eq!(
            true_right,
            Some(&Expr::Literal {
                value: Scalar::Bool(true)
            })
        );

        let false_expr = super::parse_expr("flag != False").unwrap();
        let false_right = match &false_expr {
            Expr::Compare { right, .. } => Some(right.as_ref()),
            _ => None,
        };
        assert_eq!(
            false_right,
            Some(&Expr::Literal {
                value: Scalar::Bool(false)
            })
        );
    }

    #[test]
    fn parse_float_literal() {
        let expr = super::parse_expr("x > 4.56").unwrap();
        assert!(
            matches!(&expr, Expr::Compare { .. }),
            "expected Compare, got {expr:?}"
        );
        if let Expr::Compare { right, .. } = expr {
            assert_eq!(
                *right,
                Expr::Literal {
                    value: Scalar::Float64(4.56)
                }
            );
        }
    }

    #[test]
    fn parse_negative_literal() {
        let expr = super::parse_expr("x > -5").unwrap();
        assert!(
            matches!(&expr, Expr::Compare { .. }),
            "expected Compare, got {expr:?}"
        );
        if let Expr::Compare { right, .. } = expr {
            assert_eq!(
                *right,
                Expr::Literal {
                    value: Scalar::Int64(-5)
                }
            );
        }
    }

    #[test]
    fn parse_leading_dot_float_literal() {
        let expr = super::parse_expr("x > .5").unwrap();
        assert!(
            matches!(&expr, Expr::Compare { .. }),
            "expected Compare, got {expr:?}"
        );
        if let Expr::Compare { right, .. } = expr {
            assert_eq!(
                *right,
                Expr::Literal {
                    value: Scalar::Float64(0.5)
                }
            );
        }
    }

    #[test]
    fn parse_negative_leading_dot_float_literal() {
        let expr = super::parse_expr("x > -.5").unwrap();
        assert!(
            matches!(&expr, Expr::Compare { .. }),
            "expected Compare, got {expr:?}"
        );
        if let Expr::Compare { right, .. } = expr {
            assert_eq!(
                *right,
                Expr::Literal {
                    value: Scalar::Float64(-0.5)
                }
            );
        }
    }

    #[test]
    fn parse_local_reference() {
        let expr = super::parse_expr("x > @threshold").unwrap();
        assert!(
            matches!(&expr, Expr::Compare { .. }),
            "expected Compare, got {expr:?}"
        );
        if let Expr::Compare { right, .. } = expr {
            assert_eq!(
                *right,
                Expr::Local {
                    name: "threshold".into(),
                }
            );
        }
    }

    #[test]
    fn parse_membership_list_literal() {
        let expr = super::parse_expr("a in [1, 3,]").unwrap();
        let Expr::IsIn {
            left,
            values,
            negated,
        } = expr
        else {
            panic!("expected IsIn expression");
        };
        assert_eq!(
            left.as_ref(),
            &Expr::Series {
                name: SeriesRef("a".into())
            }
        );
        assert!(!negated);
        assert_eq!(values, vec![Scalar::Int64(1), Scalar::Int64(3)]);
    }

    #[test]
    fn parse_not_in_membership_list_literal() {
        let expr = super::parse_expr("name not in ['x', 'y']").unwrap();
        let Expr::IsIn {
            left,
            values,
            negated,
        } = expr
        else {
            panic!("expected IsIn expression");
        };
        assert_eq!(
            left.as_ref(),
            &Expr::Series {
                name: SeriesRef("name".into())
            }
        );
        assert!(negated);
        assert_eq!(
            values,
            vec![Scalar::Utf8("x".into()), Scalar::Utf8("y".into())]
        );
    }

    #[test]
    fn parse_abs_function_call() {
        let expr = super::parse_expr("abs(a - 3)").unwrap();
        let Expr::Abs { expr: inner } = expr else {
            panic!("expected Abs expression");
        };
        assert!(matches!(inner.as_ref(), Expr::Sub { .. }));
    }

    #[test]
    fn parse_null_predicate_method_calls() -> Result<(), ExprError> {
        let expr = super::parse_expr("a.isna()")?;
        let Expr::IsNull {
            expr: inner,
            negated,
        } = expr
        else {
            return Err(ExprError::ParseError("expected IsNull expression".into()));
        };
        assert!(!negated);
        assert_eq!(
            inner.as_ref(),
            &Expr::Series {
                name: SeriesRef("a".into())
            }
        );

        let expr = super::parse_expr("a.notnull()")?;
        let Expr::IsNull { negated, .. } = expr else {
            return Err(ExprError::ParseError("expected IsNull expression".into()));
        };
        assert!(negated);
        Ok(())
    }

    #[test]
    fn parse_isin_method_call_list_literal() -> Result<(), ExprError> {
        let expr = super::parse_expr("a.isin([1, 3,])")?;
        let Expr::IsIn {
            left,
            values,
            negated,
        } = expr
        else {
            return Err(ExprError::ParseError("expected IsIn expression".into()));
        };
        assert_eq!(
            left.as_ref(),
            &Expr::Series {
                name: SeriesRef("a".into())
            }
        );
        assert!(!negated);
        assert_eq!(values, vec![Scalar::Int64(1), Scalar::Int64(3)]);
        Ok(())
    }

    #[test]
    fn parse_between_method_call_scalar_bounds() -> Result<(), ExprError> {
        let expr = super::parse_expr("a.between(2, 8)")?;
        let Expr::Between {
            expr: inner,
            left,
            right,
            inclusive,
        } = expr
        else {
            return Err(ExprError::ParseError("expected Between expression".into()));
        };
        assert_eq!(
            inner.as_ref(),
            &Expr::Series {
                name: SeriesRef("a".into())
            }
        );
        assert_eq!(left, Scalar::Int64(2));
        assert_eq!(right, Scalar::Int64(8));
        assert_eq!(inclusive, BetweenInclusive::Both);
        Ok(())
    }

    #[test]
    fn parse_between_method_call_inclusive_argument() -> Result<(), ExprError> {
        let expr = super::parse_expr("a.between(2, 8, inclusive=\"left\")")?;
        let Expr::Between { inclusive, .. } = expr else {
            return Err(ExprError::ParseError("expected Between expression".into()));
        };
        assert_eq!(inclusive, BetweenInclusive::Left);

        let positional = super::parse_expr("a.between(2, 8, \"right\")")?;
        let Expr::Between { inclusive, .. } = positional else {
            return Err(ExprError::ParseError("expected Between expression".into()));
        };
        assert_eq!(inclusive, BetweenInclusive::Right);
        Ok(())
    }

    #[test]
    fn parse_clip_method_call_numeric_bounds() -> Result<(), ExprError> {
        let expr = super::parse_expr("a.clip(2, 8)")?;
        let Expr::Clip {
            expr: inner,
            lower,
            upper,
        } = expr
        else {
            return Err(ExprError::ParseError("expected Clip expression".into()));
        };
        assert_eq!(
            inner.as_ref(),
            &Expr::Series {
                name: SeriesRef("a".into())
            }
        );
        assert_eq!(lower, 2.0);
        assert_eq!(upper, 8.0);
        Ok(())
    }

    #[test]
    fn parse_round_method_call_optional_decimals() -> Result<(), ExprError> {
        let expr = super::parse_expr("a.round()")?;
        let Expr::Round {
            expr: inner,
            decimals,
        } = expr
        else {
            return Err(ExprError::ParseError("expected Round expression".into()));
        };
        assert_eq!(
            inner.as_ref(),
            &Expr::Series {
                name: SeriesRef("a".into())
            }
        );
        assert_eq!(decimals, 0);

        let positional = super::parse_expr("a.round(-1)")?;
        let Expr::Round { decimals, .. } = positional else {
            return Err(ExprError::ParseError("expected Round expression".into()));
        };
        assert_eq!(decimals, -1);

        let keyword = super::parse_expr("a.round(decimals=1)")?;
        let Expr::Round { decimals, .. } = keyword else {
            return Err(ExprError::ParseError("expected Round expression".into()));
        };
        assert_eq!(decimals, 1);
        Ok(())
    }

    #[test]
    fn parse_backtick_identifier() {
        let expr = super::parse_expr("`gross margin` + normal").unwrap();
        assert!(matches!(&expr, Expr::Add { .. }));
        if let Expr::Add { left, right } = expr {
            assert_eq!(
                *left,
                Expr::Series {
                    name: SeriesRef("gross margin".into()),
                }
            );
            assert_eq!(
                *right,
                Expr::Series {
                    name: SeriesRef("normal".into()),
                }
            );
        }
    }

    #[test]
    fn parse_error_single_equals() {
        assert!(super::parse_expr("x = 1").is_err());
    }

    #[test]
    fn parse_error_unterminated_backtick_identifier() {
        let err = super::parse_expr("`gross margin + normal").unwrap_err();
        assert!(
            matches!(err, ExprError::ParseError(msg) if msg.contains("unterminated backtick identifier"))
        );
    }

    #[test]
    fn parse_string_with_escaped_quote() {
        let expr = super::parse_expr(r#"name == 'O\'Brien'"#).unwrap();
        if let Expr::Compare { right, .. } = expr {
            assert_eq!(
                *right,
                Expr::Literal {
                    value: Scalar::Utf8("O'Brien".into())
                }
            );
        } else {
            panic!("expected Compare");
        }
    }

    #[test]
    fn parse_string_with_escape_sequences() {
        let expr = super::parse_expr(r#"msg == "hello\nworld""#).unwrap();
        if let Expr::Compare { right, .. } = expr {
            assert_eq!(
                *right,
                Expr::Literal {
                    value: Scalar::Utf8("hello\nworld".into())
                }
            );
        } else {
            panic!("expected Compare");
        }
    }

    #[test]
    fn parse_string_with_escaped_backslash() {
        let expr = super::parse_expr(r#"path == "c:\\users""#).unwrap();
        if let Expr::Compare { right, .. } = expr {
            assert_eq!(
                *right,
                Expr::Literal {
                    value: Scalar::Utf8("c:\\users".into())
                }
            );
        } else {
            panic!("expected Compare");
        }
    }

    #[test]
    fn parse_error_multiple_decimal_points() {
        let err = super::parse_expr("x > 1.2.3").unwrap_err();
        assert!(
            matches!(err, ExprError::ParseError(msg) if msg.contains("multiple decimal points"))
        );
    }

    #[test]
    fn parse_error_negative_multiple_decimal_points() {
        let err = super::parse_expr("x > -1.2.3").unwrap_err();
        assert!(
            matches!(err, ExprError::ParseError(msg) if msg.contains("multiple decimal points"))
        );
    }

    #[test]
    fn parse_error_leading_dot_multiple_decimal_points() {
        let err = super::parse_expr("x > .1.2").unwrap_err();
        assert!(
            matches!(err, ExprError::ParseError(msg) if msg.contains("multiple decimal points"))
        );
    }

    #[test]
    fn parse_eval_integration() {
        // Parse, then evaluate against a context
        let expr = super::parse_expr("a + b").unwrap();
        let policy = RuntimePolicy::hardened(Some(100));
        let mut ledger = EvidenceLedger::new();

        let a = fp_frame::Series::from_values(
            "a",
            vec![0_i64.into(), 1_i64.into()],
            vec![Scalar::Int64(10), Scalar::Int64(20)],
        )
        .unwrap();
        let b = fp_frame::Series::from_values(
            "b",
            vec![0_i64.into(), 1_i64.into()],
            vec![Scalar::Int64(3), Scalar::Int64(7)],
        )
        .unwrap();

        let mut ctx = EvalContext::new();
        ctx.insert_series(a);
        ctx.insert_series(b);

        let result = evaluate(&expr, &ctx, &policy, &mut ledger).unwrap();
        assert_eq!(result.values()[0], Scalar::Int64(13));
        assert_eq!(result.values()[1], Scalar::Int64(27));
    }

    #[test]
    fn eval_str_arithmetic() {
        let policy = RuntimePolicy::hardened(Some(100));
        let mut ledger = EvidenceLedger::new();

        let frame = fp_frame::DataFrame::from_series(vec![
            fp_frame::Series::from_values(
                "a",
                vec![0_i64.into(), 1_i64.into()],
                vec![Scalar::Int64(10), Scalar::Int64(20)],
            )
            .unwrap(),
            fp_frame::Series::from_values(
                "b",
                vec![0_i64.into(), 1_i64.into()],
                vec![Scalar::Int64(3), Scalar::Int64(7)],
            )
            .unwrap(),
        ])
        .unwrap();

        let result = super::eval_str("a + b", &frame, &policy, &mut ledger).unwrap();
        assert_eq!(result.values()[0], Scalar::Int64(13));
        assert_eq!(result.values()[1], Scalar::Int64(27));
    }

    #[test]
    fn eval_str_with_locals_broadcasts_scalar_bindings() {
        let policy = RuntimePolicy::hardened(Some(100));
        let mut ledger = EvidenceLedger::new();

        let frame = fp_frame::DataFrame::from_series(vec![
            fp_frame::Series::from_values(
                "a",
                vec![0_i64.into(), 1_i64.into()],
                vec![Scalar::Int64(10), Scalar::Int64(20)],
            )
            .unwrap(),
        ])
        .unwrap();

        let locals = BTreeMap::from([("offset".to_owned(), Scalar::Int64(5))]);
        let result =
            super::eval_str_with_locals("a + @offset * 2", &frame, &locals, &policy, &mut ledger)
                .unwrap();
        assert_eq!(result.values()[0], Scalar::Int64(20));
        assert_eq!(result.values()[1], Scalar::Int64(30));
    }

    #[test]
    fn eval_str_accepts_backtick_column_names() {
        let policy = RuntimePolicy::hardened(Some(100));
        let mut ledger = EvidenceLedger::new();

        let frame = fp_frame::DataFrame::from_series(vec![
            fp_frame::Series::from_values(
                "gross margin",
                vec![0_i64.into(), 1_i64.into()],
                vec![Scalar::Int64(10), Scalar::Int64(20)],
            )
            .unwrap(),
            fp_frame::Series::from_values(
                "and",
                vec![0_i64.into(), 1_i64.into()],
                vec![Scalar::Int64(1), Scalar::Int64(2)],
            )
            .unwrap(),
        ])
        .unwrap();

        let result =
            super::eval_str("`gross margin` + `and`", &frame, &policy, &mut ledger).unwrap();
        assert_eq!(result.values()[0], Scalar::Int64(11));
        assert_eq!(result.values()[1], Scalar::Int64(22));
    }

    #[test]
    fn eval_str_unary_minus_respects_pow_precedence() {
        let policy = RuntimePolicy::hardened(Some(100));
        let mut ledger = EvidenceLedger::new();

        let frame = fp_frame::DataFrame::from_series(vec![
            fp_frame::Series::from_values("anchor", vec![0_i64.into()], vec![Scalar::Int64(1)])
                .unwrap(),
        ])
        .unwrap();

        let result = super::eval_str("-2 ** 2", &frame, &policy, &mut ledger).unwrap();
        assert_eq!(result.values()[0], Scalar::Float64(-4.0));

        let result = super::eval_str("(-2) ** 2", &frame, &policy, &mut ledger).unwrap();
        assert_eq!(result.values()[0], Scalar::Float64(4.0));
    }

    #[test]
    fn query_str_with_locals_filters_rows() {
        let policy = RuntimePolicy::hardened(Some(100));
        let mut ledger = EvidenceLedger::new();

        let frame = fp_frame::DataFrame::from_series(vec![
            fp_frame::Series::from_values(
                "a",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
                vec![Scalar::Int64(1), Scalar::Int64(5), Scalar::Int64(3)],
            )
            .unwrap(),
            fp_frame::Series::from_values(
                "b",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
                vec![Scalar::Int64(10), Scalar::Int64(20), Scalar::Int64(30)],
            )
            .unwrap(),
        ])
        .unwrap();

        let locals = BTreeMap::from([
            ("threshold".to_owned(), Scalar::Int64(2)),
            ("limit".to_owned(), Scalar::Int64(25)),
        ]);
        let result = super::query_str_with_locals(
            "a > @threshold and b < @limit",
            &frame,
            &locals,
            &policy,
            &mut ledger,
        )
        .unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result.columns()["a"].values()[0], Scalar::Int64(5));
        assert_eq!(result.columns()["b"].values()[0], Scalar::Int64(20));
    }

    #[test]
    fn query_str_accepts_backtick_column_names() {
        let policy = RuntimePolicy::hardened(Some(100));
        let mut ledger = EvidenceLedger::new();

        let frame = fp_frame::DataFrame::from_series(vec![
            fp_frame::Series::from_values(
                "gross margin",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
                vec![Scalar::Int64(10), Scalar::Int64(20), Scalar::Int64(30)],
            )
            .unwrap(),
            fp_frame::Series::from_values(
                "and",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
                vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)],
            )
            .unwrap(),
        ])
        .unwrap();

        let result = super::query_str(
            "`gross margin` > 15 and `and` < 3",
            &frame,
            &policy,
            &mut ledger,
        )
        .unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(
            result.columns()["gross margin"].values()[0],
            Scalar::Int64(20)
        );
        assert_eq!(result.columns()["and"].values()[0], Scalar::Int64(2));
    }

    #[test]
    fn eval_str_with_locals_errors_on_unknown_binding() {
        let policy = RuntimePolicy::hardened(Some(100));
        let mut ledger = EvidenceLedger::new();

        let frame = fp_frame::DataFrame::from_series(vec![
            fp_frame::Series::from_values("a", vec![0_i64.into()], vec![Scalar::Int64(10)])
                .unwrap(),
        ])
        .unwrap();

        let err = super::eval_str_with_locals(
            "a > @threshold",
            &frame,
            &BTreeMap::new(),
            &policy,
            &mut ledger,
        )
        .unwrap_err();
        assert!(matches!(err, ExprError::UnknownLocal(name) if name == "threshold"));
    }

    #[test]
    fn query_str_filters_rows() {
        let policy = RuntimePolicy::hardened(Some(100));
        let mut ledger = EvidenceLedger::new();

        let frame = fp_frame::DataFrame::from_series(vec![
            fp_frame::Series::from_values(
                "x",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
                vec![Scalar::Int64(1), Scalar::Int64(5), Scalar::Int64(3)],
            )
            .unwrap(),
            fp_frame::Series::from_values(
                "y",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
                vec![Scalar::Int64(10), Scalar::Int64(20), Scalar::Int64(30)],
            )
            .unwrap(),
        ])
        .unwrap();

        let result = super::query_str("x > 2", &frame, &policy, &mut ledger).unwrap();
        assert_eq!(result.len(), 2); // rows where x=5 and x=3
        assert_eq!(result.columns()["x"].values()[0], Scalar::Int64(5));
        assert_eq!(result.columns()["x"].values()[1], Scalar::Int64(3));
    }

    #[test]
    fn query_str_filters_with_list_membership() {
        let policy = RuntimePolicy::hardened(Some(100));
        let mut ledger = EvidenceLedger::new();

        let frame = fp_frame::DataFrame::from_series(vec![
            fp_frame::Series::from_values(
                "a",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
                vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)],
            )
            .unwrap(),
            fp_frame::Series::from_values(
                "b",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
                vec![
                    Scalar::Utf8("x".into()),
                    Scalar::Utf8("y".into()),
                    Scalar::Utf8("x".into()),
                ],
            )
            .unwrap(),
        ])
        .unwrap();

        let result = super::query_str("a in [1, 3]", &frame, &policy, &mut ledger).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(
            result.columns()["a"].values(),
            &[Scalar::Int64(1), Scalar::Int64(3)]
        );

        let result = super::query_str("b not in ['x']", &frame, &policy, &mut ledger).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result.columns()["a"].values(), &[Scalar::Int64(2)]);
        assert_eq!(result.columns()["b"].values(), &[Scalar::Utf8("y".into())]);
    }

    #[test]
    fn query_str_exposes_index_namespace() {
        let policy = RuntimePolicy::hardened(Some(100));
        let mut ledger = EvidenceLedger::new();

        let frame = fp_frame::DataFrame::from_series(vec![
            fp_frame::Series::from_values(
                "a",
                vec![5_i64.into(), 6_i64.into(), 7_i64.into()],
                vec![Scalar::Int64(10), Scalar::Int64(20), Scalar::Int64(30)],
            )
            .unwrap(),
        ])
        .unwrap();

        let result = super::query_str("index > 5", &frame, &policy, &mut ledger).unwrap();
        assert_eq!(result.index().labels(), &[6_i64.into(), 7_i64.into()]);
        assert_eq!(
            result.columns()["a"].values(),
            &[Scalar::Int64(20), Scalar::Int64(30)]
        );

        let result = super::query_str("ilevel_0 > 5", &frame, &policy, &mut ledger).unwrap();
        assert_eq!(result.index().labels(), &[6_i64.into(), 7_i64.into()]);
    }

    #[test]
    fn query_str_exposes_named_index_without_shadowing_columns() {
        let policy = RuntimePolicy::hardened(Some(100));
        let mut ledger = EvidenceLedger::new();

        let frame = fp_frame::DataFrame::from_series(vec![
            fp_frame::Series::from_values(
                "idx",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
                vec![Scalar::Int64(5), Scalar::Int64(6), Scalar::Int64(7)],
            )
            .unwrap(),
            fp_frame::Series::from_values(
                "value",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
                vec![Scalar::Int64(10), Scalar::Int64(20), Scalar::Int64(30)],
            )
            .unwrap(),
        ])
        .unwrap()
        .set_index("idx", true)
        .unwrap();

        let result = super::query_str("idx > 5", &frame, &policy, &mut ledger).unwrap();
        assert_eq!(result.index().labels(), &[6_i64.into(), 7_i64.into()]);
        assert_eq!(
            result.columns()["value"].values(),
            &[Scalar::Int64(20), Scalar::Int64(30)]
        );

        let shadowing_frame = fp_frame::DataFrame::from_series(vec![
            fp_frame::Series::from_values(
                "index",
                vec![5_i64.into(), 6_i64.into(), 7_i64.into()],
                vec![Scalar::Int64(1), Scalar::Int64(10), Scalar::Int64(1)],
            )
            .unwrap(),
            fp_frame::Series::from_values(
                "value",
                vec![5_i64.into(), 6_i64.into(), 7_i64.into()],
                vec![Scalar::Int64(10), Scalar::Int64(20), Scalar::Int64(30)],
            )
            .unwrap(),
        ])
        .unwrap();

        let result = super::query_str("index > 5", &shadowing_frame, &policy, &mut ledger).unwrap();
        assert_eq!(result.index().labels(), &[6_i64.into()]);
        assert_eq!(result.columns()["index"].values(), &[Scalar::Int64(10)]);
    }

    #[test]
    fn eval_and_query_str_accept_abs_function() {
        let policy = RuntimePolicy::hardened(Some(100));
        let mut ledger = EvidenceLedger::new();

        let frame = fp_frame::DataFrame::from_series(vec![
            fp_frame::Series::from_values(
                "a",
                vec![
                    0_i64.into(),
                    1_i64.into(),
                    2_i64.into(),
                    3_i64.into(),
                    4_i64.into(),
                ],
                vec![
                    Scalar::Int64(-2),
                    Scalar::Int64(-1),
                    Scalar::Int64(0),
                    Scalar::Int64(1),
                    Scalar::Int64(2),
                ],
            )
            .unwrap(),
        ])
        .unwrap();

        let evaluated = super::eval_str("abs(a)", &frame, &policy, &mut ledger).unwrap();
        assert_eq!(
            evaluated.values(),
            &[
                Scalar::Int64(2),
                Scalar::Int64(1),
                Scalar::Int64(0),
                Scalar::Int64(1),
                Scalar::Int64(2),
            ]
        );

        let filtered = super::query_str("abs(a) > 1", &frame, &policy, &mut ledger).unwrap();
        assert_eq!(filtered.index().labels(), &[0_i64.into(), 4_i64.into()]);
        assert_eq!(
            filtered.columns()["a"].values(),
            &[Scalar::Int64(-2), Scalar::Int64(2)]
        );
    }

    #[test]
    fn eval_and_query_str_accept_null_predicate_methods() -> Result<(), ExprError> {
        let policy = RuntimePolicy::hardened(Some(100));
        let mut ledger = EvidenceLedger::new();

        let frame = fp_frame::DataFrame::from_series(vec![
            fp_frame::Series::from_values(
                "a",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
                vec![
                    Scalar::Int64(1),
                    Scalar::Null(fp_types::NullKind::Null),
                    Scalar::Int64(3),
                ],
            )
            .map_err(ExprError::from)?,
        ])
        .map_err(ExprError::from)?;

        let isna = super::eval_str("a.isna()", &frame, &policy, &mut ledger)?;
        assert_eq!(
            isna.values(),
            &[Scalar::Bool(false), Scalar::Bool(true), Scalar::Bool(false)]
        );

        let isnull = super::query_str("a.isnull()", &frame, &policy, &mut ledger)?;
        assert_eq!(isnull.index().labels(), &[1_i64.into()]);
        assert_eq!(
            isnull.columns()["a"].values(),
            &[Scalar::Null(fp_types::NullKind::Null)]
        );

        let notna = super::query_str("a.notna()", &frame, &policy, &mut ledger)?;
        assert_eq!(notna.index().labels(), &[0_i64.into(), 2_i64.into()]);

        let notnull = super::eval_str("a.notnull()", &frame, &policy, &mut ledger)?;
        assert_eq!(
            notnull.values(),
            &[Scalar::Bool(true), Scalar::Bool(false), Scalar::Bool(true)]
        );
        Ok(())
    }

    #[test]
    fn eval_and_query_str_accept_isin_method_call() -> Result<(), ExprError> {
        let policy = RuntimePolicy::hardened(Some(100));
        let mut ledger = EvidenceLedger::new();

        let frame = fp_frame::DataFrame::from_series(vec![
            fp_frame::Series::from_values(
                "a",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
                vec![Scalar::Int64(1), Scalar::Int64(4), Scalar::Int64(9)],
            )
            .map_err(ExprError::from)?,
        ])
        .map_err(ExprError::from)?;

        let mask = super::eval_str("a.isin([1, 9])", &frame, &policy, &mut ledger)?;
        assert_eq!(
            mask.values(),
            &[Scalar::Bool(true), Scalar::Bool(false), Scalar::Bool(true)]
        );

        let filtered = super::query_str("a.isin([1, 9])", &frame, &policy, &mut ledger)?;
        assert_eq!(filtered.index().labels(), &[0_i64.into(), 2_i64.into()]);
        assert_eq!(
            filtered.columns()["a"].values(),
            &[Scalar::Int64(1), Scalar::Int64(9)]
        );
        Ok(())
    }

    #[test]
    fn eval_and_query_str_accept_between_method_call() -> Result<(), ExprError> {
        let policy = RuntimePolicy::hardened(Some(100));
        let mut ledger = EvidenceLedger::new();

        let frame = fp_frame::DataFrame::from_series(vec![
            fp_frame::Series::from_values(
                "a",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
                vec![
                    Scalar::Int64(1),
                    Scalar::Int64(2),
                    Scalar::Int64(8),
                    Scalar::Int64(9),
                ],
            )
            .map_err(ExprError::from)?,
        ])
        .map_err(ExprError::from)?;

        let mask = super::eval_str("a.between(2, 8)", &frame, &policy, &mut ledger)?;
        assert_eq!(
            mask.values(),
            &[
                Scalar::Bool(false),
                Scalar::Bool(true),
                Scalar::Bool(true),
                Scalar::Bool(false)
            ]
        );

        let filtered = super::query_str("a.between(2, 8)", &frame, &policy, &mut ledger)?;
        assert_eq!(filtered.index().labels(), &[1_i64.into(), 2_i64.into()]);
        assert_eq!(
            filtered.columns()["a"].values(),
            &[Scalar::Int64(2), Scalar::Int64(8)]
        );

        let left_inclusive = super::eval_str(
            "a.between(2, 8, inclusive=\"left\")",
            &frame,
            &policy,
            &mut ledger,
        )?;
        assert_eq!(
            left_inclusive.values(),
            &[
                Scalar::Bool(false),
                Scalar::Bool(true),
                Scalar::Bool(false),
                Scalar::Bool(false)
            ]
        );

        let right_inclusive =
            super::eval_str("a.between(2, 8, \"right\")", &frame, &policy, &mut ledger)?;
        assert_eq!(
            right_inclusive.values(),
            &[
                Scalar::Bool(false),
                Scalar::Bool(false),
                Scalar::Bool(true),
                Scalar::Bool(false)
            ]
        );

        let neither = super::query_str(
            "a.between(2, 8, inclusive=\"neither\")",
            &frame,
            &policy,
            &mut ledger,
        )?;
        assert!(neither.is_empty());
        Ok(())
    }

    #[test]
    fn eval_and_query_str_accept_clip_method_call() -> Result<(), ExprError> {
        let policy = RuntimePolicy::hardened(Some(100));
        let mut ledger = EvidenceLedger::new();

        let frame = fp_frame::DataFrame::from_series(vec![
            fp_frame::Series::from_values(
                "a",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
                vec![Scalar::Int64(1), Scalar::Int64(4), Scalar::Int64(9)],
            )
            .map_err(ExprError::from)?,
        ])
        .map_err(ExprError::from)?;

        let clipped = super::eval_str("a.clip(2, 8)", &frame, &policy, &mut ledger)?;
        assert_eq!(
            clipped.values(),
            &[Scalar::Int64(2), Scalar::Int64(4), Scalar::Int64(8)]
        );

        let filtered = super::query_str("a.clip(2, 8) == 8", &frame, &policy, &mut ledger)?;
        assert_eq!(filtered.index().labels(), &[2_i64.into()]);
        assert_eq!(filtered.columns()["a"].values(), &[Scalar::Int64(9)]);
        Ok(())
    }

    #[test]
    fn eval_and_query_str_accept_round_method_call() -> Result<(), ExprError> {
        let policy = RuntimePolicy::hardened(Some(100));
        let mut ledger = EvidenceLedger::new();

        let frame = fp_frame::DataFrame::from_series(vec![
            fp_frame::Series::from_values(
                "a",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
                vec![
                    Scalar::Float64(1.25),
                    Scalar::Float64(2.75),
                    Scalar::Float64(8.25),
                ],
            )
            .map_err(ExprError::from)?,
        ])
        .map_err(ExprError::from)?;

        let rounded = super::eval_str("a.round()", &frame, &policy, &mut ledger)?;
        assert_eq!(
            rounded.values(),
            &[
                Scalar::Float64(1.0),
                Scalar::Float64(3.0),
                Scalar::Float64(8.0)
            ]
        );

        let one_decimal = super::eval_str("a.round(decimals=1)", &frame, &policy, &mut ledger)?;
        assert_eq!(
            one_decimal.values(),
            &[
                Scalar::Float64(1.2),
                Scalar::Float64(2.8),
                Scalar::Float64(8.2)
            ]
        );

        let filtered = super::query_str("a.round(-1) >= 10", &frame, &policy, &mut ledger)?;
        assert_eq!(filtered.index().labels(), &[2_i64.into()]);
        assert_eq!(filtered.columns()["a"].values(), &[Scalar::Float64(8.25)]);
        Ok(())
    }

    #[test]
    fn query_str_compound_filter() {
        let policy = RuntimePolicy::hardened(Some(100));
        let mut ledger = EvidenceLedger::new();

        let frame = fp_frame::DataFrame::from_series(vec![
            fp_frame::Series::from_values(
                "a",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
                vec![Scalar::Int64(1), Scalar::Int64(5), Scalar::Int64(3)],
            )
            .unwrap(),
            fp_frame::Series::from_values(
                "b",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
                vec![Scalar::Int64(10), Scalar::Int64(20), Scalar::Int64(30)],
            )
            .unwrap(),
        ])
        .unwrap();

        // Query for a > 2 and b < 25
        let result = super::query_str("a > 2 and b < 25", &frame, &policy, &mut ledger).unwrap();
        assert_eq!(result.len(), 1); // only row 1 (a=5, b=20)
        assert_eq!(result.columns()["a"].values()[0], Scalar::Int64(5));
        assert_eq!(result.columns()["b"].values()[0], Scalar::Int64(20));
    }

    #[test]
    fn query_str_chained_comparison_matches_pandas_pairwise_semantics() {
        let policy = RuntimePolicy::hardened(Some(100));
        let mut ledger = EvidenceLedger::new();

        let frame = fp_frame::DataFrame::from_series(vec![
            fp_frame::Series::from_values(
                "a",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
                vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)],
            )
            .unwrap(),
            fp_frame::Series::from_values(
                "b",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
                vec![Scalar::Int64(2), Scalar::Int64(2), Scalar::Int64(4)],
            )
            .unwrap(),
            fp_frame::Series::from_values(
                "c",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
                vec![Scalar::Int64(3), Scalar::Int64(1), Scalar::Int64(5)],
            )
            .unwrap(),
        ])
        .unwrap();

        let result = super::query_str("a < b < c", &frame, &policy, &mut ledger).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(
            result.columns()["a"].values(),
            &[Scalar::Int64(1), Scalar::Int64(3)]
        );
    }

    // ── eval/query parity tests (frankenpandas-xmp) ──

    #[test]
    fn eval_str_subtraction() {
        let policy = RuntimePolicy::hardened(Some(100));
        let mut ledger = EvidenceLedger::new();

        let frame = fp_frame::DataFrame::from_series(vec![
            fp_frame::Series::from_values(
                "a",
                vec![0_i64.into(), 1_i64.into()],
                vec![Scalar::Int64(10), Scalar::Int64(20)],
            )
            .unwrap(),
            fp_frame::Series::from_values(
                "b",
                vec![0_i64.into(), 1_i64.into()],
                vec![Scalar::Int64(3), Scalar::Int64(7)],
            )
            .unwrap(),
        ])
        .unwrap();

        let result = super::eval_str("a - b", &frame, &policy, &mut ledger).unwrap();
        assert_eq!(result.values()[0], Scalar::Int64(7));
        assert_eq!(result.values()[1], Scalar::Int64(13));
    }

    #[test]
    fn eval_str_multiplication() {
        let policy = RuntimePolicy::hardened(Some(100));
        let mut ledger = EvidenceLedger::new();

        let frame = fp_frame::DataFrame::from_series(vec![
            fp_frame::Series::from_values(
                "a",
                vec![0_i64.into(), 1_i64.into()],
                vec![Scalar::Int64(3), Scalar::Int64(5)],
            )
            .unwrap(),
            fp_frame::Series::from_values(
                "b",
                vec![0_i64.into(), 1_i64.into()],
                vec![Scalar::Int64(4), Scalar::Int64(6)],
            )
            .unwrap(),
        ])
        .unwrap();

        let result = super::eval_str("a * b", &frame, &policy, &mut ledger).unwrap();
        assert_eq!(result.values()[0], Scalar::Int64(12));
        assert_eq!(result.values()[1], Scalar::Int64(30));
    }

    #[test]
    fn eval_str_division() {
        let policy = RuntimePolicy::hardened(Some(100));
        let mut ledger = EvidenceLedger::new();

        let frame = fp_frame::DataFrame::from_series(vec![
            fp_frame::Series::from_values(
                "a",
                vec![0_i64.into(), 1_i64.into()],
                vec![Scalar::Float64(10.0), Scalar::Float64(21.0)],
            )
            .unwrap(),
            fp_frame::Series::from_values(
                "b",
                vec![0_i64.into(), 1_i64.into()],
                vec![Scalar::Float64(2.0), Scalar::Float64(7.0)],
            )
            .unwrap(),
        ])
        .unwrap();

        let result = super::eval_str("a / b", &frame, &policy, &mut ledger).unwrap();
        assert_eq!(result.values()[0], Scalar::Float64(5.0));
        assert_eq!(result.values()[1], Scalar::Float64(3.0));
    }

    #[test]
    fn eval_str_parenthesized_expression() {
        let policy = RuntimePolicy::hardened(Some(100));
        let mut ledger = EvidenceLedger::new();

        let frame = fp_frame::DataFrame::from_series(vec![
            fp_frame::Series::from_values("a", vec![0_i64.into()], vec![Scalar::Int64(2)]).unwrap(),
            fp_frame::Series::from_values("b", vec![0_i64.into()], vec![Scalar::Int64(3)]).unwrap(),
            fp_frame::Series::from_values("c", vec![0_i64.into()], vec![Scalar::Int64(4)]).unwrap(),
        ])
        .unwrap();

        // (a + b) * c = (2 + 3) * 4 = 20
        let result = super::eval_str("(a + b) * c", &frame, &policy, &mut ledger).unwrap();
        assert_eq!(result.values()[0], Scalar::Int64(20));
    }

    #[test]
    fn eval_str_comparison() {
        let policy = RuntimePolicy::hardened(Some(100));
        let mut ledger = EvidenceLedger::new();

        let frame = fp_frame::DataFrame::from_series(vec![
            fp_frame::Series::from_values(
                "x",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
                vec![Scalar::Int64(1), Scalar::Int64(5), Scalar::Int64(3)],
            )
            .unwrap(),
        ])
        .unwrap();

        let result = super::eval_str("x >= 3", &frame, &policy, &mut ledger).unwrap();
        assert_eq!(result.values()[0], Scalar::Bool(false));
        assert_eq!(result.values()[1], Scalar::Bool(true));
        assert_eq!(result.values()[2], Scalar::Bool(true));
    }

    #[test]
    fn query_str_equality() {
        let policy = RuntimePolicy::hardened(Some(100));
        let mut ledger = EvidenceLedger::new();

        let frame = fp_frame::DataFrame::from_series(vec![
            fp_frame::Series::from_values(
                "name",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
                vec![Scalar::Int64(10), Scalar::Int64(20), Scalar::Int64(10)],
            )
            .unwrap(),
        ])
        .unwrap();

        let result = super::query_str("name == 10", &frame, &policy, &mut ledger).unwrap();
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn query_str_or_filter() {
        let policy = RuntimePolicy::hardened(Some(100));
        let mut ledger = EvidenceLedger::new();

        let frame = fp_frame::DataFrame::from_series(vec![
            fp_frame::Series::from_values(
                "x",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
                vec![
                    Scalar::Int64(1),
                    Scalar::Int64(5),
                    Scalar::Int64(3),
                    Scalar::Int64(8),
                ],
            )
            .unwrap(),
        ])
        .unwrap();

        // x < 2 or x > 7 → rows where x=1 or x=8
        let result = super::query_str("x < 2 or x > 7", &frame, &policy, &mut ledger).unwrap();
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn query_str_accepts_pandas_boolean_literals_in_compound_filters() {
        let policy = RuntimePolicy::hardened(Some(100));
        let mut ledger = EvidenceLedger::new();

        let frame = fp_frame::DataFrame::from_series(vec![
            fp_frame::Series::from_values(
                "x",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
                vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)],
            )
            .unwrap(),
            fp_frame::Series::from_values(
                "flag",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
                vec![Scalar::Bool(true), Scalar::Bool(false), Scalar::Bool(true)],
            )
            .unwrap(),
        ])
        .unwrap();

        let true_rows = super::query_str("flag == True", &frame, &policy, &mut ledger).unwrap();
        assert_eq!(true_rows.len(), 2);
        assert_eq!(
            true_rows.column("x").expect("x").values(),
            &[Scalar::Int64(1), Scalar::Int64(3)]
        );

        let gated = super::query_str("x > 1 and True", &frame, &policy, &mut ledger).unwrap();
        assert_eq!(gated.len(), 2);
        assert_eq!(
            gated.column("x").expect("x").values(),
            &[Scalar::Int64(2), Scalar::Int64(3)]
        );

        let false_or = super::query_str("False or x > 1", &frame, &policy, &mut ledger).unwrap();
        assert_eq!(
            false_or.column("x").expect("x").values(),
            gated.column("x").expect("x").values()
        );
    }

    #[test]
    fn query_str_rejects_scalar_boolean_literal_filters() {
        let policy = RuntimePolicy::hardened(Some(100));
        let mut ledger = EvidenceLedger::new();

        let frame = fp_frame::DataFrame::from_series(vec![
            fp_frame::Series::from_values(
                "x",
                vec![0_i64.into(), 1_i64.into()],
                vec![Scalar::Int64(1), Scalar::Int64(2)],
            )
            .unwrap(),
        ])
        .unwrap();

        let result = super::query_str("True", &frame, &policy, &mut ledger);
        assert!(matches!(
            result,
            Err(ExprError::Frame(FrameError::CompatibilityRejected(message)))
                if message.contains("scalar boolean query")
        ));
    }

    #[test]
    fn eval_extension_trait() {
        use super::DataFrameExprExt;

        let frame = fp_frame::DataFrame::from_dict(
            &["a", "b"],
            vec![
                ("a", vec![Scalar::Int64(1), Scalar::Int64(2)]),
                ("b", vec![Scalar::Int64(10), Scalar::Int64(20)]),
            ],
        )
        .unwrap();

        let result = frame.eval("a + b").unwrap();
        assert_eq!(result.values()[0], Scalar::Int64(11));
        assert_eq!(result.values()[1], Scalar::Int64(22));
    }

    #[test]
    fn query_extension_trait() {
        use super::DataFrameExprExt;

        let frame = fp_frame::DataFrame::from_dict(
            &["x", "y"],
            vec![
                (
                    "x",
                    vec![Scalar::Int64(1), Scalar::Int64(5), Scalar::Int64(3)],
                ),
                (
                    "y",
                    vec![Scalar::Int64(10), Scalar::Int64(20), Scalar::Int64(30)],
                ),
            ],
        )
        .unwrap();

        let result = frame.query("x > 2 and y < 25").unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result.columns()["x"].values()[0], Scalar::Int64(5));
    }

    #[test]
    fn eval_with_locals_extension_trait() {
        use super::DataFrameExprExt;

        let frame = fp_frame::DataFrame::from_dict(
            &["val"],
            vec![(
                "val",
                vec![Scalar::Int64(10), Scalar::Int64(20), Scalar::Int64(30)],
            )],
        )
        .unwrap();

        let locals = BTreeMap::from([("threshold".to_owned(), Scalar::Int64(15))]);
        let result = frame
            .query_with_locals("val > @threshold", &locals)
            .unwrap();
        assert_eq!(result.len(), 2);
    }
}
