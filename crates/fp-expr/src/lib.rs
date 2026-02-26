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

    pub fn from_dataframe(frame: &fp_frame::DataFrame) -> Result<Self, ExprError> {
        let mut context = Self::new();
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
}

#[derive(Debug, Error)]
pub enum ExprError {
    #[error("unknown series reference: {0}")]
    UnknownSeries(String),
    #[error("cannot evaluate a pure literal expression without an index anchor")]
    UnanchoredLiteral,
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

pub fn evaluate_on_dataframe(
    expr: &Expr,
    frame: &fp_frame::DataFrame,
    policy: &RuntimePolicy,
    ledger: &mut EvidenceLedger,
) -> Result<Series, ExprError> {
    let context = EvalContext::from_dataframe(frame)?;
    evaluate(expr, &context, policy, ledger)
}

pub fn filter_dataframe_on_expr(
    expr: &Expr,
    frame: &fp_frame::DataFrame,
    policy: &RuntimePolicy,
    ledger: &mut EvidenceLedger,
) -> Result<fp_frame::DataFrame, ExprError> {
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
    let expr = parse_expr(expr_str)?;
    evaluate_on_dataframe(&expr, frame, policy, ledger)
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
    let expr = parse_expr(expr_str)?;
    filter_dataframe_on_expr(&expr, frame, policy, ledger)
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

    /// Filter rows by a boolean expression string.
    ///
    /// Matches `pd.DataFrame.query(expr)`.
    fn query(&self, expr_str: &str) -> Result<fp_frame::DataFrame, ExprError>;
}

impl DataFrameExprExt for fp_frame::DataFrame {
    fn eval(&self, expr_str: &str) -> Result<Series, ExprError> {
        let policy = RuntimePolicy::hardened(Some(100_000));
        let mut ledger = EvidenceLedger::new();
        eval_str(expr_str, self, &policy, &mut ledger)
    }

