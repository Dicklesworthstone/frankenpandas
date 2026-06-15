//! PyO3 Python bindings for FrankenPandas.
//!
//! Exposes DataFrame, Series, and core operations to Python.
//!
//! ```python
//! import frankenpandas as fp
//!
//! # Create a Series
//! s = fp.Series("values", [1, 2, 3, 4, 5])
//! print(s.sum())  # 15
//!
//! # Create a DataFrame
//! df = fp.DataFrame({"a": [1, 2, 3], "b": [4.0, 5.0, 6.0]})
//! print(df.head(2))
//! ```

use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::{DataFrame, Series};
use fp_index::{Index, IndexLabel};
use fp_types::Scalar;
use pyo3::{
    IntoPyObjectExt,
    prelude::*,
    types::{PyDict, PyList},
};

/// Convert a Python value to a FrankenPandas Scalar.
fn py_to_scalar(_py: Python<'_>, obj: &Bound<'_, PyAny>) -> PyResult<Scalar> {
    if obj.is_none() {
        return Ok(Scalar::Null(fp_types::NullKind::Null));
    }
    if let Ok(b) = obj.extract::<bool>() {
        return Ok(Scalar::Bool(b));
    }
    if let Ok(i) = obj.extract::<i64>() {
        return Ok(Scalar::Int64(i));
    }
    if let Ok(f) = obj.extract::<f64>() {
        return Ok(Scalar::Float64(f));
    }
    if let Ok(s) = obj.extract::<String>() {
        return Ok(Scalar::Utf8(s));
    }
    Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(format!(
        "Cannot convert {} to Scalar",
        obj.get_type().name()?
    )))
}

/// Convert a FrankenPandas Scalar to a Python object.
fn scalar_to_py(py: Python<'_>, scalar: &Scalar) -> PyResult<Py<PyAny>> {
    match scalar {
        Scalar::Null(_) => Ok(py.None()),
        Scalar::Bool(b) => b.into_py_any(py),
        Scalar::Int64(i) => i.into_py_any(py),
        Scalar::Float64(f) => f.into_py_any(py),
        Scalar::Utf8(s) => s.into_py_any(py),
        Scalar::Datetime64(ns) => ns.into_py_any(py),
        Scalar::Timedelta64(ns) => ns.into_py_any(py),
        Scalar::Period(p) => p.ordinal.into_py_any(py),
        Scalar::Interval(_) => Ok(py.None()),
    }
}

/// Python wrapper for FrankenPandas Series.
#[pyclass(name = "Series", from_py_object)]
#[derive(Clone)]
pub struct PySeries {
    inner: Series,
}

#[pymethods]
impl PySeries {
    /// Create a new Series from a name and list of values.
    #[new]
    #[pyo3(signature = (name, values))]
    fn new(py: Python<'_>, name: &str, values: &Bound<'_, PyList>) -> PyResult<Self> {
        let scalars: Vec<Scalar> = values
            .iter()
            .map(|v| py_to_scalar(py, &v))
            .collect::<PyResult<Vec<_>>>()?;

        let labels: Vec<IndexLabel> = (0..scalars.len())
            .map(|i| IndexLabel::Int64(i as i64))
            .collect();

        let series = Series::from_values(name, labels, scalars)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;

        Ok(PySeries { inner: series })
    }

    /// Return the name of the Series.
    #[getter]
    fn name(&self) -> &str {
        self.inner.name()
    }

    /// Return the length of the Series.
    fn __len__(&self) -> usize {
        self.inner.len()
    }

    /// Return a string representation (renders values, like pandas).
    fn __repr__(&self) -> String {
        format!("{}", self.inner)
    }

    /// Return the sum of the Series.
    fn sum(&self) -> PyResult<Py<PyAny>> {
        Python::attach(|py| {
            let result = self
                .inner
                .sum()
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
            scalar_to_py(py, &result)
        })
    }

    /// Return the mean of the Series.
    fn mean(&self) -> PyResult<Py<PyAny>> {
        Python::attach(|py| {
            let result = self
                .inner
                .mean()
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
            scalar_to_py(py, &result)
        })
    }

