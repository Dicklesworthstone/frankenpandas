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
        Scalar::Period(ordinal) => ordinal.into_py_any(py),
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

    /// Return a string representation.
    fn __repr__(&self) -> String {
        format!("Series('{}', len={})", self.inner.name(), self.inner.len())
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

    /// Return a string representation.
    fn __repr__(&self) -> String {
        format!(
            "DataFrame(shape=({}, {}))",
            self.inner.len(),
            self.inner.columns().len()
        )
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

    /// Export to CSV string.
    fn to_csv(&self) -> String {
        self.inner.to_csv(',', false)
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
}

/// Read a CSV file into a DataFrame.
#[pyfunction]
fn read_csv(path: &str) -> PyResult<PyDataFrame> {
    let path = std::path::Path::new(path);
    let df = fp_io::read_csv(path)
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
    Ok(PyDataFrame { inner: df })
}

/// FrankenPandas Python module.
#[pymodule]
fn frankenpandas(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PySeries>()?;
    m.add_class::<PyDataFrame>()?;
    m.add_class::<PyGroupBy>()?;
    m.add_function(wrap_pyfunction!(read_csv, m)?)?;
    Ok(())
}