    fn query(&self, expr_str: &str) -> Result<fp_frame::DataFrame, ExprError> {
        let policy = RuntimePolicy::hardened(Some(100_000));
        let mut ledger = EvidenceLedger::new();
        query_str(expr_str, self, &policy, &mut ledger)
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

    fn extract_series(expr: &Expr, series_set: &mut std::collections::BTreeSet<String>) {
        match expr {
            Expr::Series { name } => {
                series_set.insert(name.0.clone());
            }
            Expr::Add { left, right }
            | Expr::Sub { left, right }
            | Expr::Mul { left, right }
            | Expr::Div { left, right }
            | Expr::And { left, right }
            | Expr::Or { left, right }
            | Expr::Compare { left, right, .. } => {
                Self::extract_series(left, series_set);
                Self::extract_series(right, series_set);
            }
            Expr::Not { expr } => Self::extract_series(expr, series_set),
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

// ── Expression Parser ───────────────────────────────────────────────────
//
// A simple recursive-descent parser for pandas-style query/eval expressions.
// Supports:
//   - Column references (identifiers)
//   - Numeric literals (integer and float)
//   - String literals ('...' or "...")
//   - Comparison operators: ==, !=, >, >=, <, <=
//   - Logical operators: and, or, not
//   - Arithmetic operators: +, -, *, /
//   - Parenthesized sub-expressions

/// Parse a string expression into an `Expr` AST.
///
/// Syntax:
///   expr       → or_expr
///   or_expr    → and_expr ( "or" and_expr )*
///   and_expr   → not_expr ( "and" not_expr )*
///   not_expr   → "not" not_expr | comparison
///   comparison → add_expr ( ("==" | "!=" | ">" | ">=" | "<" | "<=") add_expr )?
///   add_expr   → mul_expr ( ("+" | "-") mul_expr )*
///   mul_expr   → atom ( ("*" | "/") atom )*
///   atom       → NUMBER | STRING | IDENT | "(" expr ")"
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
    Int(i64),
    Float(f64),
    Str(String),
    // Comparison
    EqEq,
    NotEq,
    Gt,
    Ge,
    Lt,
    Le,
    // Arithmetic
    Plus,
    Minus,
    Star,
    Slash,
    // Grouping
    LParen,
    RParen,
    // Logical (keywords)
    And,
    Or,
    Not,
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
                    && chars[i + 1].is_ascii_digit()
                    && (tokens.is_empty()
                        || matches!(
                            tokens.last(),
                            Some(
                                Token::LParen
                                    | Token::EqEq
                                    | Token::NotEq
                                    | Token::Gt
                                    | Token::Ge
                                    | Token::Lt
                                    | Token::Le
                                    | Token::Plus
                                    | Token::Minus
                                    | Token::Star
                                    | Token::Slash
                                    | Token::And
                                    | Token::Or
                                    | Token::Not
                            )
                        ))
                {
                    let start = i;
                    i += 1;
                    while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                        i += 1;
                    }
                    let num_str: String = chars[start..i].iter().collect();
                    if num_str.contains('.') {
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
            '*' => {
                tokens.push(Token::Star);
                i += 1;
            }
            '/' => {
                tokens.push(Token::Slash);
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
            '=' => {
                if i + 1 < chars.len() && chars[i + 1] == '=' {
                    tokens.push(Token::EqEq);
                    i += 2;
                } else {
                    return Err(ExprError::ParseError(
                        "expected '==' but found single '='".into(),
                    ));
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
                let start = i;
                while i < chars.len() && chars[i] != quote {
                    i += 1;
                }
                if i >= chars.len() {
                    return Err(ExprError::ParseError("unterminated string literal".into()));
                }
                let s: String = chars[start..i].iter().collect();
                tokens.push(Token::Str(s));
                i += 1; // skip closing quote
            }
            _ if c.is_ascii_digit() => {
                let start = i;
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                    i += 1;
                }
                let num_str: String = chars[start..i].iter().collect();
                if num_str.contains('.') {
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
    let left = parse_add(tokens, pos)?;
    if *pos < tokens.len() {
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
            return Ok(Expr::Compare {
                left: Box::new(left),
                right: Box::new(right),
                op,
            });
        }
    }
    Ok(left)
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
    let mut left = parse_atom(tokens, pos)?;
    while *pos < tokens.len() {
        match &tokens[*pos] {
            Token::Star => {
                *pos += 1;
                let right = parse_atom(tokens, pos)?;
                left = Expr::Mul {
                    left: Box::new(left),
                    right: Box::new(right),
                };
            }
            Token::Slash => {
                *pos += 1;
                let right = parse_atom(tokens, pos)?;
                left = Expr::Div {
                    left: Box::new(left),
                    right: Box::new(right),
                };
            }
            _ => break,
        }
    }
    Ok(left)
}

fn parse_atom(tokens: &[Token], pos: &mut usize) -> Result<Expr, ExprError> {
    if *pos >= tokens.len() {
        return Err(ExprError::ParseError(
            "unexpected end of expression".into(),
        ));
    }
    match &tokens[*pos] {
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
        Token::Ident(name) => {
            let name = name.clone();
            *pos += 1;
            Ok(Expr::Series {
                name: SeriesRef(name),
            })
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
    }
}

#[cfg(test)]
mod tests {
    use fp_columnar::ComparisonOp;
    use fp_runtime::{EvidenceLedger, RuntimePolicy};
    use fp_types::Scalar;

    use super::{EvalContext, Expr, ExprError, SeriesRef, evaluate};
    use fp_frame::{FrameError, Series};

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
    fn parse_and_or_expression() {
        let expr = super::parse_expr("a > 1 and b < 2").unwrap();
        match expr {
            Expr::And { left, right } => {
                assert!(matches!(*left, Expr::Compare { .. }));
                assert!(matches!(*right, Expr::Compare { .. }));
            }
            other => panic!("expected And, got {other:?}"),
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
        match expr {
            Expr::Add { left, right } => {
                assert!(matches!(*left, Expr::Series { .. }));
                assert!(matches!(*right, Expr::Mul { .. }));
            }
            other => panic!("expected Add, got {other:?}"),
        }
    }

    #[test]
    fn parse_parentheses() {
        let expr = super::parse_expr("(a + b) * 2").unwrap();
        match expr {
            Expr::Mul { left, right } => {
                assert!(matches!(*left, Expr::Add { .. }));
                assert!(matches!(
                    *right,
                    Expr::Literal {
                        value: Scalar::Int64(2)
                    }
                ));
            }
            other => panic!("expected Mul, got {other:?}"),
        }
    }

    #[test]
    fn parse_string_literal() {
        let expr = super::parse_expr("name == 'alice'").unwrap();
        match expr {
            Expr::Compare { right, op, .. } => {
                assert_eq!(op, ComparisonOp::Eq);
                assert_eq!(
                    *right,
                    Expr::Literal {
                        value: Scalar::Utf8("alice".into())
                    }
                );
            }
            other => panic!("expected Compare, got {other:?}"),
        }
    }

    #[test]
    fn parse_float_literal() {
        let expr = super::parse_expr("x > 4.56").unwrap();
        match expr {
            Expr::Compare { right, .. } => {
                assert_eq!(
                    *right,
                    Expr::Literal {
                        value: Scalar::Float64(4.56)
                    }
                );
            }
            other => panic!("expected Compare, got {other:?}"),
        }
    }

    #[test]
    fn parse_negative_literal() {
        let expr = super::parse_expr("x > -5").unwrap();
        match expr {
            Expr::Compare { right, .. } => {
                assert_eq!(
                    *right,
                    Expr::Literal {
                        value: Scalar::Int64(-5)
                    }
                );
            }
            other => panic!("expected Compare, got {other:?}"),
        }
    }

    #[test]
    fn parse_error_single_equals() {
        assert!(super::parse_expr("x = 1").is_err());
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
        let result =
            super::query_str("a > 2 and b < 25", &frame, &policy, &mut ledger).unwrap();
        assert_eq!(result.len(), 1); // only row 1 (a=5, b=20)
        assert_eq!(result.columns()["a"].values()[0], Scalar::Int64(5));
        assert_eq!(result.columns()["b"].values()[0], Scalar::Int64(20));
    }
}