    /// Return the minimum value.
    fn min(&self) -> PyResult<Py<PyAny>> {
        Python::attach(|py| {
            let result = self
                .inner
                .min()
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
            scalar_to_py(py, &result)
        })
    }

    /// Return the maximum value.
    fn max(&self) -> PyResult<Py<PyAny>> {
        Python::attach(|py| {
            let result = self
                .inner
                .max()
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
            scalar_to_py(py, &result)
        })
    }

    /// Return the standard deviation.
    fn std(&self) -> PyResult<Py<PyAny>> {
        Python::attach(|py| {
            let result = self
                .inner
                .std()
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
            scalar_to_py(py, &result)
        })
    }

    /// Return the first n elements.
    fn head(&self, n: Option<i64>) -> PyResult<PySeries> {
        let n = n.unwrap_or(5);
        let result = self
            .inner
            .head(n)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PySeries { inner: result })
    }

    /// Return the last n elements.
    fn tail(&self, n: Option<i64>) -> PyResult<PySeries> {
        let n = n.unwrap_or(5);
        let result = self
            .inner
            .tail(n)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PySeries { inner: result })
    }

    /// Return values as a Python list.
    fn tolist(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let values: Vec<Py<PyAny>> = self
            .inner
            .column()
            .values()
            .iter()
            .map(|s| scalar_to_py(py, s))
            .collect::<PyResult<Vec<Py<PyAny>>>>()?;
        Ok(PyList::new(py, values)?.into_any().unbind())
    }

    /// Return the median value.
    fn median(&self) -> PyResult<Py<PyAny>> {
        Python::attach(|py| {
            let r = self
                .inner
                .median()
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
            scalar_to_py(py, &r)
        })
    }

    /// Return the (sample) variance.
    fn var(&self) -> PyResult<Py<PyAny>> {
        Python::attach(|py| {
            let r = self
                .inner
                .var()
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
            scalar_to_py(py, &r)
        })
    }

    /// Return the product of the values.
    fn prod(&self) -> PyResult<Py<PyAny>> {
        Python::attach(|py| {
            let r = self
                .inner
                .prod()
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
            scalar_to_py(py, &r)
        })
    }

    /// Return the quantile at `q` in [0, 1].
    fn quantile(&self, q: f64) -> PyResult<Py<PyAny>> {
        Python::attach(|py| {
            let r = self
                .inner
                .quantile(q)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
            scalar_to_py(py, &r)
        })
    }

    /// Return the number of non-missing values.
    fn count(&self) -> usize {
        self.inner.count()
    }

    /// Return the number of distinct non-missing values.
    fn nunique(&self) -> usize {
        self.inner.nunique()
    }

    /// Return the sample skewness.
    fn skew(&self) -> PyResult<f64> {
        self.inner
            .skew()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))
    }

    /// Return the sample (excess) kurtosis.
    fn kurt(&self) -> PyResult<f64> {
        self.inner
            .kurt()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))
    }

    /// Return the absolute value of each element as a new Series.
    fn abs(&self) -> PyResult<PySeries> {
        let r = self
            .inner
            .abs()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PySeries { inner: r })
    }

    /// Return the cumulative sum as a new Series.
    fn cumsum(&self) -> PyResult<PySeries> {
        let r = self
            .inner
            .cumsum()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PySeries { inner: r })
    }

    /// Return counts of unique values (descending) as a new Series.
    fn value_counts(&self) -> PyResult<PySeries> {
        let r = self
            .inner
            .value_counts()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PySeries { inner: r })
    }

    /// Return the distinct values as a Python list (order of first appearance).
    fn unique(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let values: Vec<Py<PyAny>> = self
            .inner
            .unique()
            .iter()
            .map(|s| scalar_to_py(py, s))
            .collect::<PyResult<Vec<Py<PyAny>>>>()?;
        Ok(PyList::new(py, values)?.into_any().unbind())
    }

    /// Sort the Series by value, returning a new Series.
    #[pyo3(signature = (ascending=true))]
    fn sort_values(&self, ascending: bool) -> PyResult<PySeries> {
        let r = self
            .inner
            .sort_values(ascending)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PySeries { inner: r })
    }

    /// Fill missing values with `value`, returning a new Series.
    fn fillna(&self, py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<PySeries> {
        let fill = py_to_scalar(py, value)?;
        let r = self
            .inner
            .fillna(&fill)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PySeries { inner: r })
    }

    /// Drop missing values, returning a new Series.
    fn dropna(&self) -> PyResult<PySeries> {
        let r = self
            .inner
            .dropna()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PySeries { inner: r })
    }

    /// Return the cumulative product as a new Series.
    fn cumprod(&self) -> PyResult<PySeries> {
        let r = self
            .inner
            .cumprod()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PySeries { inner: r })
    }

    /// Return the cumulative minimum as a new Series.
    fn cummin(&self) -> PyResult<PySeries> {
        let r = self
            .inner
            .cummin()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PySeries { inner: r })
    }

    /// Return the cumulative maximum as a new Series.
    fn cummax(&self) -> PyResult<PySeries> {
        let r = self
            .inner
            .cummax()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PySeries { inner: r })
    }

    /// Return the discrete first difference over `periods`.
    #[pyo3(signature = (periods=1))]
    fn diff(&self, periods: i64) -> PyResult<PySeries> {
        let r = self
            .inner
            .diff(periods)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PySeries { inner: r })
    }

    /// Return the fractional change over `periods`.
    #[pyo3(signature = (periods=1))]
    fn pct_change(&self, periods: i64) -> PyResult<PySeries> {
        let r = self
            .inner
            .pct_change(periods)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PySeries { inner: r })
    }

    /// Round each value to `decimals` places, returning a new Series.
    #[pyo3(signature = (decimals=0))]
    fn round(&self, decimals: i32) -> PyResult<PySeries> {
        let r = self
            .inner
            .round(decimals)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PySeries { inner: r })
    }

    /// Compute numerical data ranks (pandas `Series.rank`).
    #[pyo3(signature = (method="average", ascending=true, na_option="keep"))]
    fn rank(&self, method: &str, ascending: bool, na_option: &str) -> PyResult<PySeries> {
        let r = self
            .inner
            .rank(method, ascending, na_option)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PySeries { inner: r })
    }
}

/// Python wrapper for FrankenPandas DataFrame.
#[pyclass(name = "DataFrame", from_py_object)]
#[derive(Clone)]
pub struct PyDataFrame {
    inner: DataFrame,
}

#[pymethods]
impl PyDataFrame {
    /// Create a new DataFrame from a dictionary of column name -> values.
    #[new]
    #[pyo3(signature = (data))]
    fn new(py: Python<'_>, data: &Bound<'_, PyDict>) -> PyResult<Self> {
        let mut columns = BTreeMap::new();
        let mut column_order = Vec::new();
        let mut n_rows = 0usize;

        for (key, value) in data.iter() {
            let col_name: String = key.extract()?;
            let values: &Bound<'_, PyList> = value.cast()?;

            let scalars: Vec<Scalar> = values
                .iter()
                .map(|v| py_to_scalar(py, &v))
                .collect::<PyResult<Vec<_>>>()?;

            if n_rows == 0 {
                n_rows = scalars.len();
            } else if scalars.len() != n_rows {
                return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
                    "All columns must have the same length",
                ));
            }

            let column = Column::from_values(scalars)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;

            column_order.push(col_name.clone());
            columns.insert(col_name, column);
        }

        let labels: Vec<IndexLabel> = (0..n_rows).map(|i| IndexLabel::Int64(i as i64)).collect();
        let index = Index::new(labels);

        let df = DataFrame::new_with_column_order(index, columns, column_order)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;

        Ok(PyDataFrame { inner: df })
    }

    /// Return the shape of the DataFrame as (rows, cols).
    #[getter]
    fn shape(&self) -> (usize, usize) {
        (self.inner.len(), self.inner.columns().len())
    }

    /// Return the column names.
    #[getter]
    fn columns(&self) -> Vec<String> {
        self.inner.columns().keys().cloned().collect()
    }

    /// Return the number of rows.
    fn __len__(&self) -> usize {
        self.inner.len()
    }

    /// Return a string representation (renders the table, like pandas;
    /// the underlying Display truncates to 60 rows).
    fn __repr__(&self) -> String {
        format!("{}", self.inner)
    }

    /// Return the first n rows.
    fn head(&self, n: Option<i64>) -> PyResult<PyDataFrame> {
        let n = n.unwrap_or(5);
        let result = self
            .inner
            .head(n)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PyDataFrame { inner: result })
    }

    /// Return the last n rows.
    fn tail(&self, n: Option<i64>) -> PyResult<PyDataFrame> {
        let n = n.unwrap_or(5);
        let result = self
            .inner
            .tail(n)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PyDataFrame { inner: result })
    }

    /// Return a column as a Series.
    fn __getitem__(&self, col: &str) -> PyResult<PySeries> {
        let series = self.inner.get_column(col);
        Ok(PySeries { inner: series })
    }

    /// Return summary statistics.
    fn describe(&self) -> PyResult<PyDataFrame> {
        let result = self
            .inner
            .describe()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PyDataFrame { inner: result })
    }

    /// Return the sum of each column.
    fn sum(&self) -> PyResult<PySeries> {
        let result = self
            .inner
            .sum()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PySeries { inner: result })
    }

    /// Return the mean of each column.
    fn mean(&self) -> PyResult<PySeries> {
        let result = self
            .inner
            .mean()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PySeries { inner: result })
    }

    /// Return the median of each column.
    fn median(&self) -> PyResult<PySeries> {
        let result = self
            .inner
            .median()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PySeries { inner: result })
    }

    /// Return the standard deviation of each column.
    fn std(&self) -> PyResult<PySeries> {
        let result = self
            .inner
            .std()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PySeries { inner: result })
    }

    /// Return the variance of each column.
    fn var(&self) -> PyResult<PySeries> {
        let result = self
            .inner
            .var()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PySeries { inner: result })
    }

    /// Return the count of non-missing values per column.
    fn count(&self) -> PyResult<PySeries> {
        let result = self
            .inner
            .count()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PySeries { inner: result })
    }

    /// Return the minimum of each column.
    fn min(&self) -> PyResult<PySeries> {
        let result = self
            .inner
            .min()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PySeries { inner: result })
    }

    /// Return the maximum of each column.
    fn max(&self) -> PyResult<PySeries> {
        let result = self
            .inner
            .max()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PySeries { inner: result })
    }

    /// Return the column-pair correlation matrix as a DataFrame.
    fn corr(&self) -> PyResult<PyDataFrame> {
        let result = self
            .inner
            .corr()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PyDataFrame { inner: result })
    }

    /// Fill missing values with `value`, returning a new DataFrame.
    fn fillna(&self, py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<PyDataFrame> {
        let fill = py_to_scalar(py, value)?;
        let result = self
            .inner
            .fillna(&fill)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PyDataFrame { inner: result })
    }

    /// Drop rows containing any missing value, returning a new DataFrame.
    fn dropna(&self) -> PyResult<PyDataFrame> {
        let result = self
            .inner
            .dropna()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PyDataFrame { inner: result })
    }

    /// Reset the index to a default integer range, returning a new DataFrame.
    #[pyo3(signature = (drop=false))]
    fn reset_index(&self, drop: bool) -> PyResult<PyDataFrame> {
        let result = self
            .inner
            .reset_index(drop)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PyDataFrame { inner: result })
    }

    /// Sort by a column.
    fn sort_values(&self, by: &str, ascending: Option<bool>) -> PyResult<PyDataFrame> {
        let asc = ascending.unwrap_or(true);
        let result = self
            .inner
            .sort_values(by, asc)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PyDataFrame { inner: result })
    }

    /// Drop duplicate rows.
    fn drop_duplicates(&self) -> PyResult<PyDataFrame> {
        let result = self
            .inner
            .drop_duplicates(None, fp_index::DuplicateKeep::First, false)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PyDataFrame { inner: result })
    }

    /// Group by columns.
    fn groupby(&self, by: Vec<String>) -> PyResult<PyGroupBy> {
        let by_refs: Vec<&str> = by.iter().map(|s| s.as_str()).collect();
        let _gb = self
            .inner
            .groupby(&by_refs)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PyGroupBy {
            df: self.inner.clone(),
            by,
        })
    }

    /// Export to CSV. With no `path`, returns the CSV string; with a `path`,
    /// writes the file and returns `None` (pandas `DataFrame.to_csv`).
    #[pyo3(signature = (path=None, index=false))]
    fn to_csv(&self, path: Option<&str>, index: bool) -> PyResult<Option<String>> {
        let csv = self.inner.to_csv(',', index);
        match path {
            Some(p) => {
                std::fs::write(p, csv)
                    .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
                Ok(None)
            }
            None => Ok(Some(csv)),
        }
    }

    /// Export to a column-oriented dict `{column: [values]}` (pandas
    /// `DataFrame.to_dict(orient="list")`).
    fn to_dict(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let out = PyDict::new(py);
        for name in self.inner.column_names() {
            let col = self.inner.column(name).ok_or_else(|| {
                PyErr::new::<pyo3::exceptions::PyKeyError, _>(format!("column {name:?} missing"))
            })?;
            let values: Vec<Py<PyAny>> = col
                .values()
                .iter()
                .map(|s| scalar_to_py(py, s))
                .collect::<PyResult<Vec<_>>>()?;
            out.set_item(name, PyList::new(py, values)?)?;
        }
        Ok(out.into_any().unbind())
    }

    /// Render the DataFrame as an HTML table (pandas `DataFrame.to_html`).
    #[pyo3(signature = (index=true))]
    fn to_html(&self, index: bool) -> String {
        self.inner.to_html(index)
    }

    /// Render the DataFrame as a GitHub-flavored Markdown table
    /// (pandas `DataFrame.to_markdown`).
    #[pyo3(signature = (index=true))]
    fn to_markdown(&self, index: bool) -> PyResult<String> {
        self.inner
            .to_markdown(index, None)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))
    }

    /// Render the DataFrame as a plain-text table (pandas `DataFrame.to_string`).
    #[pyo3(signature = (index=true))]
    fn to_string(&self, index: bool) -> String {
        self.inner.to_string_table(index)
    }

    /// Return a chainable Styler for HTML formatting (pandas `DataFrame.style`).
    fn style(&self) -> PyStyler {
        PyStyler {
            df: self.inner.clone(),
            ops: Vec::new(),
        }
    }
}

/// Recorded Styler directive, replayed onto a fresh `StyledDataFrame` at
/// render time (the Rust Styler borrows its DataFrame, so the Python wrapper
/// owns a clone and replays the chain instead of holding the borrow).
#[derive(Clone)]
enum StyleOp {
    HighlightMax(String),
    HighlightMin(String),
    BackgroundGradient(String, String),
    Format(String),
    NaRep(String),
    SetCaption(String),
    SetProperties(Vec<(String, String)>),
    Bar(String),
    HideIndex,
}

/// Python wrapper for FrankenPandas DataFrame.style (Styler).
///
/// Builder methods return a new `Styler` so the chain composes exactly like
/// pandas: `df.style().highlight_max("yellow").format("{:.2f}").to_html()`.
#[pyclass(name = "Styler", from_py_object)]
#[derive(Clone)]
pub struct PyStyler {
    df: DataFrame,
    ops: Vec<StyleOp>,
}

impl PyStyler {
    fn with_op(&self, op: StyleOp) -> PyStyler {
        let mut next = self.clone();
        next.ops.push(op);
        next
    }
}

#[pymethods]
impl PyStyler {
    fn __repr__(&self) -> String {
        format!("Styler(directives={})", self.ops.len())
    }

    /// Highlight the per-column maximum cell(s) with `color`.
    fn highlight_max(&self, color: &str) -> PyStyler {
        self.with_op(StyleOp::HighlightMax(color.to_owned()))
    }

    /// Highlight the per-column minimum cell(s) with `color`.
    fn highlight_min(&self, color: &str) -> PyStyler {
        self.with_op(StyleOp::HighlightMin(color.to_owned()))
    }

    /// Shade numeric cells along a two-colour `#rrggbb` gradient.
    fn background_gradient(&self, low: &str, high: &str) -> PyStyler {
        self.with_op(StyleOp::BackgroundGradient(low.to_owned(), high.to_owned()))
    }

    /// Apply a Python-style numeric format spec, e.g. `"{:.2f}"`.
    fn format(&self, fmt: &str) -> PyStyler {
        self.with_op(StyleOp::Format(fmt.to_owned()))
    }

    /// Render missing/NaN cells with `placeholder` instead of `"NaN"`.
    fn na_rep(&self, placeholder: &str) -> PyStyler {
        self.with_op(StyleOp::NaRep(placeholder.to_owned()))
    }

    /// Set the table `<caption>`.
    fn set_caption(&self, caption: &str) -> PyStyler {
        self.with_op(StyleOp::SetCaption(caption.to_owned()))
    }

    /// Apply fixed CSS `{property: value}` pairs to every data cell.
    fn set_properties(&self, props: &Bound<'_, PyDict>) -> PyResult<PyStyler> {
        let mut pairs: Vec<(String, String)> = Vec::with_capacity(props.len());
        for (k, v) in props.iter() {
            pairs.push((k.extract::<String>()?, v.extract::<String>()?));
        }
        Ok(self.with_op(StyleOp::SetProperties(pairs)))
    }

    /// Draw an in-cell bar chart in each numeric cell.
    fn bar(&self, color: &str) -> PyStyler {
        self.with_op(StyleOp::Bar(color.to_owned()))
    }

    /// Omit the index column/header from the HTML render.
    fn hide_index(&self) -> PyStyler {
        self.with_op(StyleOp::HideIndex)
    }

    /// Render the styled table as HTML (pandas `Styler.to_html`).
    #[pyo3(signature = (index=true))]
    fn to_html(&self, index: bool) -> String {
        let mut styled = self.df.style();
        for op in &self.ops {
            styled = match op {
                StyleOp::HighlightMax(c) => styled.highlight_max(c),
                StyleOp::HighlightMin(c) => styled.highlight_min(c),
                StyleOp::BackgroundGradient(lo, hi) => styled.background_gradient(lo, hi),
                StyleOp::Format(f) => styled.format(f),
                StyleOp::NaRep(n) => styled.na_rep(n),
                StyleOp::SetCaption(c) => styled.set_caption(c),
                StyleOp::SetProperties(pairs) => {
                    let refs: Vec<(&str, &str)> =
                        pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
                    styled.set_properties(&refs)
                }
                StyleOp::Bar(c) => styled.bar(c),
                StyleOp::HideIndex => styled.hide_index(),
            };
        }
        styled.to_html(index)
    }
}

/// Python wrapper for FrankenPandas GroupBy.
#[pyclass(name = "DataFrameGroupBy")]
pub struct PyGroupBy {
    df: DataFrame,
    by: Vec<String>,
}

#[pymethods]
impl PyGroupBy {
    fn __repr__(&self) -> String {
        format!("DataFrameGroupBy(by={:?})", self.by)
    }

    fn sum(&self) -> PyResult<PyDataFrame> {
        let by_refs: Vec<&str> = self.by.iter().map(|s| s.as_str()).collect();
        let result = self
            .df
            .groupby(&by_refs)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?
            .sum()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PyDataFrame { inner: result })
    }

    fn mean(&self) -> PyResult<PyDataFrame> {
        let by_refs: Vec<&str> = self.by.iter().map(|s| s.as_str()).collect();
        let result = self
            .df
            .groupby(&by_refs)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?
            .mean()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PyDataFrame { inner: result })
    }

    fn count(&self) -> PyResult<PyDataFrame> {
        let by_refs: Vec<&str> = self.by.iter().map(|s| s.as_str()).collect();
        let result = self
            .df
            .groupby(&by_refs)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?
            .count()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PyDataFrame { inner: result })
    }

    fn min(&self) -> PyResult<PyDataFrame> {
        let by_refs: Vec<&str> = self.by.iter().map(|s| s.as_str()).collect();
        let result = self
            .df
            .groupby(&by_refs)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?
            .min()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PyDataFrame { inner: result })
    }

    fn max(&self) -> PyResult<PyDataFrame> {
        let by_refs: Vec<&str> = self.by.iter().map(|s| s.as_str()).collect();
        let result = self
            .df
            .groupby(&by_refs)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?
            .max()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PyDataFrame { inner: result })
    }

    fn var(&self) -> PyResult<PyDataFrame> {
        let by_refs: Vec<&str> = self.by.iter().map(|s| s.as_str()).collect();
        let result = self
            .df
            .groupby(&by_refs)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?
            .var()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PyDataFrame { inner: result })
    }

    fn std(&self) -> PyResult<PyDataFrame> {
        let by_refs: Vec<&str> = self.by.iter().map(|s| s.as_str()).collect();
        let result = self
            .df
            .groupby(&by_refs)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?
            .std()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PyDataFrame { inner: result })
    }

    fn median(&self) -> PyResult<PyDataFrame> {
        let by_refs: Vec<&str> = self.by.iter().map(|s| s.as_str()).collect();
        let result = self
            .df
            .groupby(&by_refs)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?
            .median()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PyDataFrame { inner: result })
    }

    fn prod(&self) -> PyResult<PyDataFrame> {
        let by_refs: Vec<&str> = self.by.iter().map(|s| s.as_str()).collect();
        let result = self
            .df
            .groupby(&by_refs)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?
            .prod()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PyDataFrame { inner: result })
    }
}

/// Read a CSV file into a DataFrame.
#[pyfunction]
fn read_csv(path: &str) -> PyResult<PyDataFrame> {
    let path = std::path::Path::new(path);
    let df = fp_io::read_csv(path)
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
    Ok(PyDataFrame { inner: df })
}

/// Map a pandas `orient=` string to the fp-io `JsonOrient` enum.
fn parse_json_orient(orient: &str) -> PyResult<fp_io::JsonOrient> {
    match orient {
        "records" => Ok(fp_io::JsonOrient::Records),
        "columns" => Ok(fp_io::JsonOrient::Columns),
        "index" => Ok(fp_io::JsonOrient::Index),
        "split" => Ok(fp_io::JsonOrient::Split),
        "values" => Ok(fp_io::JsonOrient::Values),
        other => Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
            "unknown JSON orient {other:?}; expected one of records/columns/index/split/values"
        ))),
    }
}

/// Read a JSON file into a DataFrame (pandas `read_json`). `orient` is one of
/// records/columns/index/split/values.
#[pyfunction]
#[pyo3(signature = (path, orient="records"))]
fn read_json(path: &str, orient: &str) -> PyResult<PyDataFrame> {
    let orient = parse_json_orient(orient)?;
    let df = fp_io::read_json(std::path::Path::new(path), orient)
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
    Ok(PyDataFrame { inner: df })
}

/// Read a line-delimited JSON file into a DataFrame (pandas
/// `read_json(lines=True)`).
#[pyfunction]
fn read_jsonl(path: &str) -> PyResult<PyDataFrame> {
    let df = fp_io::read_jsonl(std::path::Path::new(path))
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
    Ok(PyDataFrame { inner: df })
}

/// Read a Parquet file into a DataFrame (pandas `read_parquet`).
#[pyfunction]
fn read_parquet(path: &str) -> PyResult<PyDataFrame> {
    let df = fp_io::read_parquet(std::path::Path::new(path))
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
    Ok(PyDataFrame { inner: df })
}

/// FrankenPandas Python module.
#[pymodule]
fn frankenpandas(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PySeries>()?;
    m.add_class::<PyDataFrame>()?;
    m.add_class::<PyGroupBy>()?;
    m.add_class::<PyStyler>()?;
    m.add_function(wrap_pyfunction!(read_csv, m)?)?;
    m.add_function(wrap_pyfunction!(read_json, m)?)?;
    m.add_function(wrap_pyfunction!(read_jsonl, m)?)?;
    m.add_function(wrap_pyfunction!(read_parquet, m)?)?;
    Ok(())
}
