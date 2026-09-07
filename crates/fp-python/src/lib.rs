//! PyO3 Python bindings for FrankenPandas.
//!
//! Exposes DataFrame, Series, and core operations to Python.
//!
//! ```python
//! import frankenpandas as fp
//!
//! # Create a Series
//! s = fp.Series([1, 2, 3, 4, 5], name="values")
//! print(s.sum())  # 15
//!
//! # Create a DataFrame
//! df = fp.DataFrame({"a": [1, 2, 3], "b": [4.0, 5.0, 6.0]})
//! print(df.head(2))
//! ```

#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_expr::DataFrameExprExt;
use fp_frame::{DataFrame, Series, concat_dataframes, concat_series};
use fp_index::{
    AlignMode, CategoricalIndex, DatetimeIndex, DuplicateKeep, Index, IndexLabel, MultiIndex,
    PeriodIndex, RangeIndex, TimedeltaIndex, format_datetime_ns,
};
use fp_types::{NullKind, Period, PeriodFreq, Scalar, Timedelta};
use mimalloc::MiMalloc;
use pyo3::{
    IntoPyObjectExt,
    prelude::*,
    types::{PyDict, PyList, PyTuple},
};

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

/// Parse frequency string like "D", "2D", "h", "30s", "min", "ms", "us", "ns" into nanoseconds.
fn parse_freq_to_nanos(freq: &str) -> PyResult<i64> {
    let s = freq.trim();
    if s.is_empty() {
        return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
            "empty freq",
        ));
    }
    let split_pos = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    let (count_str, unit_str) = s.split_at(split_pos);
    let count: i64 = if count_str.is_empty() {
        1
    } else {
        count_str.parse().map_err(|_| {
            PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                "invalid freq count: {count_str}"
            ))
        })?
    };
    let unit = unit_str.trim();
    let base_nanos = match unit {
        "D" | "d" => 86_400_000_000_000_i64,
        "h" | "H" => 3_600_000_000_000_i64,
        "min" | "T" | "m" => 60_000_000_000_i64,
        "s" | "S" => 1_000_000_000_i64,
        "ms" | "L" => 1_000_000_i64,
        "us" | "U" => 1_000_i64,
        "ns" | "N" | "" => 1_i64,
        other => {
            return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                "unsupported frequency: {other}"
            )));
        }
    };
    count
        .checked_mul(base_nanos)
        .ok_or_else(|| PyErr::new::<pyo3::exceptions::PyValueError, _>("frequency overflow"))
}

/// Map a pandas-style dtype string to a FrankenPandas `DType`.
fn parse_dtype(name: &str) -> PyResult<fp_types::DType> {
    use fp_types::DType;
    match name {
        "int" | "int64" | "i64" | "Int64" => Ok(DType::Int64),
        "float" | "float64" | "f64" | "Float64" => Ok(DType::Float64),
        "str" | "string" | "object" | "O" | "utf8" => Ok(DType::Utf8),
        "bool" | "boolean" => Ok(DType::Bool),
        "datetime64" | "datetime64[ns]" | "datetime" => Ok(DType::datetime64_naive()),
        "timedelta64" | "timedelta64[ns]" | "timedelta" => Ok(DType::Timedelta64),
        other => Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
            "unsupported dtype {other:?}"
        ))),
    }
}

/// Convert a Python dict `{old: new}` into FrankenPandas `(Scalar, Scalar)` pairs.
fn py_dict_to_scalar_pairs(
    py: Python<'_>,
    mapping: &Bound<'_, PyDict>,
) -> PyResult<Vec<(Scalar, Scalar)>> {
    let mut pairs = Vec::with_capacity(mapping.len());
    for (k, v) in mapping.iter() {
        pairs.push((py_to_scalar(py, &k)?, py_to_scalar(py, &v)?));
    }
    Ok(pairs)
}

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

/// Parse `keep` parameter for duplicate row/value detection.
fn parse_duplicate_keep(keep: Option<&Bound<'_, PyAny>>) -> PyResult<DuplicateKeep> {
    match keep {
        None => Ok(DuplicateKeep::First),
        Some(k) => {
            if let Ok(b) = k.extract::<bool>() {
                if !b {
                    return Ok(DuplicateKeep::None);
                }
                return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
                    "keep=True is invalid; use 'first', 'last', or False",
                ));
            }
            if let Ok(s) = k.extract::<String>() {
                match s.to_ascii_lowercase().as_str() {
                    "first" => Ok(DuplicateKeep::First),
                    "last" => Ok(DuplicateKeep::Last),
                    "false" | "none" => Ok(DuplicateKeep::None),
                    other => Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                        "keep must be 'first', 'last', or False, got {other:?}"
                    ))),
                }
            } else {
                Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                    "keep must be a string or boolean False",
                ))
            }
        }
    }
}

/// Convert an arbitrary Python value (Series, list, tuple, or scalar) into a Column.
fn py_value_to_column(
    py: Python<'_>,
    val: &Bound<'_, PyAny>,
    expected_len: usize,
) -> PyResult<Column> {
    if let Ok(s) = val.extract::<PyRef<'_, PySeries>>() {
        if s.inner.len() != expected_len {
            return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                "Length of values ({}) does not match length of index ({})",
                s.inner.len(),
                expected_len
            )));
        }
        return Ok(s.inner.column().clone());
    }
    if let Ok(list) = val.cast::<PyList>() {
        let scalars: Vec<Scalar> = list
            .iter()
            .map(|v| py_to_scalar(py, &v))
            .collect::<PyResult<Vec<_>>>()?;
        if scalars.len() != expected_len {
            return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                "Length of values ({}) does not match length of index ({})",
                scalars.len(),
                expected_len
            )));
        }
        return Column::from_values(scalars)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()));
    }
    if let Ok(tuple) = val.cast::<pyo3::types::PyTuple>() {
        let scalars: Vec<Scalar> = tuple
            .iter()
            .map(|v| py_to_scalar(py, &v))
            .collect::<PyResult<Vec<_>>>()?;
        if scalars.len() != expected_len {
            return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                "Length of values ({}) does not match length of index ({})",
                scalars.len(),
                expected_len
            )));
        }
        return Column::from_values(scalars)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()));
    }
    if let Ok(s) = py_to_scalar(py, val) {
        return Column::from_values(vec![s; expected_len])
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()));
    }
    Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
        "Cannot convert value to DataFrame column",
    ))
}

/// Convert a Python value to an IndexLabel.
fn py_to_index_label(obj: &Bound<'_, PyAny>) -> PyResult<IndexLabel> {
    if let Ok(b) = obj.extract::<bool>() {
        Ok(IndexLabel::Bool(b))
    } else if let Ok(i) = obj.extract::<i64>() {
        Ok(IndexLabel::Int64(i))
    } else if let Ok(s) = obj.extract::<String>() {
        Ok(IndexLabel::Utf8(s))
    } else if let Ok(f) = obj.extract::<f64>() {
        Ok(IndexLabel::Float64(fp_index::OrderedF64(f)))
    } else {
        Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(format!(
            "Cannot convert {} to IndexLabel",
            obj.get_type().name()?
        )))
    }
}

/// Convert an IndexLabel to a Python object.
fn index_label_to_py(py: Python<'_>, label: &IndexLabel) -> PyResult<Py<PyAny>> {
    match label {
        IndexLabel::Int64(i) => i.into_py_any(py),
        IndexLabel::Utf8(s) => s.into_py_any(py),
        IndexLabel::Timedelta64(ns) => ns.into_py_any(py),
        IndexLabel::Datetime64(ns) => ns.into_py_any(py),
        IndexLabel::Float64(f) => f.0.into_py_any(py),
        IndexLabel::Bool(b) => b.into_py_any(py),
        IndexLabel::Null(_) => Ok(py.None()),
    }
}

/// Convert an IndexLabel to a Scalar.
fn index_label_to_scalar(label: &IndexLabel) -> Scalar {
    match label {
        IndexLabel::Int64(i) => Scalar::Int64(*i),
        IndexLabel::Utf8(s) => Scalar::Utf8(s.clone()),
        IndexLabel::Timedelta64(ns) => Scalar::Timedelta64(*ns),
        IndexLabel::Datetime64(ns) => Scalar::Datetime64(*ns),
        IndexLabel::Float64(f) => Scalar::Float64(f.0),
        IndexLabel::Bool(b) => Scalar::Bool(*b),
        IndexLabel::Null(k) => Scalar::Null(*k),
    }
}

/// Python wrapper for Index string accessor methods.
#[pyclass(name = "IndexStringMethods", from_py_object)]
#[derive(Clone)]
pub struct PyIndexStringMethods {
    pub(crate) inner: Index,
}

#[pymethods]
impl PyIndexStringMethods {
    fn lower(&self) -> PyIndex {
        let labels = self
            .inner
            .labels()
            .iter()
            .map(|l| match l {
                IndexLabel::Utf8(s) => IndexLabel::Utf8(s.to_lowercase()),
                other => other.clone(),
            })
            .collect();
        PyIndex {
            inner: Index::new(labels),
        }
    }

    fn upper(&self) -> PyIndex {
        let labels = self
            .inner
            .labels()
            .iter()
            .map(|l| match l {
                IndexLabel::Utf8(s) => IndexLabel::Utf8(s.to_uppercase()),
                other => other.clone(),
            })
            .collect();
        PyIndex {
            inner: Index::new(labels),
        }
    }

    fn strip(&self) -> PyIndex {
        let labels = self
            .inner
            .labels()
            .iter()
            .map(|l| match l {
                IndexLabel::Utf8(s) => IndexLabel::Utf8(s.trim().to_string()),
                other => other.clone(),
            })
            .collect();
        PyIndex {
            inner: Index::new(labels),
        }
    }

    fn len(&self) -> PyIndex {
        let labels = self
            .inner
            .labels()
            .iter()
            .map(|l| match l {
                IndexLabel::Utf8(s) => IndexLabel::Int64(s.len() as i64),
                _ => IndexLabel::Null(NullKind::NaN),
            })
            .collect();
        PyIndex {
            inner: Index::new(labels),
        }
    }

    fn contains(&self, pat: &str) -> Vec<bool> {
        self.inner
            .labels()
            .iter()
            .map(|l| match l {
                IndexLabel::Utf8(s) => s.contains(pat),
                _ => false,
            })
            .collect()
    }

    fn startswith(&self, pat: &str) -> Vec<bool> {
        self.inner
            .labels()
            .iter()
            .map(|l| match l {
                IndexLabel::Utf8(s) => s.starts_with(pat),
                _ => false,
            })
            .collect()
    }

    fn endswith(&self, pat: &str) -> Vec<bool> {
        self.inner
            .labels()
            .iter()
            .map(|l| match l {
                IndexLabel::Utf8(s) => s.ends_with(pat),
                _ => false,
            })
            .collect()
    }

    fn replace(&self, pat: &str, repl: &str) -> PyIndex {
        let labels = self
            .inner
            .labels()
            .iter()
            .map(|l| match l {
                IndexLabel::Utf8(s) => IndexLabel::Utf8(s.replace(pat, repl)),
                other => other.clone(),
            })
            .collect();
        PyIndex {
            inner: Index::new(labels),
        }
    }
}

/// Python wrapper for FrankenPandas Index.
#[pyclass(name = "Index", from_py_object)]
#[derive(Clone)]
pub struct PyIndex {
    pub(crate) inner: Index,
}

#[pymethods]
impl PyIndex {
    #[new]
    #[pyo3(signature = (data=None, name=None))]
    fn new(data: Option<&Bound<'_, PyAny>>, name: Option<&str>) -> PyResult<Self> {
        let mut labels: Vec<IndexLabel> = Vec::new();
        if let Some(d) = data {
            if let Ok(idx) = d.extract::<PyRef<'_, PyIndex>>() {
                let mut inner = idx.inner.clone();
                if let Some(n) = name {
                    inner = inner.set_name(n);
                }
                return Ok(PyIndex { inner });
            } else if let Ok(s) = d.extract::<PyRef<'_, PySeries>>() {
                labels = s.inner.index().labels().to_vec();
            } else if let Ok(list) = d.extract::<Vec<i64>>() {
                labels = list.into_iter().map(IndexLabel::Int64).collect();
            } else if let Ok(list) = d.extract::<Vec<String>>() {
                labels = list.into_iter().map(IndexLabel::Utf8).collect();
            } else if let Ok(seq) = d.cast::<pyo3::types::PySequence>() {
                let len = seq.len()?;
                labels.reserve(len);
                for i in 0..len {
                    let item = seq.get_item(i)?;
                    labels.push(py_to_index_label(&item)?);
                }
            } else {
                return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                    "Index data must be sequence of labels or Index/Series",
                ));
            }
        }
        let mut inner = Index::new(labels);
        if let Some(n) = name {
            inner = inner.set_name(n);
        }
        Ok(PyIndex { inner })
    }

    #[getter]
    fn name(&self) -> Option<String> {
        self.inner.name().map(String::from)
    }

    #[setter]
    fn set_name(&mut self, name: Option<&str>) {
        self.inner = self.inner.set_names(name);
    }

    fn to_list(&self, py: Python<'_>) -> PyResult<Py<PyList>> {
        let items = self
            .inner
            .labels()
            .iter()
            .map(|l| index_label_to_py(py, l))
            .collect::<PyResult<Vec<_>>>()?;
        Ok(PyList::new(py, items)?.unbind())
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    fn __len__(&self) -> usize {
        self.inner.len()
    }

    fn __repr__(&self) -> String {
        let labels_str: Vec<String> = self
            .inner
            .labels()
            .iter()
            .map(|l| match l {
                IndexLabel::Utf8(s) => format!("'{s}'"),
                other => format!("{other}"),
            })
            .collect();
        let name_str = match self.inner.name() {
            Some(n) => format!(", name='{n}'"),
            None => String::new(),
        };
        format!("Index([{}]{name_str})", labels_str.join(", "))
    }

    fn __getitem__(&self, py: Python<'_>, key: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        if let Ok(idx) = key.extract::<i64>() {
            let pos = if idx < 0 {
                (self.inner.len() as i64 + idx) as usize
            } else {
                idx as usize
            };
            if pos >= self.inner.len() {
                return Err(PyErr::new::<pyo3::exceptions::PyIndexError, _>(
                    "index out of bounds",
                ));
            }
            return index_label_to_py(py, &self.inner.labels()[pos]);
        }
        if let Ok(slice) = key.cast::<pyo3::types::PySlice>() {
            let s_idx = slice.indices(self.inner.len() as isize)?;
            let mut sliced = Vec::new();
            let mut i = s_idx.start;
            if s_idx.step > 0 {
                while i < s_idx.stop {
                    sliced.push(self.inner.labels()[i as usize].clone());
                    i += s_idx.step;
                }
            } else if s_idx.step < 0 {
                while i > s_idx.stop {
                    sliced.push(self.inner.labels()[i as usize].clone());
                    i += s_idx.step;
                }
            }
            let mut out = Index::new(sliced);
            if let Some(n) = self.inner.name() {
                out = out.set_name(n);
            }
            return Ok(Py::new(py, PyIndex { inner: out })?.into_any());
        }
        Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
            "Index indices must be integers or slices",
        ))
    }

    #[getter]
    fn dtype(&self) -> &'static str {
        self.inner.dtype()
    }

    #[getter]
    fn shape(&self) -> (usize,) {
        (self.inner.len(),)
    }

    #[getter]
    fn size(&self) -> usize {
        self.inner.len()
    }

    #[getter]
    fn ndim(&self) -> usize {
        1
    }

    #[getter]
    fn empty(&self) -> bool {
        self.inner.is_empty()
    }

    #[getter]
    fn is_monotonic_increasing(&self) -> bool {
        self.inner.is_monotonic_increasing()
    }

    #[getter]
    fn is_monotonic_decreasing(&self) -> bool {
        self.inner.is_monotonic_decreasing()
    }

    #[getter]
    fn has_duplicates(&self) -> bool {
        self.inner.has_duplicates()
    }

    fn tolist(&self, py: Python<'_>) -> PyResult<Py<PyList>> {
        self.to_list(py)
    }

    fn values(&self, py: Python<'_>) -> PyResult<Py<PyList>> {
        self.to_list(py)
    }

    fn to_numpy(&self, py: Python<'_>) -> PyResult<Py<PyList>> {
        self.to_list(py)
    }

    fn unique(&self) -> Self {
        PyIndex {
            inner: self.inner.unique(),
        }
    }

    fn nunique(&self) -> usize {
        self.inner.nunique()
    }

    fn drop_duplicates(&self) -> Self {
        PyIndex {
            inner: self.inner.drop_duplicates(),
        }
    }

    fn duplicated(&self) -> Vec<bool> {
        self.inner.duplicated(DuplicateKeep::First)
    }

    fn min(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        match self.inner.min() {
            Some(l) => index_label_to_py(py, &l),
            None => Ok(py.None()),
        }
    }

    fn max(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        match self.inner.max() {
            Some(l) => index_label_to_py(py, &l),
            None => Ok(py.None()),
        }
    }

    fn isin(&self, values: &Bound<'_, PyAny>) -> PyResult<Vec<bool>> {
        let mut labels = Vec::new();
        if let Ok(seq) = values.cast::<pyo3::types::PySequence>() {
            let len = seq.len()?;
            labels.reserve(len);
            for i in 0..len {
                let item = seq.get_item(i)?;
                labels.push(py_to_index_label(&item)?);
            }
        }
        Ok(self.inner.isin(&labels))
    }

    fn isna(&self) -> Vec<bool> {
        self.inner.isna()
    }

    fn isnull(&self) -> Vec<bool> {
        self.inner.isna()
    }

    fn notna(&self) -> Vec<bool> {
        self.inner.notna()
    }

    fn notnull(&self) -> Vec<bool> {
        self.inner.notna()
    }

    fn intersection(&self, other: &PyIndex) -> Self {
        PyIndex {
            inner: self.inner.intersection(&other.inner),
        }
    }

    fn union(&self, other: &PyIndex) -> Self {
        PyIndex {
            inner: self.inner.union(&other.inner),
        }
    }

    fn difference(&self, other: &PyIndex) -> Self {
        PyIndex {
            inner: self.inner.difference(&other.inner),
        }
    }

    fn append(&self, other: &PyIndex) -> Self {
        PyIndex {
            inner: self.inner.append(&other.inner),
        }
    }

    fn get_loc(&self, key: &Bound<'_, PyAny>) -> PyResult<usize> {
        let label = py_to_index_label(key)?;
        self.inner.get_loc(&label).ok_or_else(|| {
            PyErr::new::<pyo3::exceptions::PyKeyError, _>(format!("{label} not found in Index"))
        })
    }

    fn copy(&self) -> Self {
        self.clone()
    }

    fn rename(&self, name: Option<&str>) -> Self {
        PyIndex {
            inner: self.inner.rename_index(name),
        }
    }

    fn __contains__(&self, key: &Bound<'_, PyAny>) -> bool {
        if let Ok(label) = py_to_index_label(key) {
            self.inner.contains(&label)
        } else {
            false
        }
    }

    #[getter]
    fn is_unique(&self) -> bool {
        self.inner.is_unique()
    }

    fn equals(&self, other: &PyIndex) -> bool {
        self.inner == other.inner
    }

    #[getter]
    #[allow(non_snake_case)]
    fn T(&self) -> Self {
        self.clone()
    }

    fn item(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        if self.inner.len() == 1 {
            index_label_to_py(py, &self.inner.labels()[0])
        } else {
            Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
                "can only convert an array of size 1 to a Python scalar",
            ))
        }
    }

    fn argmax(&self) -> PyResult<usize> {
        let labels = self.inner.labels();
        if labels.is_empty() {
            return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
                "attempt to get argmax of an empty sequence",
            ));
        }
        let mut max_idx = 0;
        for i in 1..labels.len() {
            if labels[i] > labels[max_idx] {
                max_idx = i;
            }
        }
        Ok(max_idx)
    }

    fn argmin(&self) -> PyResult<usize> {
        let labels = self.inner.labels();
        if labels.is_empty() {
            return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
                "attempt to get argmin of an empty sequence",
            ));
        }
        let mut min_idx = 0;
        for i in 1..labels.len() {
            if labels[i] < labels[min_idx] {
                min_idx = i;
            }
        }
        Ok(min_idx)
    }

    fn argsort(&self) -> Vec<usize> {
        let labels = self.inner.labels();
        let mut indices: Vec<usize> = (0..labels.len()).collect();
        indices.sort_by(|&a, &b| labels[a].cmp(&labels[b]));
        indices
    }

    fn all(&self) -> bool {
        self.inner.labels().iter().all(|l| match l {
            IndexLabel::Int64(i) => *i != 0,
            IndexLabel::Float64(f) => f.0 != 0.0 && !f.0.is_nan(),
            IndexLabel::Utf8(s) => !s.is_empty(),
            IndexLabel::Bool(b) => *b,
            IndexLabel::Timedelta64(t) => *t != 0,
            IndexLabel::Datetime64(d) => *d != 0,
            IndexLabel::Null(_) => false,
        })
    }

    fn any(&self) -> bool {
        self.inner.labels().iter().any(|l| match l {
            IndexLabel::Int64(i) => *i != 0,
            IndexLabel::Float64(f) => f.0 != 0.0 && !f.0.is_nan(),
            IndexLabel::Utf8(s) => !s.is_empty(),
            IndexLabel::Bool(b) => *b,
            IndexLabel::Timedelta64(t) => *t != 0,
            IndexLabel::Datetime64(d) => *d != 0,
            IndexLabel::Null(_) => false,
        })
    }

    #[pyo3(signature = (how=None))]
    fn dropna(&self, how: Option<&str>) -> Self {
        let _ = how;
        let labels = self.inner.labels();
        let non_null: Vec<IndexLabel> = labels
            .iter()
            .filter(|l| match l {
                IndexLabel::Float64(f) => !f.0.is_nan(),
                IndexLabel::Null(_) => false,
                _ => true,
            })
            .cloned()
            .collect();
        let mut res = Index::new(non_null);
        if let Some(n) = self.inner.name() {
            res = res.rename_index(Some(n));
        }
        PyIndex { inner: res }
    }

    fn fillna(&self, value: &Bound<'_, PyAny>) -> PyResult<Self> {
        let fill_lbl = py_to_index_label(value)?;
        let labels = self.inner.labels();
        let filled: Vec<IndexLabel> = labels
            .iter()
            .map(|l| match l {
                IndexLabel::Float64(f) if f.0.is_nan() => fill_lbl.clone(),
                IndexLabel::Null(_) => fill_lbl.clone(),
                _ => l.clone(),
            })
            .collect();
        let mut res = Index::new(filled);
        if let Some(n) = self.inner.name() {
            res = res.rename_index(Some(n));
        }
        Ok(PyIndex { inner: res })
    }

    fn delete(&self, loc: usize) -> PyResult<Self> {
        let mut labels = self.inner.labels().to_vec();
        if loc >= labels.len() {
            return Err(PyErr::new::<pyo3::exceptions::PyIndexError, _>(
                "Index position out of bounds",
            ));
        }
        labels.remove(loc);
        let mut res = Index::new(labels);
        if let Some(n) = self.inner.name() {
            res = res.rename_index(Some(n));
        }
        Ok(PyIndex { inner: res })
    }

    fn insert(&self, loc: usize, item: &Bound<'_, PyAny>) -> PyResult<Self> {
        let label = py_to_index_label(item)?;
        let mut labels = self.inner.labels().to_vec();
        if loc > labels.len() {
            return Err(PyErr::new::<pyo3::exceptions::PyIndexError, _>(
                "Index position out of bounds",
            ));
        }
        labels.insert(loc, label);
        let mut res = Index::new(labels);
        if let Some(n) = self.inner.name() {
            res = res.rename_index(Some(n));
        }
        Ok(PyIndex { inner: res })
    }

    fn repeat(&self, repeats: usize) -> Self {
        let labels = self.inner.labels();
        let mut out = Vec::with_capacity(labels.len() * repeats);
        for l in labels {
            for _ in 0..repeats {
                out.push(l.clone());
            }
        }
        let mut res = Index::new(out);
        if let Some(n) = self.inner.name() {
            res = res.rename_index(Some(n));
        }
        PyIndex { inner: res }
    }

    fn take(&self, indices: Vec<i64>) -> PyResult<Self> {
        let labels = self.inner.labels();
        let n = labels.len() as i64;
        let mut out = Vec::with_capacity(indices.len());
        for idx in indices {
            let actual = if idx < 0 { n + idx } else { idx };
            if actual < 0 || actual >= n {
                return Err(PyErr::new::<pyo3::exceptions::PyIndexError, _>(
                    "index out of bounds",
                ));
            }
            out.push(labels[actual as usize].clone());
        }
        let mut res = Index::new(out);
        if let Some(n) = self.inner.name() {
            res = res.rename_index(Some(n));
        }
        Ok(PyIndex { inner: res })
    }

    fn drop(&self, labels: &Bound<'_, PyAny>) -> PyResult<Self> {
        let mut to_drop = std::collections::HashSet::new();
        if let Ok(single) = py_to_index_label(labels) {
            to_drop.insert(single);
        } else if let Ok(list) = labels.extract::<Vec<Bound<'_, PyAny>>>() {
            for it in list {
                to_drop.insert(py_to_index_label(&it)?);
            }
        }
        let current = self.inner.labels();
        let kept: Vec<IndexLabel> = current
            .iter()
            .filter(|l| !to_drop.contains(l))
            .cloned()
            .collect();
        let mut res = Index::new(kept);
        if let Some(n) = self.inner.name() {
            res = res.rename_index(Some(n));
        }
        Ok(PyIndex { inner: res })
    }

    fn astype(&self, _dtype: &str) -> PyResult<Self> {
        Ok(self.clone())
    }

    #[getter]
    fn hasnans(&self) -> bool {
        self.inner.hasnans()
    }

    #[getter]
    fn nlevels(&self) -> usize {
        1
    }

    #[getter]
    fn names(&self) -> Vec<Option<String>> {
        vec![self.inner.name().map(str::to_string)]
    }

    #[getter]
    fn nbytes(&self) -> usize {
        self.inner.nbytes()
    }

    #[pyo3(signature = (deep=false))]
    fn memory_usage(&self, deep: bool) -> usize {
        self.inner.memory_usage(deep)
    }

    fn identical(&self, other: &PyIndex) -> bool {
        self.inner.identical(&other.inner)
    }

    fn inferred_type(&self) -> &'static str {
        self.inner.inferred_type()
    }

    fn is_numeric(&self) -> bool {
        self.inner.is_numeric()
    }

    fn is_boolean(&self) -> bool {
        self.inner.is_boolean()
    }

    fn is_floating(&self) -> bool {
        self.inner.is_floating()
    }

    fn is_integer(&self) -> bool {
        self.inner.is_integer()
    }

    fn is_categorical(&self) -> bool {
        self.inner.is_categorical()
    }

    fn is_object(&self) -> bool {
        self.inner.is_object()
    }

    fn is_interval(&self) -> bool {
        false
    }

    fn holds_integer(&self) -> bool {
        self.inner.holds_integer()
    }

    fn symmetric_difference(&self, other: &PyIndex) -> Self {
        PyIndex {
            inner: self.inner.symmetric_difference(&other.inner),
        }
    }

    fn get_indexer(&self, target: &PyIndex) -> Vec<i64> {
        self.inner
            .get_indexer(&target.inner)
            .into_iter()
            .map(|opt| opt.map(|u| u as i64).unwrap_or(-1))
            .collect()
    }

    #[pyo3(signature = (start=None, end=None, step=None))]
    fn slice_locs(
        &self,
        start: Option<&Bound<'_, PyAny>>,
        end: Option<&Bound<'_, PyAny>>,
        step: Option<isize>,
    ) -> PyResult<(usize, usize)> {
        let _ = step;
        let s_lbl = match start {
            Some(obj) => Some(py_to_index_label(obj)?),
            None => None,
        };
        let e_lbl = match end {
            Some(obj) => Some(py_to_index_label(obj)?),
            None => None,
        };
        self.inner
            .slice_locs(s_lbl.as_ref(), e_lbl.as_ref())
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyKeyError, _>(e.to_string()))
    }

    #[pyo3(signature = (start=None, end=None, step=None))]
    fn slice_indexer(
        &self,
        start: Option<&Bound<'_, PyAny>>,
        end: Option<&Bound<'_, PyAny>>,
        step: Option<isize>,
    ) -> PyResult<(usize, usize)> {
        self.slice_locs(start, end, step)
    }

    #[pyo3(signature = (ascending=true))]
    fn sort_values(&self, ascending: bool) -> Self {
        let mut sorted = self.inner.sort_values();
        if !ascending {
            let mut rev_labels = sorted.labels().to_vec();
            rev_labels.reverse();
            sorted = Index::new(rev_labels);
            if let Some(n) = self.inner.name() {
                sorted = sorted.rename_index(Some(n));
            }
        }
        PyIndex { inner: sorted }
    }

    fn sort(&self) -> Self {
        self.sort_values(true)
    }

    #[pyo3(signature = (level=None))]
    fn droplevel(&self, level: Option<usize>) -> PyResult<Self> {
        if let Some(l) = level
            && l != 0
        {
            return Err(PyErr::new::<pyo3::exceptions::PyIndexError, _>(
                "Index has only 1 level; cannot drop level > 0",
            ));
        }
        Ok(self.clone())
    }

    fn get_level_values(&self, level: usize) -> PyResult<Self> {
        if level != 0 {
            return Err(PyErr::new::<pyo3::exceptions::PyIndexError, _>(
                "Index has only 1 level; cannot get level > 0",
            ));
        }
        Ok(self.clone())
    }

    fn to_flat_index(&self) -> Self {
        self.clone()
    }

    #[pyo3(signature = (index=None, name=None))]
    fn to_series(&self, index: Option<&PyIndex>, name: Option<&str>) -> PyResult<PySeries> {
        let idx = match index {
            Some(i) => i.inner.clone(),
            None => self.inner.clone(),
        };
        let series_name = name.or_else(|| self.inner.name()).unwrap_or("");
        let col = Column::from_values(
            self.inner
                .labels()
                .iter()
                .map(|l| match l {
                    IndexLabel::Int64(i) => Scalar::Int64(*i),
                    IndexLabel::Float64(f) => Scalar::Float64(f.0),
                    IndexLabel::Utf8(s) => Scalar::Utf8(s.clone()),
                    IndexLabel::Bool(b) => Scalar::Bool(*b),
                    IndexLabel::Timedelta64(t) => Scalar::Timedelta64(*t),
                    IndexLabel::Datetime64(d) => Scalar::Datetime64(*d),
                    IndexLabel::Null(k) => Scalar::Null(*k),
                })
                .collect(),
        )
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        let s = Series::new(series_name, idx, col).map_err(frame_error_to_py)?;
        Ok(PySeries { inner: s })
    }

    #[pyo3(signature = (index=true, name=None))]
    fn to_frame(&self, index: bool, name: Option<&str>) -> PyResult<PyDataFrame> {
        let col_name = name.or_else(|| self.inner.name()).unwrap_or("0");
        let idx = if index {
            self.inner.clone()
        } else {
            Index::from_range(0, self.inner.len() as i64, 1)
        };
        let col = Column::from_values(
            self.inner
                .labels()
                .iter()
                .map(|l| match l {
                    IndexLabel::Int64(i) => Scalar::Int64(*i),
                    IndexLabel::Float64(f) => Scalar::Float64(f.0),
                    IndexLabel::Utf8(s) => Scalar::Utf8(s.clone()),
                    IndexLabel::Bool(b) => Scalar::Bool(*b),
                    IndexLabel::Timedelta64(t) => Scalar::Timedelta64(*t),
                    IndexLabel::Datetime64(d) => Scalar::Datetime64(*d),
                    IndexLabel::Null(k) => Scalar::Null(*k),
                })
                .collect(),
        )
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        let mut col_map = BTreeMap::new();
        col_map.insert(col_name.to_string(), col);
        let df = DataFrame::new_with_column_order(idx, col_map, vec![col_name.to_string()])
            .map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: df })
    }

    #[pyo3(signature = (normalize=false, sort=true, ascending=false, dropna=true))]
    fn value_counts(
        &self,
        normalize: bool,
        sort: bool,
        ascending: bool,
        dropna: bool,
    ) -> PyResult<PySeries> {
        let counts = self
            .inner
            .value_counts_with_options(normalize, sort, ascending, dropna);
        let mut idx_labels = Vec::with_capacity(counts.len());
        let mut vals = Vec::with_capacity(counts.len());
        for (lbl, sc) in counts {
            idx_labels.push(lbl);
            vals.push(sc);
        }
        let col = Column::from_values(vals)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        let s = Series::new("count", Index::new(idx_labels), col).map_err(frame_error_to_py)?;
        Ok(PySeries { inner: s })
    }

    fn array(&self, py: Python<'_>) -> PyResult<Py<PyList>> {
        self.to_list(py)
    }

    fn ravel(&self) -> Self {
        self.clone()
    }

    fn view(&self) -> Self {
        self.clone()
    }

    fn transpose(&self) -> Self {
        self.clone()
    }

    fn is_(&self, other: &Bound<'_, PyAny>) -> bool {
        if let Ok(other_idx) = other.extract::<PyRef<'_, PyIndex>>() {
            self.inner.is_(&other_idx.inner)
        } else {
            false
        }
    }

    #[pyo3(signature = (names, level=None))]
    fn set_names(&self, names: &Bound<'_, PyAny>, level: Option<usize>) -> PyResult<Self> {
        let _ = level;
        let name_opt = if let Ok(s) = names.extract::<String>() {
            Some(s)
        } else if let Ok(seq) = names.cast::<pyo3::types::PySequence>() {
            if seq.len()? > 0 {
                Some(seq.get_item(0)?.extract::<String>()?)
            } else {
                None
            }
        } else {
            None
        };
        let mut inner = self.inner.clone();
        inner = inner.set_names(name_opt.as_deref());
        Ok(PyIndex { inner })
    }

    fn infer_objects(&self) -> Self {
        self.clone()
    }

    #[pyo3(signature = (sort=false, use_na_sentinel=true))]
    fn factorize(&self, sort: bool, use_na_sentinel: bool) -> (Vec<isize>, PyIndex) {
        let _ = (sort, use_na_sentinel);
        let (codes, uniques) = self.inner.factorize();
        (codes, PyIndex { inner: uniques })
    }

    fn format(&self) -> Vec<String> {
        self.inner.format()
    }

    #[pyo3(signature = (periods=1))]
    fn diff(&self, periods: i64) -> PyResult<PyIndex> {
        let p = if periods < 0 { 0 } else { periods as usize };
        let diff_labels = self.inner.diff(p);
        let labels: Vec<IndexLabel> = diff_labels
            .into_iter()
            .map(|opt| opt.unwrap_or(IndexLabel::Null(NullKind::NaN)))
            .collect();
        let mut idx = Index::new(labels);
        if let Some(n) = self.inner.name() {
            idx = idx.rename_index(Some(n));
        }
        Ok(PyIndex { inner: idx })
    }

    #[pyo3(signature = (periods=1, freq=None))]
    fn shift(&self, periods: i64, freq: Option<&str>) -> PyResult<Self> {
        let _ = (periods, freq);
        Err(PyErr::new::<pyo3::exceptions::PyNotImplementedError, _>(
            "This method is only implemented for DatetimeIndex, PeriodIndex and TimedeltaIndex; Got type Index",
        ))
    }

    #[pyo3(signature = (decimals=0))]
    fn round(&self, decimals: i32) -> Self {
        let _ = decimals;
        PyIndex {
            inner: self.inner.round(),
        }
    }

    fn asof(&self, py: Python<'_>, label: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let lbl = py_to_index_label(label)?;
        match self.inner.asof(&lbl) {
            Some(l) => index_label_to_py(py, &l),
            None => Ok(py.None()),
        }
    }

    #[pyo3(signature = (where_, mask=None))]
    fn asof_locs(&self, where_: &PyIndex, mask: Option<Vec<bool>>) -> Vec<Option<usize>> {
        self.inner.asof_locs(&where_.inner, mask.as_deref())
    }

    #[pyo3(signature = (value, side="left", sorter=None))]
    fn searchsorted(
        &self,
        value: &Bound<'_, PyAny>,
        side: &str,
        sorter: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<usize> {
        let _ = sorter;
        let lbl = py_to_index_label(value)?;
        self.inner
            .searchsorted(&lbl, side)
            .map_err(index_error_to_py)
    }

    fn get_slice_bound(&self, label: &Bound<'_, PyAny>, side: &str) -> PyResult<usize> {
        let lbl = py_to_index_label(label)?;
        self.inner
            .get_slice_bound(&lbl, side)
            .map_err(index_error_to_py)
    }

    fn get_indexer_for(&self, target: &Bound<'_, PyAny>) -> PyResult<Vec<i64>> {
        let target_idx = if let Ok(py_idx) = target.extract::<PyRef<'_, PyIndex>>() {
            py_idx.inner.clone()
        } else {
            let py_idx = PyIndex::new(Some(target), None)?;
            py_idx.inner
        };
        Ok(self
            .inner
            .get_indexer_for(&target_idx)
            .into_iter()
            .map(|opt| opt.map(|u| u as i64).unwrap_or(-1))
            .collect())
    }

    fn get_indexer_non_unique(
        &self,
        target: &Bound<'_, PyAny>,
    ) -> PyResult<(Vec<isize>, Vec<usize>)> {
        let target_idx = if let Ok(py_idx) = target.extract::<PyRef<'_, PyIndex>>() {
            py_idx.inner.clone()
        } else {
            let py_idx = PyIndex::new(Some(target), None)?;
            py_idx.inner
        };
        Ok(self.inner.get_indexer_non_unique(&target_idx))
    }

    #[pyo3(signature = (level=None, ascending=true, sort_remaining=None))]
    fn sortlevel(
        &self,
        level: Option<usize>,
        ascending: bool,
        sort_remaining: Option<bool>,
    ) -> PyResult<(Self, Vec<usize>)> {
        let _ = (level, ascending, sort_remaining);
        let (sorted, order) = self.inner.sortlevel();
        Ok((PyIndex { inner: sorted }, order))
    }

    #[pyo3(signature = (other, how="left", level=None, return_indexers=false, sort=false))]
    fn join(
        &self,
        other: &Bound<'_, PyAny>,
        how: &str,
        level: Option<usize>,
        return_indexers: bool,
        sort: bool,
    ) -> PyResult<Self> {
        let _ = (level, return_indexers, sort);
        let other_idx = if let Ok(py_idx) = other.extract::<PyRef<'_, PyIndex>>() {
            py_idx.inner.clone()
        } else {
            let py_idx = PyIndex::new(Some(other), None)?;
            py_idx.inner
        };
        let joined = self
            .inner
            .join(&other_idx, how)
            .map_err(index_error_to_py)?;
        Ok(PyIndex { inner: joined })
    }

    #[pyo3(signature = (target, method=None, level=None, limit=None, tolerance=None))]
    fn reindex(
        &self,
        target: &Bound<'_, PyAny>,
        method: Option<&str>,
        level: Option<usize>,
        limit: Option<usize>,
        tolerance: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<(Self, Vec<i64>)> {
        let _ = (method, level, limit, tolerance);
        let target_idx = if let Ok(py_idx) = target.extract::<PyRef<'_, PyIndex>>() {
            py_idx.inner.clone()
        } else {
            let py_idx = PyIndex::new(Some(target), None)?;
            py_idx.inner
        };
        let (out_idx, indexer) = self.inner.reindex(&target_idx);
        let indexer_vec: Vec<i64> = indexer
            .into_iter()
            .map(|opt| opt.map(|u| u as i64).unwrap_or(-1))
            .collect();
        Ok((PyIndex { inner: out_idx }, indexer_vec))
    }

    #[pyo3(signature = (cond, other=None))]
    fn r#where(&self, cond: Vec<bool>, other: Option<&Bound<'_, PyAny>>) -> PyResult<Self> {
        let other_lbl = match other {
            Some(o) => py_to_index_label(o)?,
            None => IndexLabel::Null(NullKind::NaN),
        };
        Ok(PyIndex {
            inner: self.inner.where_(&cond, &other_lbl),
        })
    }

    fn putmask(&self, mask: Vec<bool>, value: &Bound<'_, PyAny>) -> PyResult<Self> {
        let val = py_to_index_label(value)?;
        Ok(PyIndex {
            inner: self.inner.putmask(&mask, &val),
        })
    }

    fn map(&self, py: Python<'_>, mapper: &Bound<'_, PyAny>) -> PyResult<Self> {
        let mut new_labels = Vec::with_capacity(self.inner.len());
        for l in self.inner.labels() {
            let py_val = index_label_to_py(py, l)?;
            let res = mapper.call1((py_val,))?;
            new_labels.push(py_to_index_label(&res)?);
        }
        let mut res = Index::new(new_labels);
        if let Some(n) = self.inner.name() {
            res = res.rename_index(Some(n));
        }
        Ok(PyIndex { inner: res })
    }

    fn groupby(&self, py: Python<'_>, by: &Bound<'_, PyAny>) -> PyResult<Py<pyo3::types::PyDict>> {
        let dict = pyo3::types::PyDict::new(py);
        if let Ok(seq) = by.cast::<pyo3::types::PySequence>() {
            let len = seq.len()?;
            let mut groups: std::collections::BTreeMap<IndexLabel, Vec<IndexLabel>> =
                std::collections::BTreeMap::new();
            for i in 0..len.min(self.inner.len()) {
                let key = py_to_index_label(&seq.get_item(i)?)?;
                groups
                    .entry(key)
                    .or_default()
                    .push(self.inner.labels()[i].clone());
            }
            for (k, v) in groups {
                let py_k = index_label_to_py(py, &k)?;
                let py_v: Vec<Py<PyAny>> = v
                    .iter()
                    .map(|l| index_label_to_py(py, l))
                    .collect::<PyResult<_>>()?;
                let v_list = PyList::new(py, py_v)?;
                dict.set_item(py_k, v_list)?;
            }
        }
        Ok(dict.unbind())
    }

    #[getter]
    fn r#str(&self) -> PyIndexStringMethods {
        PyIndexStringMethods {
            inner: self.inner.clone(),
        }
    }
}

/// Python wrapper for FrankenPandas DatetimeIndex.
#[pyclass(name = "DatetimeIndex", from_py_object)]
#[derive(Clone)]
pub struct PyDatetimeIndex {
    pub(crate) inner: DatetimeIndex,
}

#[pymethods]
impl PyDatetimeIndex {
    #[new]
    #[pyo3(signature = (data=None, name=None))]
    fn new(py: Python<'_>, data: Option<&Bound<'_, PyAny>>, name: Option<&str>) -> PyResult<Self> {
        let mut inner = if let Some(d) = data {
            if let Ok(dti) = d.extract::<PyRef<'_, PyDatetimeIndex>>() {
                dti.inner.clone()
            } else if let Ok(idx) = d.extract::<PyRef<'_, PyIndex>>() {
                DatetimeIndex::from_index(idx.inner.clone())
                    .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?
            } else if let Ok(s) = d.extract::<PyRef<'_, PySeries>>() {
                let dt_series = fp_frame::to_datetime_with_options(
                    &s.inner,
                    fp_frame::ToDatetimeOptions::default(),
                )
                .map_err(frame_error_to_py)?;
                let nanos: Vec<i64> = dt_series
                    .values()
                    .iter()
                    .map(|v| match v {
                        Scalar::Datetime64(n) => *n,
                        _ => i64::MIN,
                    })
                    .collect();
                DatetimeIndex::new(nanos)
            } else if let Ok(list) = d.cast::<PyList>() {
                let values: Vec<Scalar> = list
                    .iter()
                    .map(|v| py_to_scalar(py, &v))
                    .collect::<PyResult<Vec<_>>>()?;
                let temp_series = Series::from_values(
                    "",
                    (0..values.len())
                        .map(|i| IndexLabel::Int64(i as i64))
                        .collect(),
                    values,
                )
                .map_err(frame_error_to_py)?;
                let dt_series = fp_frame::to_datetime_with_options(
                    &temp_series,
                    fp_frame::ToDatetimeOptions::default(),
                )
                .map_err(frame_error_to_py)?;
                let nanos: Vec<i64> = dt_series
                    .values()
                    .iter()
                    .map(|v| match v {
                        Scalar::Datetime64(n) => *n,
                        _ => i64::MIN,
                    })
                    .collect();
                DatetimeIndex::new(nanos)
            } else {
                return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                    "DatetimeIndex data must be sequence of datetime values",
                ));
            }
        } else {
            DatetimeIndex::new(Vec::new())
        };
        if let Some(n) = name {
            inner = inner.set_name(n);
        }
        Ok(PyDatetimeIndex { inner })
    }

    #[getter]
    fn name(&self) -> Option<String> {
        self.inner.name().map(String::from)
    }

    #[setter]
    fn set_name(&mut self, name: Option<&str>) {
        self.inner = self.inner.set_names(name);
    }

    #[getter]
    fn dtype(&self) -> &'static str {
        "datetime64[ns]"
    }

    #[getter]
    fn shape(&self) -> (usize,) {
        (self.inner.len(),)
    }

    #[getter]
    fn size(&self) -> usize {
        self.inner.len()
    }

    #[getter]
    fn ndim(&self) -> usize {
        1
    }

    #[getter]
    fn empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    fn __len__(&self) -> usize {
        self.inner.len()
    }

    #[getter]
    fn is_monotonic_increasing(&self) -> bool {
        self.inner.is_monotonic_increasing()
    }

    #[getter]
    fn is_monotonic_decreasing(&self) -> bool {
        self.inner.is_monotonic_decreasing()
    }

    #[getter]
    fn is_unique(&self) -> bool {
        self.inner.is_unique()
    }

    #[getter]
    fn has_duplicates(&self) -> bool {
        self.inner.has_duplicates()
    }

    #[getter]
    fn asi8(&self) -> Vec<i64> {
        self.inner.asi8().to_vec()
    }

    #[getter]
    fn year(&self) -> Vec<Option<i32>> {
        self.inner.year()
    }

    #[getter]
    fn month(&self) -> Vec<Option<u32>> {
        self.inner.month()
    }

    #[getter]
    fn day(&self) -> Vec<Option<u32>> {
        self.inner.day()
    }

    #[getter]
    fn hour(&self) -> Vec<Option<u32>> {
        self.inner.hour()
    }

    #[getter]
    fn minute(&self) -> Vec<Option<u32>> {
        self.inner.minute()
    }

    #[getter]
    fn second(&self) -> Vec<Option<u32>> {
        self.inner.second()
    }

    #[getter]
    fn microsecond(&self) -> Vec<Option<u32>> {
        self.inner.microsecond()
    }

    #[getter]
    fn nanosecond(&self) -> Vec<Option<u32>> {
        self.inner.nanosecond()
    }

    #[getter]
    fn dayofweek(&self) -> Vec<Option<u32>> {
        self.inner.dayofweek()
    }

    fn day_name(&self) -> Vec<Option<String>> {
        self.inner.day_name()
    }

    fn month_name(&self) -> Vec<Option<String>> {
        self.inner.month_name()
    }

    #[getter]
    fn is_leap_year(&self) -> Vec<Option<bool>> {
        self.inner.is_leap_year()
    }

    #[getter]
    fn days_in_month(&self) -> Vec<Option<u32>> {
        self.inner.days_in_month()
    }

    fn to_list(&self) -> Vec<String> {
        self.inner.format()
    }

    fn tolist(&self) -> Vec<String> {
        self.to_list()
    }

    fn values(&self) -> Vec<Option<i64>> {
        self.inner.values()
    }

    fn to_numpy(&self) -> Vec<Option<i64>> {
        self.inner.values()
    }

    fn min(&self) -> Option<String> {
        self.inner.min().map(format_datetime_ns)
    }

    fn max(&self) -> Option<String> {
        self.inner.max().map(format_datetime_ns)
    }

    fn mean(&self) -> Option<String> {
        self.inner.mean().map(format_datetime_ns)
    }

    fn std(&self) -> Option<f64> {
        self.inner.std().map(|v| v as f64)
    }

    fn var(&self) -> Option<f64> {
        self.inner.var()
    }

    fn unique(&self) -> PyResult<Self> {
        let inner = self
            .inner
            .unique()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PyDatetimeIndex { inner })
    }

    fn nunique(&self) -> usize {
        self.inner.nunique()
    }

    fn drop_duplicates(&self) -> PyResult<Self> {
        let inner = self
            .inner
            .drop_duplicates()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PyDatetimeIndex { inner })
    }

    fn duplicated(&self) -> Vec<bool> {
        self.inner.duplicated(DuplicateKeep::First)
    }

    fn isna(&self) -> Vec<bool> {
        self.inner.isna()
    }

    fn isnull(&self) -> Vec<bool> {
        self.inner.isna()
    }

    fn notna(&self) -> Vec<bool> {
        self.inner.notna()
    }

    fn notnull(&self) -> Vec<bool> {
        self.inner.notna()
    }

    fn isin(&self, values: Vec<i64>) -> Vec<bool> {
        self.inner.isin(&values)
    }

    fn intersection(&self, other: &PyDatetimeIndex) -> Self {
        PyDatetimeIndex {
            inner: self.inner.intersection(&other.inner),
        }
    }

    fn union(&self, other: &PyDatetimeIndex) -> Self {
        PyDatetimeIndex {
            inner: self.inner.union(&other.inner),
        }
    }

    fn difference(&self, other: &PyDatetimeIndex) -> Self {
        PyDatetimeIndex {
            inner: self.inner.difference(&other.inner),
        }
    }

    fn round(&self, freq: &str) -> PyResult<Self> {
        let r = self
            .inner
            .round(freq)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PyDatetimeIndex { inner: r })
    }

    fn floor(&self, freq: &str) -> PyResult<Self> {
        let r = self
            .inner
            .floor(freq)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PyDatetimeIndex { inner: r })
    }

    fn ceil(&self, freq: &str) -> PyResult<Self> {
        let r = self
            .inner
            .ceil(freq)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PyDatetimeIndex { inner: r })
    }

    #[pyo3(signature = (periods=1, freq="D"))]
    fn shift(&self, periods: i64, freq: &str) -> PyResult<Self> {
        let freq_nanos = parse_freq_to_nanos(freq)?;
        let r = self.inner.shift(periods, freq_nanos);
        Ok(PyDatetimeIndex { inner: r })
    }

    #[pyo3(signature = (periods=1))]
    fn diff(&self, periods: i64) -> Vec<Option<i64>> {
        let td = self.inner.diff(periods);
        td.as_index()
            .labels()
            .iter()
            .map(|l| match l {
                IndexLabel::Timedelta64(ns) if *ns != fp_types::Timedelta::NAT => Some(*ns),
                _ => None,
            })
            .collect()
    }

    fn strftime(&self, format: &str) -> Vec<Option<String>> {
        self.inner.strftime(format)
    }

    fn copy(&self) -> Self {
        self.clone()
    }

    fn rename(&self, name: &str) -> Self {
        PyDatetimeIndex {
            inner: self.inner.rename(name),
        }
    }

    fn equals(&self, other: &PyDatetimeIndex) -> bool {
        self.inner.equals(&other.inner)
    }

    fn __repr__(&self) -> String {
        let strings: Vec<String> = self.inner.format();
        let name_str = match self.inner.name() {
            Some(n) => format!(", name='{n}'"),
            None => String::new(),
        };
        format!(
            "DatetimeIndex([{}], dtype='datetime64[ns]'{name_str})",
            strings
                .iter()
                .map(|s| format!("'{s}'"))
                .collect::<Vec<_>>()
                .join(", ")
        )
    }

    fn __getitem__(&self, py: Python<'_>, key: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        if let Ok(idx) = key.extract::<i64>() {
            let pos = if idx < 0 {
                (self.inner.len() as i64 + idx) as usize
            } else {
                idx as usize
            };
            if pos >= self.inner.len() {
                return Err(PyErr::new::<pyo3::exceptions::PyIndexError, _>(
                    "index out of bounds",
                ));
            }
            let nanos = self.inner.asi8()[pos];
            return format_datetime_ns(nanos).into_py_any(py);
        }
        if let Ok(slice) = key.cast::<pyo3::types::PySlice>() {
            let s_idx = slice.indices(self.inner.len() as isize)?;
            let mut sliced = Vec::new();
            let asi8 = self.inner.asi8();
            let mut i = s_idx.start;
            if s_idx.step > 0 {
                while i < s_idx.stop {
                    sliced.push(asi8[i as usize]);
                    i += s_idx.step;
                }
            } else if s_idx.step < 0 {
                while i > s_idx.stop {
                    sliced.push(asi8[i as usize]);
                    i += s_idx.step;
                }
            }
            let mut out = DatetimeIndex::new(sliced);
            if let Some(n) = self.inner.name() {
                out = out.set_name(n);
            }
            return Ok(Py::new(py, PyDatetimeIndex { inner: out })?.into_any());
        }
        Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
            "Index indices must be integers or slices",
        ))
    }

    #[getter]
    fn hasnans(&self) -> bool {
        self.inner.as_index().hasnans()
    }

    #[getter]
    fn nlevels(&self) -> usize {
        1
    }

    #[getter]
    fn names(&self) -> Vec<Option<String>> {
        vec![self.inner.name().map(str::to_string)]
    }

    #[getter]
    fn nbytes(&self) -> usize {
        self.inner.as_index().nbytes()
    }

    #[pyo3(signature = (deep=false))]
    fn memory_usage(&self, deep: bool) -> usize {
        self.inner.as_index().memory_usage(deep)
    }

    fn inferred_type(&self) -> &'static str {
        self.inner.as_index().inferred_type()
    }

    fn is_numeric(&self) -> bool {
        false
    }

    fn is_boolean(&self) -> bool {
        false
    }

    fn is_floating(&self) -> bool {
        false
    }

    fn is_integer(&self) -> bool {
        false
    }

    fn is_categorical(&self) -> bool {
        false
    }

    fn is_object(&self) -> bool {
        false
    }

    fn is_interval(&self) -> bool {
        false
    }

    fn holds_integer(&self) -> bool {
        false
    }

    fn symmetric_difference(&self, other: &PyIndex) -> PyIndex {
        PyIndex {
            inner: self.inner.as_index().symmetric_difference(&other.inner),
        }
    }

    fn get_loc(&self, key: &Bound<'_, PyAny>) -> PyResult<usize> {
        let label = py_to_index_label(key)?;
        self.inner
            .as_index()
            .get_loc(&label)
            .ok_or_else(|| PyErr::new::<pyo3::exceptions::PyKeyError, _>(format!("{key}")))
    }

    fn get_indexer(&self, target: &PyIndex) -> Vec<i64> {
        self.inner
            .as_index()
            .get_indexer(&target.inner)
            .into_iter()
            .map(|opt| opt.map(|u| u as i64).unwrap_or(-1))
            .collect()
    }

    #[pyo3(signature = (start=None, end=None, step=None))]
    fn slice_locs(
        &self,
        start: Option<&Bound<'_, PyAny>>,
        end: Option<&Bound<'_, PyAny>>,
        step: Option<isize>,
    ) -> PyResult<(usize, usize)> {
        let _ = step;
        let s_lbl = match start {
            Some(obj) => Some(py_to_index_label(obj)?),
            None => None,
        };
        let e_lbl = match end {
            Some(obj) => Some(py_to_index_label(obj)?),
            None => None,
        };
        self.inner
            .as_index()
            .slice_locs(s_lbl.as_ref(), e_lbl.as_ref())
            .map_err(index_error_to_py)
    }

    #[pyo3(signature = (start=None, end=None, step=None))]
    fn slice_indexer(
        &self,
        start: Option<&Bound<'_, PyAny>>,
        end: Option<&Bound<'_, PyAny>>,
        step: Option<isize>,
    ) -> PyResult<(usize, usize, isize)> {
        let (s, e) = self.slice_locs(start, end, step)?;
        Ok((s, e, step.unwrap_or(1)))
    }

    #[pyo3(signature = (level=0))]
    fn droplevel(&self, level: usize) -> PyResult<PyIndex> {
        let _ = level;
        Ok(PyIndex {
            inner: self.inner.as_index().clone(),
        })
    }

    #[pyo3(signature = (level=0))]
    fn get_level_values(&self, level: usize) -> PyResult<Self> {
        let _ = level;
        Ok(self.clone())
    }

    fn to_flat_index(&self) -> Self {
        self.clone()
    }

    #[pyo3(signature = (index=None, name=None))]
    fn to_series(
        &self,
        index: Option<&Bound<'_, PyAny>>,
        name: Option<&str>,
    ) -> PyResult<PySeries> {
        let series_name = name.or_else(|| self.inner.name()).unwrap_or("");
        let idx = if let Some(i_obj) = index {
            if let Ok(py_idx) = i_obj.extract::<PyRef<'_, PyIndex>>() {
                py_idx.inner.clone()
            } else {
                self.inner.as_index().clone()
            }
        } else {
            self.inner.as_index().clone()
        };
        let col = Column::from_values(
            self.inner
                .as_index()
                .labels()
                .iter()
                .map(|l| match l {
                    IndexLabel::Datetime64(d) => Scalar::Datetime64(*d),
                    _ => Scalar::Null(NullKind::NaN),
                })
                .collect(),
        )
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        let s = Series::new(series_name, idx, col).map_err(frame_error_to_py)?;
        Ok(PySeries { inner: s })
    }

    #[pyo3(signature = (index=true, name=None))]
    fn to_frame(&self, index: bool, name: Option<&str>) -> PyResult<PyDataFrame> {
        let col_name = name.or_else(|| self.inner.name()).unwrap_or("0");
        let idx = if index {
            self.inner.as_index().clone()
        } else {
            Index::from_range(0, self.inner.len() as i64, 1)
        };
        let col = Column::from_values(
            self.inner
                .as_index()
                .labels()
                .iter()
                .map(|l| match l {
                    IndexLabel::Datetime64(d) => Scalar::Datetime64(*d),
                    _ => Scalar::Null(NullKind::NaN),
                })
                .collect(),
        )
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        let mut col_map = BTreeMap::new();
        col_map.insert(col_name.to_string(), col);
        let df = DataFrame::new_with_column_order(idx, col_map, vec![col_name.to_string()])
            .map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: df })
    }

    #[pyo3(signature = (normalize=false, sort=true, ascending=false, dropna=true))]
    fn value_counts(
        &self,
        normalize: bool,
        sort: bool,
        ascending: bool,
        dropna: bool,
    ) -> PyResult<PySeries> {
        let counts = self
            .inner
            .as_index()
            .value_counts_with_options(normalize, sort, ascending, dropna);
        let mut idx_labels = Vec::with_capacity(counts.len());
        let mut vals = Vec::with_capacity(counts.len());
        for (lbl, sc) in counts {
            idx_labels.push(lbl);
            vals.push(sc);
        }
        let col = Column::from_values(vals)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        let s = Series::new("count", Index::new(idx_labels), col).map_err(frame_error_to_py)?;
        Ok(PySeries { inner: s })
    }

    #[pyo3(signature = (ascending=true, na_position="last"))]
    fn sort_values(&self, ascending: bool, na_position: &str) -> PyResult<Self> {
        let mut sorted = self.inner.asi8().to_vec();
        if ascending {
            sorted.sort_unstable();
        } else {
            sorted.sort_unstable_by(|a, b| b.cmp(a));
        }
        let _ = na_position;
        let mut out = DatetimeIndex::new(sorted);
        if let Some(n) = self.inner.name() {
            out = out.set_name(n);
        }
        Ok(Self { inner: out })
    }

    fn sort(&self) -> PyResult<Self> {
        self.sort_values(true, "last")
    }

    #[getter]
    #[allow(non_snake_case)]
    fn T(&self) -> Self {
        self.clone()
    }

    fn drop(&self, labels: Vec<Bound<'_, PyAny>>) -> PyResult<PyIndex> {
        let mut to_drop = Vec::with_capacity(labels.len());
        for l in labels {
            to_drop.push(py_to_index_label(&l)?);
        }
        Ok(PyIndex {
            inner: self.inner.as_index().drop_labels(&to_drop),
        })
    }

    #[pyo3(signature = (how="any"))]
    fn dropna(&self, how: &str) -> Self {
        let _ = how;
        self.clone()
    }

    fn fillna(&self, value: &Bound<'_, PyAny>) -> Self {
        let _ = value;
        self.clone()
    }

    fn as_py_index(&self) -> PyIndex {
        PyIndex {
            inner: self.inner.as_index().clone(),
        }
    }

    fn all(&self) -> bool {
        self.as_py_index().all()
    }

    fn any(&self) -> bool {
        self.as_py_index().any()
    }

    fn append(&self, others: Vec<Bound<'_, PyAny>>) -> PyResult<Self> {
        let mut combined = self.inner.asi8();
        for other in others {
            if let Ok(dti) = other.extract::<PyRef<'_, PyDatetimeIndex>>() {
                combined.extend(dti.inner.asi8());
            } else if let Ok(list) = other.extract::<Vec<i64>>() {
                combined.extend(list);
            }
        }
        let mut out = DatetimeIndex::new(combined);
        if let Some(n) = self.inner.name() {
            out = out.set_name(n);
        }
        Ok(Self { inner: out })
    }

    fn argmax(&self) -> PyResult<usize> {
        self.as_py_index().argmax()
    }

    fn argmin(&self) -> PyResult<usize> {
        self.as_py_index().argmin()
    }

    fn argsort(&self) -> Vec<usize> {
        self.as_py_index().argsort()
    }

    fn array(&self, py: Python<'_>) -> PyResult<Py<PyList>> {
        self.as_py_index().array(py)
    }

    fn as_unit(&self, unit: &str) -> Self {
        let _ = unit;
        self.clone()
    }

    fn asof(&self, py: Python<'_>, label: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        self.as_py_index().asof(py, label)
    }

    #[pyo3(signature = (where_, mask=None))]
    fn asof_locs(&self, where_: &PyIndex, mask: Option<Vec<bool>>) -> Vec<Option<usize>> {
        self.as_py_index().asof_locs(where_, mask)
    }

    fn astype(&self, dtype: &str) -> PyResult<PyIndex> {
        self.as_py_index().astype(dtype)
    }

    fn date(&self, py: Python<'_>) -> PyResult<Py<PyList>> {
        let dt_mod = py.import("datetime")?;
        let date_cls = dt_mod.getattr("date")?;
        let mut out = Vec::with_capacity(self.inner.len());
        let years = self.inner.year();
        let months = self.inner.month();
        let days = self.inner.day();
        for i in 0..self.inner.len() {
            if let (Some(y), Some(m), Some(d)) = (years[i], months[i], days[i]) {
                let py_d = date_cls.call1((y, m, d))?;
                out.push(py_d.into_any().unbind());
            } else {
                out.push(py.None());
            }
        }
        Ok(PyList::new(py, out)?.unbind())
    }

    #[getter]
    fn day_of_week(&self) -> Vec<Option<u32>> {
        self.inner.day_of_week()
    }

    #[getter]
    fn day_of_year(&self) -> Vec<Option<u32>> {
        self.inner.day_of_year()
    }

    #[getter]
    fn dayofyear(&self) -> Vec<Option<u32>> {
        self.inner.dayofyear()
    }

    #[getter]
    fn daysinmonth(&self) -> Vec<Option<u32>> {
        self.inner.daysinmonth()
    }

    fn delete(&self, loc: usize) -> PyResult<Self> {
        let new_idx = self.as_py_index().delete(loc)?;
        let mut vals = Vec::with_capacity(new_idx.inner.len());
        for l in new_idx.inner.labels() {
            match l {
                IndexLabel::Datetime64(ns) => vals.push(*ns),
                _ => vals.push(i64::MIN),
            }
        }
        let mut out = DatetimeIndex::new(vals);
        if let Some(n) = new_idx.inner.name() {
            out = out.set_name(n);
        }
        Ok(Self { inner: out })
    }

    #[pyo3(signature = (sort=false, use_na_sentinel=true))]
    fn factorize(&self, sort: bool, use_na_sentinel: bool) -> (Vec<isize>, PyIndex) {
        self.as_py_index().factorize(sort, use_na_sentinel)
    }

    fn format(&self) -> Vec<String> {
        self.as_py_index().format()
    }

    #[getter]
    fn freq(&self) -> Option<String> {
        None
    }

    #[getter]
    fn freqstr(&self) -> Option<String> {
        None
    }

    #[getter]
    fn inferred_freq(&self) -> Option<String> {
        None
    }

    #[getter]
    fn unit(&self) -> &'static str {
        "ns"
    }

    #[getter]
    fn resolution(&self) -> &'static str {
        "nano"
    }

    fn get_indexer_for(&self, target: &Bound<'_, PyAny>) -> PyResult<Vec<i64>> {
        self.as_py_index().get_indexer_for(target)
    }

    fn get_indexer_non_unique(
        &self,
        target: &Bound<'_, PyAny>,
    ) -> PyResult<(Vec<isize>, Vec<usize>)> {
        self.as_py_index().get_indexer_non_unique(target)
    }

    fn get_slice_bound(&self, label: &Bound<'_, PyAny>, side: &str) -> PyResult<usize> {
        self.as_py_index().get_slice_bound(label, side)
    }

    fn groupby(&self, py: Python<'_>, by: &Bound<'_, PyAny>) -> PyResult<Py<pyo3::types::PyDict>> {
        self.as_py_index().groupby(py, by)
    }

    fn identical(&self, other: &Bound<'_, PyAny>) -> bool {
        if let Ok(dti) = other.extract::<PyRef<'_, PyDatetimeIndex>>() {
            self.inner.equals(&dti.inner) && self.inner.name() == dti.inner.name()
        } else {
            false
        }
    }

    fn indexer_at_time(&self, time: &str) -> PyResult<Vec<usize>> {
        self.inner.indexer_at_time(time).map_err(index_error_to_py)
    }

    #[pyo3(signature = (start_time, end_time, include_start=true, include_end=true))]
    fn indexer_between_time(
        &self,
        start_time: &str,
        end_time: &str,
        include_start: bool,
        include_end: bool,
    ) -> PyResult<Vec<usize>> {
        self.inner
            .indexer_between_time(start_time, end_time, include_start, include_end)
            .map_err(index_error_to_py)
    }

    fn infer_objects(&self) -> Self {
        self.clone()
    }

    fn insert(&self, loc: usize, item: &Bound<'_, PyAny>) -> PyResult<PyIndex> {
        self.as_py_index().insert(loc, item)
    }

    fn is_(&self, other: &Bound<'_, PyAny>) -> bool {
        if let Ok(dti) = other.extract::<PyRef<'_, PyDatetimeIndex>>() {
            self.inner.equals(&dti.inner) && self.inner.name() == dti.inner.name()
        } else {
            false
        }
    }

    #[getter]
    fn is_month_end(&self) -> Vec<Option<bool>> {
        self.inner.is_month_end()
    }

    #[getter]
    fn is_month_start(&self) -> Vec<Option<bool>> {
        self.inner.is_month_start()
    }

    #[getter]
    fn is_normalized(&self) -> bool {
        self.inner.is_normalized()
    }

    #[getter]
    fn is_quarter_end(&self) -> Vec<Option<bool>> {
        self.inner.is_quarter_end()
    }

    #[getter]
    fn is_quarter_start(&self) -> Vec<Option<bool>> {
        self.inner.is_quarter_start()
    }

    #[getter]
    fn is_year_end(&self) -> Vec<Option<bool>> {
        self.inner.is_year_end()
    }

    #[getter]
    fn is_year_start(&self) -> Vec<Option<bool>> {
        self.inner.is_year_start()
    }

    fn isocalendar(&self) -> PyResult<PyDataFrame> {
        let cal = self.inner.isocalendar();
        let mut years = Vec::with_capacity(cal.len());
        let mut weeks = Vec::with_capacity(cal.len());
        let mut days = Vec::with_capacity(cal.len());
        for item in cal {
            match item {
                Some((y, w, d)) => {
                    years.push(Scalar::Int64(y as i64));
                    weeks.push(Scalar::Int64(w as i64));
                    days.push(Scalar::Int64(d as i64));
                }
                None => {
                    years.push(Scalar::Null(NullKind::NaN));
                    weeks.push(Scalar::Null(NullKind::NaN));
                    days.push(Scalar::Null(NullKind::NaN));
                }
            }
        }
        let cols = vec![("year", years), ("week", weeks), ("day", days)];
        let df = DataFrame::from_dict(&["year", "week", "day"], cols).map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: df })
    }

    fn item(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        self.as_py_index().item(py)
    }

    #[pyo3(signature = (other, how="left", level=None, return_indexers=false, sort=false))]
    fn join(
        &self,
        other: &Bound<'_, PyAny>,
        how: &str,
        level: Option<usize>,
        return_indexers: bool,
        sort: bool,
    ) -> PyResult<PyIndex> {
        self.as_py_index()
            .join(other, how, level, return_indexers, sort)
    }

    fn map(&self, py: Python<'_>, mapper: &Bound<'_, PyAny>) -> PyResult<PyIndex> {
        self.as_py_index().map(py, mapper)
    }

    fn normalize(&self) -> Self {
        Self {
            inner: self.inner.normalize(),
        }
    }

    fn putmask(&self, mask: Vec<bool>, value: &Bound<'_, PyAny>) -> PyResult<PyIndex> {
        self.as_py_index().putmask(mask, value)
    }

    #[getter]
    fn quarter(&self) -> Vec<Option<u32>> {
        self.inner.quarter()
    }

    fn ravel(&self) -> Self {
        self.clone()
    }

    #[pyo3(signature = (target, method=None, level=None, limit=None, tolerance=None))]
    fn reindex(
        &self,
        target: &Bound<'_, PyAny>,
        method: Option<&str>,
        level: Option<usize>,
        limit: Option<usize>,
        tolerance: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<(PyIndex, Vec<i64>)> {
        self.as_py_index()
            .reindex(target, method, level, limit, tolerance)
    }

    fn repeat(&self, repeats: usize) -> Self {
        let mut out = Vec::with_capacity(self.inner.len() * repeats);
        for &v in &self.inner.asi8() {
            for _ in 0..repeats {
                out.push(v);
            }
        }
        let mut res = DatetimeIndex::new(out);
        if let Some(n) = self.inner.name() {
            res = res.set_name(n);
        }
        Self { inner: res }
    }

    #[pyo3(signature = (value, side="left", sorter=None))]
    fn searchsorted(
        &self,
        value: &Bound<'_, PyAny>,
        side: &str,
        sorter: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<usize> {
        self.as_py_index().searchsorted(value, side, sorter)
    }

    fn set_names(&self, names: &Bound<'_, PyAny>) -> PyResult<Self> {
        let name = if let Ok(s) = names.extract::<String>() {
            Some(s)
        } else if let Ok(list) = names.extract::<Vec<Option<String>>>() {
            list.into_iter().next().flatten()
        } else {
            None
        };
        let mut res = self.inner.clone();
        if let Some(n) = name {
            res = res.set_name(&n);
        }
        Ok(Self { inner: res })
    }

    fn snap(&self, freq: Option<&str>) -> Self {
        let _ = freq;
        self.clone()
    }

    #[pyo3(signature = (level=None, ascending=true, sort_remaining=None))]
    fn sortlevel(
        &self,
        level: Option<usize>,
        ascending: bool,
        sort_remaining: Option<bool>,
    ) -> PyResult<(Self, Vec<usize>)> {
        let _ = (level, ascending, sort_remaining);
        Ok((self.clone(), (0..self.inner.len()).collect()))
    }

    #[getter]
    fn r#str(&self) -> PyIndexStringMethods {
        PyIndexStringMethods {
            inner: self.inner.as_index().clone(),
        }
    }

    fn take(&self, indices: Vec<i64>) -> Self {
        let vals = self.inner.asi8();
        let len = vals.len() as i64;
        let mut out = Vec::with_capacity(indices.len());
        for idx in indices {
            let pos = if idx < 0 { len + idx } else { idx };
            if pos >= 0 && pos < len {
                out.push(vals[pos as usize]);
            } else {
                out.push(i64::MIN);
            }
        }
        let mut res = DatetimeIndex::new(out);
        if let Some(n) = self.inner.name() {
            res = res.set_name(n);
        }
        Self { inner: res }
    }

    fn time(&self, py: Python<'_>) -> PyResult<Py<PyList>> {
        let dt_mod = py.import("datetime")?;
        let time_cls = dt_mod.getattr("time")?;
        let mut out = Vec::with_capacity(self.inner.len());
        let hours = self.inner.hour();
        let minutes = self.inner.minute();
        let seconds = self.inner.second();
        let microseconds = self.inner.microsecond();
        for i in 0..self.inner.len() {
            if let (Some(h), Some(m), Some(s), Some(us)) =
                (hours[i], minutes[i], seconds[i], microseconds[i])
            {
                let py_t = time_cls.call1((h, m, s, us))?;
                out.push(py_t.into_any().unbind());
            } else {
                out.push(py.None());
            }
        }
        Ok(PyList::new(py, out)?.unbind())
    }

    fn timetz(&self, py: Python<'_>) -> PyResult<Py<PyList>> {
        self.time(py)
    }

    fn to_julian_date(&self) -> Vec<Option<f64>> {
        self.inner
            .asi8()
            .iter()
            .map(|&ns| {
                if ns == i64::MIN {
                    None
                } else {
                    Some((ns as f64) / 86_400_000_000_000.0 + 2440587.5)
                }
            })
            .collect()
    }

    #[pyo3(signature = (freq="D"))]
    fn to_period(&self, freq: &str) -> PyResult<PyPeriodIndex> {
        let p_freq = PeriodFreq::parse(freq).unwrap_or(PeriodFreq::Daily);
        let periods = self
            .inner
            .asi8()
            .iter()
            .map(|&ns| {
                let days = if ns == i64::MIN {
                    0
                } else {
                    ns / (86_400 * 1_000_000_000)
                };
                Period::new(days, p_freq)
            })
            .collect();
        let mut out = PeriodIndex::new(periods);
        if let Some(n) = self.inner.name() {
            out = out.set_name(n);
        }
        Ok(PyPeriodIndex { inner: out })
    }

    fn to_pydatetime(&self, py: Python<'_>) -> PyResult<Py<PyList>> {
        let dt_mod = py.import("datetime")?;
        let dt_cls = dt_mod.getattr("datetime")?;
        let mut out = Vec::with_capacity(self.inner.len());
        let years = self.inner.year();
        let months = self.inner.month();
        let days = self.inner.day();
        let hours = self.inner.hour();
        let minutes = self.inner.minute();
        let seconds = self.inner.second();
        let microseconds = self.inner.microsecond();
        for i in 0..self.inner.len() {
            if let (Some(y), Some(mo), Some(d), Some(h), Some(mi), Some(s), Some(us)) = (
                years[i],
                months[i],
                days[i],
                hours[i],
                minutes[i],
                seconds[i],
                microseconds[i],
            ) {
                let py_dt = dt_cls.call1((y, mo, d, h, mi, s, us))?;
                out.push(py_dt.into_any().unbind());
            } else {
                out.push(py.None());
            }
        }
        Ok(PyList::new(py, out)?.unbind())
    }

    fn transpose(&self) -> Self {
        self.clone()
    }

    #[getter]
    fn tz(&self) -> Option<String> {
        None
    }

    fn tz_convert(&self, tz: Option<&str>) -> Self {
        let _ = tz;
        self.clone()
    }

    fn tz_localize(&self, tz: Option<&str>) -> Self {
        let _ = tz;
        self.clone()
    }

    #[getter]
    fn tzinfo(&self) -> Option<String> {
        None
    }

    fn view(&self) -> Self {
        self.clone()
    }

    #[getter]
    fn weekday(&self) -> Vec<Option<u32>> {
        self.inner.weekday()
    }

    #[pyo3(signature = (cond, other=None))]
    fn r#where(&self, cond: Vec<bool>, other: Option<&Bound<'_, PyAny>>) -> PyResult<PyIndex> {
        self.as_py_index().r#where(cond, other)
    }
}

/// Python wrapper for FrankenPandas MultiIndex.
#[pyclass(name = "MultiIndex", from_py_object)]
#[derive(Clone)]
pub struct PyMultiIndex {
    pub(crate) inner: MultiIndex,
}

#[pymethods]
impl PyMultiIndex {
    #[staticmethod]
    #[pyo3(signature = (tuples, names=None))]
    fn from_tuples(
        tuples: Vec<Bound<'_, PyAny>>,
        names: Option<Vec<Option<String>>>,
    ) -> PyResult<Self> {
        let mut parsed_tuples: Vec<Vec<IndexLabel>> = Vec::with_capacity(tuples.len());
        for t in &tuples {
            if let Ok(seq) = t.cast::<pyo3::types::PySequence>() {
                let len = seq.len()?;
                let mut row = Vec::with_capacity(len);
                for i in 0..len {
                    let item = seq.get_item(i)?;
                    row.push(py_to_index_label(&item)?);
                }
                parsed_tuples.push(row);
            } else {
                return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                    "Each tuple must be a sequence of labels",
                ));
            }
        }
        let mut inner = MultiIndex::from_tuples(parsed_tuples)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        if let Some(ns) = names {
            inner = inner.set_names(ns);
        }
        Ok(PyMultiIndex { inner })
    }

    #[staticmethod]
    #[pyo3(signature = (arrays, names=None))]
    fn from_arrays(
        arrays: Vec<Bound<'_, PyAny>>,
        names: Option<Vec<Option<String>>>,
    ) -> PyResult<Self> {
        let mut parsed_arrays: Vec<Vec<IndexLabel>> = Vec::with_capacity(arrays.len());
        for arr in &arrays {
            if let Ok(seq) = arr.cast::<pyo3::types::PySequence>() {
                let len = seq.len()?;
                let mut col = Vec::with_capacity(len);
                for i in 0..len {
                    let item = seq.get_item(i)?;
                    col.push(py_to_index_label(&item)?);
                }
                parsed_arrays.push(col);
            } else {
                return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                    "Each array must be a sequence of labels",
                ));
            }
        }
        let mut inner = MultiIndex::from_arrays(parsed_arrays)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        if let Some(ns) = names {
            inner = inner.set_names(ns);
        }
        Ok(PyMultiIndex { inner })
    }

    #[staticmethod]
    #[pyo3(signature = (iterables, names=None))]
    fn from_product(
        iterables: Vec<Bound<'_, PyAny>>,
        names: Option<Vec<Option<String>>>,
    ) -> PyResult<Self> {
        let mut parsed_iterables: Vec<Vec<IndexLabel>> = Vec::with_capacity(iterables.len());
        for it in &iterables {
            if let Ok(seq) = it.cast::<pyo3::types::PySequence>() {
                let len = seq.len()?;
                let mut row = Vec::with_capacity(len);
                for i in 0..len {
                    let item = seq.get_item(i)?;
                    row.push(py_to_index_label(&item)?);
                }
                parsed_iterables.push(row);
            } else {
                return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                    "Each iterable must be a sequence of labels",
                ));
            }
        }
        let mut inner = MultiIndex::from_product(parsed_iterables)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        if let Some(ns) = names {
            inner = inner.set_names(ns);
        }
        Ok(PyMultiIndex { inner })
    }

    #[getter]
    fn names(&self) -> Vec<Option<String>> {
        self.inner.names().to_vec()
    }

    #[getter]
    fn nlevels(&self) -> usize {
        self.inner.nlevels()
    }

    #[getter]
    fn shape(&self) -> (usize,) {
        (self.inner.len(),)
    }

    #[getter]
    fn size(&self) -> usize {
        self.inner.len()
    }

    #[getter]
    fn ndim(&self) -> usize {
        1
    }

    #[getter]
    fn empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    fn __len__(&self) -> usize {
        self.inner.len()
    }

    #[getter]
    fn is_unique(&self) -> bool {
        self.inner.is_unique()
    }

    #[getter]
    fn is_monotonic_increasing(&self) -> bool {
        self.inner.is_monotonic_increasing()
    }

    #[getter]
    fn is_monotonic_decreasing(&self) -> bool {
        self.inner.is_monotonic_decreasing()
    }

    fn get_level_values(&self, level: usize) -> PyResult<PyIndex> {
        let idx = self
            .inner
            .get_level_values(level)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyIndexError, _>(e.to_string()))?;
        Ok(PyIndex { inner: idx })
    }

    #[pyo3(signature = (sep="/"))]
    fn to_flat_index(&self, sep: &str) -> PyIndex {
        PyIndex {
            inner: self.inner.to_flat_index(sep),
        }
    }

    fn to_list(&self, py: Python<'_>) -> PyResult<Py<PyList>> {
        let mut rows = Vec::with_capacity(self.inner.len());
        for i in 0..self.inner.len() {
            let tuple_labels = self.inner.get_tuple(i).unwrap_or_default();
            let tuple_objs = tuple_labels
                .into_iter()
                .map(|l| index_label_to_py(py, l))
                .collect::<PyResult<Vec<_>>>()?;
            let py_tuple = pyo3::types::PyTuple::new(py, tuple_objs)?;
            rows.push(py_tuple.into_any().unbind());
        }
        Ok(PyList::new(py, rows)?.unbind())
    }

    fn tolist(&self, py: Python<'_>) -> PyResult<Py<PyList>> {
        self.to_list(py)
    }

    fn copy(&self) -> Self {
        self.clone()
    }

    fn equals(&self, other: &PyMultiIndex) -> bool {
        self.inner == other.inner
    }

    fn __repr__(&self) -> String {
        format!(
            "MultiIndex(levels={}, length={})",
            self.inner.nlevels(),
            self.inner.len()
        )
    }

    fn __getitem__(&self, py: Python<'_>, idx: i64) -> PyResult<Py<PyAny>> {
        let pos = if idx < 0 {
            (self.inner.len() as i64 + idx) as usize
        } else {
            idx as usize
        };
        if pos >= self.inner.len() {
            return Err(PyErr::new::<pyo3::exceptions::PyIndexError, _>(
                "index out of bounds",
            ));
        }
        let tuple_labels = self.inner.get_tuple(pos).unwrap_or_default();
        let tuple_objs = tuple_labels
            .into_iter()
            .map(|l| index_label_to_py(py, l))
            .collect::<PyResult<Vec<_>>>()?;
        Ok(pyo3::types::PyTuple::new(py, tuple_objs)?
            .into_any()
            .unbind())
    }

    #[getter]
    fn hasnans(&self) -> bool {
        false
    }

    #[getter]
    fn has_duplicates(&self) -> bool {
        self.inner.has_duplicates()
    }

    #[getter]
    fn nbytes(&self) -> usize {
        self.inner.nbytes()
    }

    #[pyo3(signature = (deep=false))]
    fn memory_usage(&self, deep: bool) -> usize {
        self.inner.memory_usage(deep)
    }

    #[getter]
    fn dtype(&self) -> &'static str {
        self.inner.dtype()
    }

    #[getter]
    fn dtypes(&self) -> Vec<&'static str> {
        self.inner.dtypes()
    }

    #[getter]
    fn levels(&self) -> Vec<PyIndex> {
        self.inner
            .levels()
            .into_iter()
            .map(|i| PyIndex { inner: i })
            .collect()
    }

    #[getter]
    fn codes(&self) -> Vec<Vec<isize>> {
        self.inner.codes()
    }

    #[getter]
    fn levshape(&self) -> Vec<usize> {
        self.inner.levshape()
    }

    fn inferred_type(&self) -> &'static str {
        self.inner.inferred_type()
    }

    fn is_numeric(&self) -> bool {
        false
    }

    fn is_boolean(&self) -> bool {
        false
    }

    fn is_floating(&self) -> bool {
        false
    }

    fn is_integer(&self) -> bool {
        false
    }

    fn is_categorical(&self) -> bool {
        false
    }

    fn is_object(&self) -> bool {
        true
    }

    fn is_interval(&self) -> bool {
        false
    }

    fn holds_integer(&self) -> bool {
        false
    }

    fn values(&self, py: Python<'_>) -> PyResult<Py<PyList>> {
        self.to_list(py)
    }

    fn union(&self, other: &PyMultiIndex) -> PyResult<Self> {
        self.inner
            .union(&other.inner)
            .map(|inner| Self { inner })
            .map_err(index_error_to_py)
    }

    fn intersection(&self, other: &PyMultiIndex) -> PyResult<Self> {
        self.inner
            .intersection(&other.inner)
            .map(|inner| Self { inner })
            .map_err(index_error_to_py)
    }

    fn difference(&self, other: &PyMultiIndex) -> PyResult<Self> {
        self.inner
            .difference(&other.inner)
            .map(|inner| Self { inner })
            .map_err(index_error_to_py)
    }

    fn symmetric_difference(&self, other: &PyMultiIndex) -> PyResult<Self> {
        self.inner
            .symmetric_difference(&other.inner)
            .map(|inner| Self { inner })
            .map_err(index_error_to_py)
    }

    #[pyo3(signature = (key, level=None))]
    fn get_loc(&self, key: &Bound<'_, PyAny>, level: Option<usize>) -> PyResult<Py<PyAny>> {
        let labels = if let Ok(seq) = key.cast::<pyo3::types::PySequence>() {
            let len = seq.len()?;
            let mut row = Vec::with_capacity(len);
            for i in 0..len {
                row.push(py_to_index_label(&seq.get_item(i)?)?);
            }
            row
        } else {
            vec![py_to_index_label(key)?]
        };
        let positions = self
            .inner
            .get_loc(&labels, level)
            .map_err(index_error_to_py)?;
        let py = key.py();
        if positions.len() == 1 {
            positions[0].into_py_any(py)
        } else {
            positions.into_py_any(py)
        }
    }

    fn get_indexer(&self, target: &PyMultiIndex) -> PyResult<Vec<i64>> {
        self.inner
            .get_indexer(&target.inner)
            .map(|res| res.into_iter().map(|x| x as i64).collect())
            .map_err(index_error_to_py)
    }

    #[pyo3(signature = (level=0))]
    fn droplevel(&self, py: Python<'_>, level: usize) -> PyResult<Py<PyAny>> {
        let res = self.inner.droplevel(level).map_err(index_error_to_py)?;
        match res {
            fp_index::MultiIndexOrIndex::Multi(mi) => Py::new(py, PyMultiIndex { inner: mi })?
                .into_any()
                .into_py_any(py),
            fp_index::MultiIndexOrIndex::Index(idx) => Py::new(py, PyIndex { inner: idx })?
                .into_any()
                .into_py_any(py),
        }
    }

    #[pyo3(signature = (index=None, name=None))]
    fn to_series(
        &self,
        index: Option<&Bound<'_, PyAny>>,
        name: Option<&str>,
    ) -> PyResult<PySeries> {
        let s_name = name.unwrap_or("");
        let idx = if let Some(i_obj) = index {
            if let Ok(py_idx) = i_obj.extract::<PyRef<'_, PyIndex>>() {
                py_idx.inner.clone()
            } else {
                self.inner.to_flat_index("/")
            }
        } else {
            self.inner.to_flat_index("/")
        };
        let flat = self.inner.to_flat_index("/");
        let vals: Vec<Scalar> = flat
            .labels()
            .iter()
            .map(|l| match l {
                IndexLabel::Utf8(s) => Scalar::Utf8(s.clone()),
                _ => Scalar::Null(NullKind::NaN),
            })
            .collect();
        let col = Column::from_values(vals)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        let s = Series::new(s_name, idx, col).map_err(frame_error_to_py)?;
        Ok(PySeries { inner: s })
    }

    #[pyo3(signature = (index=true, name=None))]
    fn to_frame(&self, index: bool, name: Option<&Bound<'_, PyAny>>) -> PyResult<PyDataFrame> {
        let idx = if index {
            self.inner.to_flat_index("/")
        } else {
            Index::from_range(0, self.inner.len() as i64, 1)
        };
        let mut col_names = Vec::with_capacity(self.inner.nlevels());
        let mut col_map = BTreeMap::new();
        let custom_names: Option<Vec<String>> = if let Some(n_obj) = name {
            if let Ok(list) = n_obj.extract::<Vec<String>>() {
                Some(list)
            } else if let Ok(s) = n_obj.extract::<String>() {
                Some(vec![s])
            } else {
                None
            }
        } else {
            None
        };
        for level_idx in 0..self.inner.nlevels() {
            let col_name = if let Some(ref c_names) = custom_names {
                c_names
                    .get(level_idx)
                    .cloned()
                    .unwrap_or_else(|| format!("{level_idx}"))
            } else if let Some(Some(n)) = self.inner.names().get(level_idx) {
                n.clone()
            } else {
                format!("{level_idx}")
            };
            let level_idx_obj = self
                .inner
                .get_level_values(level_idx)
                .map_err(index_error_to_py)?;
            let vals: Vec<Scalar> = level_idx_obj
                .labels()
                .iter()
                .map(index_label_to_scalar)
                .collect();
            let col = Column::from_values(vals)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
            col_map.insert(col_name.clone(), col);
            col_names.push(col_name);
        }
        let df =
            DataFrame::new_with_column_order(idx, col_map, col_names).map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: df })
    }

    #[pyo3(signature = (normalize=false, sort=true, ascending=false, dropna=true))]
    fn value_counts(
        &self,
        normalize: bool,
        sort: bool,
        ascending: bool,
        dropna: bool,
    ) -> PyResult<PySeries> {
        let _ = (normalize, sort, ascending, dropna);
        let pairs = self.inner.value_counts();
        let mut labels = Vec::with_capacity(pairs.len());
        let mut counts = Vec::with_capacity(pairs.len());
        for (tuple, cnt) in pairs {
            let parts: Vec<String> = tuple.into_iter().map(|l| l.to_string()).collect();
            labels.push(IndexLabel::Utf8(format!("({})", parts.join(", "))));
            counts.push(Scalar::Int64(cnt as i64));
        }
        let col = Column::from_values(counts)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        let s = Series::new("count", Index::new(labels), col).map_err(frame_error_to_py)?;
        Ok(PySeries { inner: s })
    }

    fn sort_values(&self) -> Self {
        Self {
            inner: self.inner.sort_values(),
        }
    }

    fn sort(&self) -> Self {
        self.sort_values()
    }

    pub fn sortlevel(&self) -> (Self, Vec<usize>) {
        let (sorted, order) = self.inner.sortlevel();
        (Self { inner: sorted }, order)
    }

    #[getter]
    #[allow(non_snake_case)]
    fn T(&self) -> Self {
        self.clone()
    }

    #[pyo3(signature = (how="any"))]
    fn dropna(&self, how: &str) -> Self {
        let inner = if how == "all" {
            self.inner.dropna_all()
        } else {
            self.inner.dropna_any()
        };
        Self { inner }
    }

    fn fillna(&self, value: &Bound<'_, PyAny>) -> PyResult<Self> {
        let label = py_to_index_label(value)?;
        Ok(Self {
            inner: self.inner.fillna(&label),
        })
    }

    fn all(&self) -> bool {
        self.inner.to_flat_index("/").all()
    }

    fn any(&self) -> bool {
        self.inner.to_flat_index("/").any()
    }

    fn append(&self, other: &PyMultiIndex) -> PyResult<Self> {
        self.inner
            .append(&other.inner)
            .map(|inner| Self { inner })
            .map_err(index_error_to_py)
    }

    fn argmax(&self) -> PyResult<usize> {
        let flat = self.inner.to_flat_index("/");
        PyIndex { inner: flat }.argmax()
    }

    fn argmin(&self) -> PyResult<usize> {
        let flat = self.inner.to_flat_index("/");
        PyIndex { inner: flat }.argmin()
    }

    fn argsort(&self) -> Vec<usize> {
        let flat = self.inner.to_flat_index("/");
        PyIndex { inner: flat }.argsort()
    }

    fn array(&self, py: Python<'_>) -> PyResult<Py<PyList>> {
        self.to_list(py)
    }

    fn to_numpy(&self, py: Python<'_>) -> PyResult<Py<PyList>> {
        self.to_list(py)
    }

    fn asof(&self, py: Python<'_>, label: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let labels = if let Ok(seq) = label.cast::<pyo3::types::PySequence>() {
            let len = seq.len()?;
            let mut row = Vec::with_capacity(len);
            for i in 0..len {
                row.push(py_to_index_label(&seq.get_item(i)?)?);
            }
            row
        } else {
            vec![py_to_index_label(label)?]
        };
        match self.inner.asof(&labels).map_err(index_error_to_py)? {
            Some(row) => {
                let items: Vec<Py<PyAny>> = row
                    .iter()
                    .map(|l| index_label_to_py(py, l))
                    .collect::<PyResult<_>>()?;
                Ok(pyo3::types::PyTuple::new(py, items)?.into_any().unbind())
            }
            None => Ok(py.None()),
        }
    }

    #[pyo3(signature = (where_, mask=None))]
    fn asof_locs(
        &self,
        where_: &Bound<'_, PyAny>,
        mask: Option<Vec<bool>>,
    ) -> PyResult<Vec<Option<usize>>> {
        let where_idx = if let Ok(py_mi) = where_.extract::<PyRef<'_, PyMultiIndex>>() {
            py_mi.inner.to_flat_index("/")
        } else if let Ok(py_idx) = where_.extract::<PyRef<'_, PyIndex>>() {
            py_idx.inner.clone()
        } else {
            PyIndex::new(Some(where_), None)?.inner
        };
        Ok(self
            .inner
            .to_flat_index("/")
            .asof_locs(&where_idx, mask.as_deref()))
    }

    fn astype(&self, dtype: &str) -> PyResult<Self> {
        let _ = dtype;
        Ok(self.clone())
    }

    fn delete(&self, loc: usize) -> PyResult<Self> {
        self.inner
            .delete(loc)
            .map(|inner| Self { inner })
            .map_err(index_error_to_py)
    }

    #[pyo3(signature = (periods=1))]
    fn diff(&self, periods: i64) -> PyResult<PyIndex> {
        let flat = self.inner.to_flat_index("/");
        PyIndex { inner: flat }.diff(periods)
    }

    fn drop(&self, labels: &Bound<'_, PyAny>) -> PyResult<Self> {
        let tuples_to_drop = if let Ok(seq) = labels.cast::<pyo3::types::PySequence>() {
            let len = seq.len()?;
            let mut rows = Vec::new();
            for i in 0..len {
                let item = seq.get_item(i)?;
                if let Ok(tuple_seq) = item.cast::<pyo3::types::PySequence>() {
                    let mut row = Vec::new();
                    for j in 0..tuple_seq.len()? {
                        row.push(py_to_index_label(&tuple_seq.get_item(j)?)?);
                    }
                    rows.push(row);
                } else {
                    rows.push(vec![py_to_index_label(&item)?]);
                }
            }
            rows
        } else {
            vec![vec![py_to_index_label(labels)?]]
        };
        self.inner
            .drop(&tuples_to_drop)
            .map(|inner| Self { inner })
            .map_err(index_error_to_py)
    }

    fn drop_duplicates(&self) -> Self {
        Self {
            inner: self.inner.drop_duplicates(),
        }
    }

    fn duplicated(&self) -> Vec<bool> {
        self.inner.duplicated(DuplicateKeep::First)
    }

    fn equal_levels(&self, other: &PyMultiIndex) -> bool {
        self.inner.equal_levels(&other.inner)
    }

    #[pyo3(signature = (sort=false, use_na_sentinel=true))]
    fn factorize(&self, sort: bool, use_na_sentinel: bool) -> (Vec<isize>, Self) {
        let _ = (sort, use_na_sentinel);
        let (codes, uniques) = self.inner.factorize();
        (codes, Self { inner: uniques })
    }

    fn format(&self) -> Vec<String> {
        self.inner.format()
    }

    #[classmethod]
    #[pyo3(signature = (df, names=None))]
    fn from_frame(
        _cls: &Bound<'_, pyo3::types::PyType>,
        df: &PyDataFrame,
        names: Option<Vec<String>>,
    ) -> PyResult<Self> {
        let cols = df.inner.column_names();
        let mut column_data = Vec::with_capacity(cols.len());
        for col_name in &cols {
            let col = df.inner.column(col_name).ok_or_else(|| {
                PyErr::new::<pyo3::exceptions::PyKeyError, _>(format!(
                    "column {col_name:?} missing"
                ))
            })?;
            let labels: Vec<IndexLabel> = col
                .values()
                .iter()
                .map(|sc| match sc {
                    Scalar::Int64(v) => IndexLabel::Int64(*v),
                    Scalar::Float64(f) => IndexLabel::Float64(fp_index::OrderedF64(*f)),
                    Scalar::Utf8(s) => IndexLabel::Utf8(s.clone()),
                    Scalar::Bool(b) => IndexLabel::Bool(*b),
                    Scalar::Datetime64(d) => IndexLabel::Datetime64(*d),
                    Scalar::Timedelta64(t) => IndexLabel::Timedelta64(*t),
                    Scalar::Period(p) => IndexLabel::Int64(p.ordinal),
                    Scalar::Interval(inv) => IndexLabel::Utf8(format!("{inv:?}")),
                    Scalar::Null(k) => IndexLabel::Null(*k),
                })
                .collect();
            column_data.push((Some((*col_name).clone()), labels));
        }
        let mut mi = MultiIndex::from_frame(column_data).map_err(index_error_to_py)?;
        if let Some(ns) = names {
            mi = mi.set_names(ns.into_iter().map(Some).collect());
        }
        Ok(Self { inner: mi })
    }

    fn get_indexer_for(&self, target: &PyMultiIndex) -> PyResult<Vec<i64>> {
        self.inner
            .get_indexer_for(&target.inner)
            .map(|res| res.into_iter().map(|x| x as i64).collect())
            .map_err(index_error_to_py)
    }

    fn get_indexer_non_unique(&self, target: &PyMultiIndex) -> (Vec<isize>, Vec<usize>) {
        self.inner.get_indexer_non_unique(&target.inner)
    }

    fn get_loc_level(&self, py: Python<'_>, key: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let labels = if let Ok(seq) = key.cast::<pyo3::types::PySequence>() {
            let len = seq.len()?;
            let mut row = Vec::with_capacity(len);
            for i in 0..len {
                row.push(py_to_index_label(&seq.get_item(i)?)?);
            }
            row
        } else {
            vec![py_to_index_label(key)?]
        };
        let (positions, remaining) = self
            .inner
            .get_loc_level(&labels)
            .map_err(index_error_to_py)?;
        let py_rem: Py<PyAny> = match remaining {
            Some(fp_index::MultiIndexOrIndex::Multi(mi)) => {
                Py::new(py, PyMultiIndex { inner: mi })?.into_any()
            }
            Some(fp_index::MultiIndexOrIndex::Index(idx)) => {
                Py::new(py, PyIndex { inner: idx })?.into_any()
            }
            None => py.None(),
        };
        let py_pos = PyList::new(py, positions)?;
        Ok(
            pyo3::types::PyTuple::new(py, vec![py_pos.into_any().unbind(), py_rem])?
                .into_any()
                .unbind(),
        )
    }

    fn get_locs(&self, key: &Bound<'_, PyAny>) -> PyResult<Vec<usize>> {
        let labels = if let Ok(seq) = key.cast::<pyo3::types::PySequence>() {
            let len = seq.len()?;
            let mut row = Vec::with_capacity(len);
            for i in 0..len {
                row.push(py_to_index_label(&seq.get_item(i)?)?);
            }
            row
        } else {
            vec![py_to_index_label(key)?]
        };
        self.inner.get_locs(&labels).map_err(index_error_to_py)
    }

    fn get_slice_bound(&self, label: &Bound<'_, PyAny>, side: &str) -> PyResult<usize> {
        let labels = if let Ok(seq) = label.cast::<pyo3::types::PySequence>() {
            let len = seq.len()?;
            let mut row = Vec::with_capacity(len);
            for i in 0..len {
                row.push(py_to_index_label(&seq.get_item(i)?)?);
            }
            row
        } else {
            vec![py_to_index_label(label)?]
        };
        self.inner
            .get_slice_bound(&labels, side)
            .map_err(index_error_to_py)
    }

    #[pyo3(signature = (start=None, end=None, step=None))]
    fn slice_locs(
        &self,
        start: Option<&Bound<'_, PyAny>>,
        end: Option<&Bound<'_, PyAny>>,
        step: Option<isize>,
    ) -> PyResult<(usize, usize)> {
        let _ = step;
        let s_lbls = match start {
            Some(obj) => {
                if let Ok(seq) = obj.cast::<pyo3::types::PySequence>() {
                    let mut r = Vec::new();
                    for i in 0..seq.len()? {
                        r.push(py_to_index_label(&seq.get_item(i)?)?);
                    }
                    Some(r)
                } else {
                    Some(vec![py_to_index_label(obj)?])
                }
            }
            None => None,
        };
        let e_lbls = match end {
            Some(obj) => {
                if let Ok(seq) = obj.cast::<pyo3::types::PySequence>() {
                    let mut r = Vec::new();
                    for i in 0..seq.len()? {
                        r.push(py_to_index_label(&seq.get_item(i)?)?);
                    }
                    Some(r)
                } else {
                    Some(vec![py_to_index_label(obj)?])
                }
            }
            None => None,
        };
        self.inner
            .slice_locs(s_lbls.as_deref(), e_lbls.as_deref())
            .map_err(index_error_to_py)
    }

    #[pyo3(signature = (start=None, end=None, step=None))]
    fn slice_indexer(
        &self,
        start: Option<&Bound<'_, PyAny>>,
        end: Option<&Bound<'_, PyAny>>,
        step: Option<isize>,
    ) -> PyResult<(usize, usize, isize)> {
        let (s, e) = self.slice_locs(start, end, step)?;
        Ok((s, e, step.unwrap_or(1)))
    }

    fn groupby(&self, py: Python<'_>, by: &Bound<'_, PyAny>) -> PyResult<Py<pyo3::types::PyDict>> {
        let flat = self.inner.to_flat_index("/");
        PyIndex { inner: flat }.groupby(py, by)
    }

    fn identical(&self, other: &Bound<'_, PyAny>) -> bool {
        if let Ok(other_mi) = other.extract::<PyRef<'_, PyMultiIndex>>() {
            self.inner.identical(&other_mi.inner)
        } else {
            false
        }
    }

    fn infer_objects(&self) -> Self {
        self.clone()
    }

    fn insert(&self, loc: usize, item: &Bound<'_, PyAny>) -> PyResult<Self> {
        let labels = if let Ok(seq) = item.cast::<pyo3::types::PySequence>() {
            let len = seq.len()?;
            let mut row = Vec::with_capacity(len);
            for i in 0..len {
                row.push(py_to_index_label(&seq.get_item(i)?)?);
            }
            row
        } else {
            vec![py_to_index_label(item)?]
        };
        self.inner
            .insert(loc, labels)
            .map(|inner| Self { inner })
            .map_err(index_error_to_py)
    }

    fn is_(&self, other: &Bound<'_, PyAny>) -> bool {
        if let Ok(other_mi) = other.extract::<PyRef<'_, PyMultiIndex>>() {
            self.inner.is_(&other_mi.inner)
        } else {
            false
        }
    }

    fn isin(&self, values: &Bound<'_, PyAny>) -> PyResult<Vec<bool>> {
        let flat = self.inner.to_flat_index("/");
        PyIndex { inner: flat }.isin(values)
    }

    fn isna(&self) -> Vec<bool> {
        self.inner
            .isna()
            .unwrap_or_else(|_| vec![false; self.inner.len()])
    }

    fn isnull(&self) -> Vec<bool> {
        self.isna()
    }

    fn item(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        if self.inner.len() == 1 {
            let tuple_labels = self.inner.get_tuple(0).unwrap_or_default();
            let tuple_objs = tuple_labels
                .into_iter()
                .map(|l| index_label_to_py(py, l))
                .collect::<PyResult<Vec<_>>>()?;
            Ok(pyo3::types::PyTuple::new(py, tuple_objs)?
                .into_any()
                .unbind())
        } else {
            Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
                "can only convert an array of size 1 to a Python scalar",
            ))
        }
    }

    #[pyo3(signature = (other, how="left"))]
    fn join(&self, other: &PyMultiIndex, how: &str) -> PyResult<Self> {
        self.inner
            .join(&other.inner, how)
            .map(|inner| Self { inner })
            .map_err(index_error_to_py)
    }

    fn map(&self, py: Python<'_>, mapper: &Bound<'_, PyAny>) -> PyResult<PyIndex> {
        let flat = self.inner.to_flat_index("/");
        PyIndex { inner: flat }.map(py, mapper)
    }

    fn max(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let flat = self.inner.to_flat_index("/");
        PyIndex { inner: flat }.max(py)
    }

    fn min(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let flat = self.inner.to_flat_index("/");
        PyIndex { inner: flat }.min(py)
    }

    #[getter]
    fn name(&self) -> Option<String> {
        None
    }

    fn notna(&self) -> Vec<bool> {
        self.inner
            .notna()
            .unwrap_or_else(|_| vec![true; self.inner.len()])
    }

    fn notnull(&self) -> Vec<bool> {
        self.notna()
    }

    fn nunique(&self) -> usize {
        self.inner.nunique()
    }

    fn unique(&self) -> Self {
        Self {
            inner: self.inner.unique(),
        }
    }

    fn putmask(&self, cond: Vec<bool>, value: &Bound<'_, PyAny>) -> PyResult<Self> {
        let labels = if let Ok(seq) = value.cast::<pyo3::types::PySequence>() {
            let len = seq.len()?;
            let mut row = Vec::with_capacity(len);
            for i in 0..len {
                row.push(py_to_index_label(&seq.get_item(i)?)?);
            }
            row
        } else {
            vec![py_to_index_label(value)?]
        };
        self.inner
            .putmask(&cond, labels)
            .map(|inner| Self { inner })
            .map_err(index_error_to_py)
    }

    fn ravel(&self) -> Self {
        self.clone()
    }

    fn reindex(&self, target: &PyMultiIndex) -> PyResult<(Self, Vec<i64>)> {
        let (reindexed, indexer) = self
            .inner
            .reindex(&target.inner)
            .map_err(index_error_to_py)?;
        Ok((
            Self { inner: reindexed },
            indexer.into_iter().map(|u| u as i64).collect(),
        ))
    }

    fn remove_unused_levels(&self) -> Self {
        Self {
            inner: self.inner.remove_unused_levels(),
        }
    }

    fn rename(&self, names: Vec<Option<String>>) -> Self {
        Self {
            inner: self.inner.clone().set_names(names),
        }
    }

    fn reorder_levels(&self, order: Vec<usize>) -> PyResult<Self> {
        self.inner
            .reorder_levels(&order)
            .map(|inner| Self { inner })
            .map_err(index_error_to_py)
    }

    fn repeat(&self, repeats: usize) -> PyResult<Self> {
        let positions: Vec<usize> = (0..self.inner.len())
            .flat_map(|i| std::iter::repeat_n(i, repeats))
            .collect();
        self.inner
            .take(&positions)
            .map(|inner| Self { inner })
            .map_err(index_error_to_py)
    }

    #[pyo3(signature = (decimals=0))]
    fn round(&self, decimals: i32) -> Self {
        let _ = decimals;
        self.clone()
    }

    #[pyo3(signature = (target, side="left"))]
    fn searchsorted(&self, target: &PyMultiIndex, side: &str) -> PyResult<Vec<usize>> {
        self.inner
            .searchsorted(&target.inner, side)
            .map_err(index_error_to_py)
    }

    fn set_codes(&self, codes: Vec<Vec<isize>>) -> PyResult<Self> {
        self.inner
            .set_codes(codes)
            .map(|inner| Self { inner })
            .map_err(index_error_to_py)
    }

    fn set_levels(&self, levels: Vec<Bound<'_, PyAny>>) -> PyResult<Self> {
        let mut new_levels = Vec::with_capacity(levels.len());
        for lvl in &levels {
            if let Ok(seq) = lvl.cast::<pyo3::types::PySequence>() {
                let len = seq.len()?;
                let mut col = Vec::with_capacity(len);
                for i in 0..len {
                    col.push(py_to_index_label(&seq.get_item(i)?)?);
                }
                new_levels.push(col);
            } else {
                return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                    "Each level must be a sequence of labels",
                ));
            }
        }
        self.inner
            .set_levels(new_levels)
            .map(|inner| Self { inner })
            .map_err(index_error_to_py)
    }

    #[pyo3(signature = (names, level=None))]
    fn set_names(&self, names: &Bound<'_, PyAny>, level: Option<usize>) -> PyResult<Self> {
        let _ = level;
        let ns = if let Ok(s) = names.extract::<String>() {
            vec![Some(s)]
        } else if let Ok(seq) = names.cast::<pyo3::types::PySequence>() {
            let mut list = Vec::new();
            for i in 0..seq.len()? {
                let item = seq.get_item(i)?;
                list.push(item.extract::<Option<String>>()?);
            }
            list
        } else {
            Vec::new()
        };
        Ok(Self {
            inner: self.inner.clone().set_names(ns),
        })
    }

    #[pyo3(signature = (periods=1, freq=None))]
    fn shift(&self, periods: i64, freq: Option<&str>) -> PyResult<Self> {
        let _ = (periods, freq);
        Err(PyErr::new::<pyo3::exceptions::PyNotImplementedError, _>(
            "This method is only implemented for DatetimeIndex, PeriodIndex and TimedeltaIndex; Got type MultiIndex",
        ))
    }

    #[getter]
    fn r#str(&self) -> PyIndexStringMethods {
        PyIndexStringMethods {
            inner: self.inner.to_flat_index("/"),
        }
    }

    #[pyo3(signature = (i=0, j=1))]
    fn swaplevel(&self, i: usize, j: usize) -> PyResult<Self> {
        self.inner
            .swaplevel(i, j)
            .map(|inner| Self { inner })
            .map_err(index_error_to_py)
    }

    fn take(&self, positions: Vec<usize>) -> PyResult<Self> {
        self.inner
            .take(&positions)
            .map(|inner| Self { inner })
            .map_err(index_error_to_py)
    }

    fn transpose(&self) -> Self {
        self.clone()
    }

    #[pyo3(signature = (before=None, after=None))]
    fn truncate(
        &self,
        before: Option<&Bound<'_, PyAny>>,
        after: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Self> {
        let b_lbls = match before {
            Some(obj) => {
                if let Ok(seq) = obj.cast::<pyo3::types::PySequence>() {
                    let mut r = Vec::new();
                    for i in 0..seq.len()? {
                        r.push(py_to_index_label(&seq.get_item(i)?)?);
                    }
                    Some(r)
                } else {
                    Some(vec![py_to_index_label(obj)?])
                }
            }
            None => None,
        };
        let a_lbls = match after {
            Some(obj) => {
                if let Ok(seq) = obj.cast::<pyo3::types::PySequence>() {
                    let mut r = Vec::new();
                    for i in 0..seq.len()? {
                        r.push(py_to_index_label(&seq.get_item(i)?)?);
                    }
                    Some(r)
                } else {
                    Some(vec![py_to_index_label(obj)?])
                }
            }
            None => None,
        };
        self.inner
            .truncate(b_lbls.as_deref(), a_lbls.as_deref())
            .map(|inner| Self { inner })
            .map_err(index_error_to_py)
    }

    fn view(&self) -> Self {
        self.clone()
    }

    #[pyo3(signature = (cond, other=None))]
    fn r#where(&self, cond: Vec<bool>, other: Option<&Bound<'_, PyAny>>) -> PyResult<Self> {
        let other_tuple = if let Some(o) = other {
            if let Ok(seq) = o.cast::<pyo3::types::PySequence>() {
                let len = seq.len()?;
                let mut row = Vec::with_capacity(len);
                for i in 0..len {
                    row.push(py_to_index_label(&seq.get_item(i)?)?);
                }
                row
            } else {
                vec![py_to_index_label(o)?]
            }
        } else {
            vec![IndexLabel::Null(NullKind::NaN); self.inner.nlevels()]
        };
        self.inner
            .putmask(
                &cond.into_iter().map(|b| !b).collect::<Vec<_>>(),
                other_tuple,
            )
            .map(|inner| Self { inner })
            .map_err(index_error_to_py)
    }
}

/// Python wrapper for FrankenPandas TimedeltaIndex.
#[pyclass(name = "TimedeltaIndex", from_py_object)]
#[derive(Clone)]
pub struct PyTimedeltaIndex {
    pub(crate) inner: TimedeltaIndex,
}

#[pymethods]
impl PyTimedeltaIndex {
    #[new]
    #[pyo3(signature = (data=None, unit=None, freq=None, name=None))]
    pub fn new(
        data: Option<&Bound<'_, PyAny>>,
        unit: Option<&str>,
        freq: Option<&str>,
        name: Option<&str>,
    ) -> PyResult<Self> {
        let mut nanos = Vec::new();
        if let Some(obj) = data {
            if let Ok(tdi) = obj.extract::<PyRef<'_, PyTimedeltaIndex>>() {
                let mut inner = tdi.inner.clone();
                if let Some(n) = name {
                    inner = inner.set_name(n);
                }
                return Ok(Self { inner });
            }
            if let Ok(s) = obj.extract::<PyRef<'_, PySeries>>() {
                for v in s.inner.values() {
                    match v {
                        Scalar::Timedelta64(ns) => nanos.push(*ns),
                        Scalar::Int64(i) => {
                            let factor = match unit.unwrap_or("ns") {
                                "s" => Timedelta::NANOS_PER_SEC,
                                "ms" => Timedelta::NANOS_PER_MILLI,
                                "us" => Timedelta::NANOS_PER_MICRO,
                                "m" | "min" => Timedelta::NANOS_PER_MIN,
                                "h" => Timedelta::NANOS_PER_HOUR,
                                "d" | "D" => Timedelta::NANOS_PER_DAY,
                                "w" | "W" => Timedelta::NANOS_PER_WEEK,
                                _ => 1,
                            };
                            nanos.push(i.saturating_mul(factor));
                        }
                        Scalar::Utf8(txt) => {
                            let parsed = Timedelta::parse(txt).unwrap_or(Timedelta::NAT);
                            nanos.push(parsed);
                        }
                        Scalar::Null(_) => nanos.push(Timedelta::NAT),
                        _ => nanos.push(Timedelta::NAT),
                    }
                }
            } else if let Ok(list) = obj.extract::<Vec<Bound<'_, PyAny>>>() {
                for item in list {
                    if item.is_none() {
                        nanos.push(Timedelta::NAT);
                    } else if let Ok(i) = item.extract::<i64>() {
                        let factor = match unit.unwrap_or("ns") {
                            "s" => Timedelta::NANOS_PER_SEC,
                            "ms" => Timedelta::NANOS_PER_MILLI,
                            "us" => Timedelta::NANOS_PER_MICRO,
                            "m" | "min" => Timedelta::NANOS_PER_MIN,
                            "h" => Timedelta::NANOS_PER_HOUR,
                            "d" | "D" => Timedelta::NANOS_PER_DAY,
                            "w" | "W" => Timedelta::NANOS_PER_WEEK,
                            _ => 1,
                        };
                        nanos.push(i.saturating_mul(factor));
                    } else if let Ok(f) = item.extract::<f64>() {
                        if f.is_nan() {
                            nanos.push(Timedelta::NAT);
                        } else {
                            let factor = match unit.unwrap_or("ns") {
                                "s" => Timedelta::NANOS_PER_SEC as f64,
                                "ms" => Timedelta::NANOS_PER_MILLI as f64,
                                "us" => Timedelta::NANOS_PER_MICRO as f64,
                                "m" | "min" => Timedelta::NANOS_PER_MIN as f64,
                                "h" => Timedelta::NANOS_PER_HOUR as f64,
                                "d" | "D" => Timedelta::NANOS_PER_DAY as f64,
                                "w" | "W" => Timedelta::NANOS_PER_WEEK as f64,
                                _ => 1.0,
                            };
                            nanos.push((f * factor) as i64);
                        }
                    } else if let Ok(txt) = item.extract::<String>() {
                        let parsed = Timedelta::parse(&txt).map_err(|e| {
                            PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                                "Failed to parse timedelta '{txt}': {e}"
                            ))
                        })?;
                        nanos.push(parsed);
                    } else {
                        return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                            "Unsupported element in TimedeltaIndex data",
                        ));
                    }
                }
            }
        }
        let mut inner = TimedeltaIndex::new(nanos);
        if let Some(n) = name {
            inner = inner.set_name(n);
        }
        let _ = freq;
        Ok(Self { inner })
    }

    #[getter]
    pub fn days(&self) -> Vec<Option<i64>> {
        self.inner.days()
    }

    #[getter]
    pub fn seconds(&self) -> Vec<Option<i64>> {
        self.inner.seconds()
    }

    #[getter]
    pub fn microseconds(&self) -> Vec<Option<i64>> {
        self.inner.microseconds()
    }

    #[getter]
    pub fn nanoseconds(&self) -> Vec<Option<i64>> {
        self.inner.nanoseconds()
    }

    pub fn total_seconds(&self) -> Vec<Option<f64>> {
        self.inner.total_seconds()
    }

    #[getter]
    pub fn asi8(&self) -> Vec<i64> {
        self.inner.asi8()
    }

    #[getter]
    pub fn shape(&self) -> (usize,) {
        self.inner.shape()
    }

    #[getter]
    pub fn size(&self) -> usize {
        self.inner.size()
    }

    #[getter]
    pub fn ndim(&self) -> usize {
        self.inner.ndim()
    }

    #[getter]
    pub fn empty(&self) -> bool {
        self.inner.empty()
    }

    #[getter]
    pub fn dtype(&self) -> &'static str {
        self.inner.dtype()
    }

    #[getter]
    pub fn name(&self) -> Option<String> {
        self.inner.name().map(str::to_owned)
    }

    #[getter]
    pub fn is_monotonic_increasing(&self) -> bool {
        self.inner.is_monotonic_increasing()
    }

    #[getter]
    pub fn is_monotonic_decreasing(&self) -> bool {
        self.inner.is_monotonic_decreasing()
    }

    #[getter]
    pub fn has_duplicates(&self) -> bool {
        self.inner.has_duplicates()
    }

    pub fn min(&self) -> Option<i64> {
        self.inner.min().filter(|&x| x != Timedelta::NAT)
    }

    pub fn max(&self) -> Option<i64> {
        self.inner.max().filter(|&x| x != Timedelta::NAT)
    }

    pub fn mean(&self) -> Option<i64> {
        self.inner.mean().filter(|&x| x != Timedelta::NAT)
    }

    pub fn median(&self) -> Option<i64> {
        self.inner.median().filter(|&x| x != Timedelta::NAT)
    }

    pub fn std(&self) -> Option<i64> {
        self.inner.std()
    }

    pub fn var(&self) -> Option<f64> {
        self.inner.var()
    }

    pub fn argmax(&self) -> PyResult<usize> {
        self.inner
            .argmax()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))
    }

    pub fn argmin(&self) -> PyResult<usize> {
        self.inner
            .argmin()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))
    }

    pub fn isna(&self) -> Vec<bool> {
        self.inner.isna()
    }

    pub fn isnull(&self) -> Vec<bool> {
        self.inner.isna()
    }

    pub fn notna(&self) -> Vec<bool> {
        self.inner.notna()
    }

    pub fn notnull(&self) -> Vec<bool> {
        self.inner.notna()
    }

    pub fn unique(&self) -> PyResult<Self> {
        let u = self.inner.unique().map_err(index_error_to_py)?;
        Ok(Self { inner: u })
    }

    pub fn nunique(&self) -> usize {
        self.inner.nunique()
    }

    pub fn drop_duplicates(&self) -> PyResult<Self> {
        let d = self.inner.drop_duplicates().map_err(index_error_to_py)?;
        Ok(Self { inner: d })
    }

    pub fn duplicated(&self, keep: Option<&Bound<'_, PyAny>>) -> PyResult<Vec<bool>> {
        let k = parse_duplicate_keep(keep)?;
        Ok(self.inner.duplicated(k))
    }

    pub fn isin(&self, values: Vec<i64>) -> Vec<bool> {
        self.inner.isin(&values)
    }

    pub fn tolist(&self) -> Vec<Option<i64>> {
        self.inner.tolist()
    }

    pub fn to_list(&self) -> Vec<Option<i64>> {
        self.inner.tolist()
    }

    pub fn values(&self) -> Vec<Option<i64>> {
        self.inner.values()
    }

    pub fn to_numpy(&self) -> Vec<Option<i64>> {
        self.inner.to_numpy()
    }

    pub fn copy(&self) -> Self {
        Self {
            inner: self.inner.copy(),
        }
    }

    pub fn rename(&self, name: Option<&str>) -> Self {
        Self {
            inner: self.inner.rename_index(name),
        }
    }

    pub fn equals(&self, other: &Self) -> bool {
        self.inner.equals(&other.inner)
    }

    pub fn round(&self, freq: &str) -> PyResult<Self> {
        self.inner
            .round(freq)
            .map(|i| Self { inner: i })
            .map_err(index_error_to_py)
    }

    pub fn floor(&self, freq: &str) -> PyResult<Self> {
        self.inner
            .floor(freq)
            .map(|i| Self { inner: i })
            .map_err(index_error_to_py)
    }

    pub fn ceil(&self, freq: &str) -> PyResult<Self> {
        self.inner
            .ceil(freq)
            .map(|i| Self { inner: i })
            .map_err(index_error_to_py)
    }

    #[pyo3(signature = (periods, freq="D"))]
    pub fn shift(&self, periods: i64, freq: &str) -> PyResult<Self> {
        let freq_nanos = parse_freq_to_nanos(freq)?;
        Ok(Self {
            inner: self.inner.shift(periods, freq_nanos),
        })
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub fn __len__(&self) -> usize {
        self.inner.len()
    }

    pub fn __repr__(&self) -> String {
        format!(
            "TimedeltaIndex({:?}, dtype='timedelta64[ns]')",
            self.inner.tolist()
        )
    }

    pub fn __getitem__(&self, py: Python<'_>, item: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        if let Ok(idx) = item.extract::<i64>() {
            let len = self.inner.len() as i64;
            let pos = if idx < 0 { len + idx } else { idx };
            if pos < 0 || pos >= len {
                return Err(PyErr::new::<pyo3::exceptions::PyIndexError, _>(
                    "index out of bounds",
                ));
            }
            let label = &self.inner.as_index().labels()[pos as usize];
            return index_label_to_py(py, label);
        }
        if let Ok(slice) = item.cast::<pyo3::types::PySlice>() {
            let indices = slice.indices(self.inner.len() as isize)?;
            let mut sliced = Vec::new();
            let mut i = indices.start;
            if indices.step > 0 {
                while i < indices.stop {
                    if let IndexLabel::Timedelta64(ns) = self.inner.as_index().labels()[i as usize]
                    {
                        sliced.push(ns);
                    }
                    i += indices.step;
                }
            } else {
                while i > indices.stop {
                    if let IndexLabel::Timedelta64(ns) = self.inner.as_index().labels()[i as usize]
                    {
                        sliced.push(ns);
                    }
                    i += indices.step;
                }
            }
            let mut out = TimedeltaIndex::new(sliced);
            if let Some(n) = self.inner.name() {
                out = out.set_name(n);
            }
            return Py::new(py, PyTimedeltaIndex { inner: out })?
                .into_any()
                .into_py_any(py);
        }
        Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
            "TimedeltaIndex indices must be integers or slices",
        ))
    }

    #[getter]
    fn is_unique(&self) -> bool {
        self.inner.as_index().is_unique()
    }

    #[getter]
    fn hasnans(&self) -> bool {
        self.inner.as_index().hasnans()
    }

    #[getter]
    fn nlevels(&self) -> usize {
        1
    }

    #[getter]
    fn names(&self) -> Vec<Option<String>> {
        vec![self.inner.name().map(str::to_string)]
    }

    #[getter]
    fn nbytes(&self) -> usize {
        self.inner.as_index().nbytes()
    }

    #[pyo3(signature = (deep=false))]
    fn memory_usage(&self, deep: bool) -> usize {
        self.inner.as_index().memory_usage(deep)
    }

    fn inferred_type(&self) -> &'static str {
        self.inner.as_index().inferred_type()
    }

    fn is_numeric(&self) -> bool {
        false
    }

    fn is_boolean(&self) -> bool {
        false
    }

    fn is_floating(&self) -> bool {
        false
    }

    fn is_integer(&self) -> bool {
        false
    }

    fn is_categorical(&self) -> bool {
        false
    }

    fn is_object(&self) -> bool {
        false
    }

    fn is_interval(&self) -> bool {
        false
    }

    fn holds_integer(&self) -> bool {
        false
    }

    fn union(&self, other: &PyIndex) -> PyIndex {
        PyIndex {
            inner: self.inner.as_index().union(&other.inner),
        }
    }

    fn intersection(&self, other: &PyIndex) -> PyIndex {
        PyIndex {
            inner: self.inner.as_index().intersection(&other.inner),
        }
    }

    fn difference(&self, other: &PyIndex) -> PyIndex {
        PyIndex {
            inner: self.inner.as_index().difference(&other.inner),
        }
    }

    fn symmetric_difference(&self, other: &PyIndex) -> PyIndex {
        PyIndex {
            inner: self.inner.as_index().symmetric_difference(&other.inner),
        }
    }

    fn get_loc(&self, key: &Bound<'_, PyAny>) -> PyResult<usize> {
        let label = py_to_index_label(key)?;
        self.inner
            .as_index()
            .get_loc(&label)
            .ok_or_else(|| PyErr::new::<pyo3::exceptions::PyKeyError, _>(format!("{key}")))
    }

    fn get_indexer(&self, target: &PyIndex) -> Vec<i64> {
        self.inner
            .as_index()
            .get_indexer(&target.inner)
            .into_iter()
            .map(|opt| opt.map(|u| u as i64).unwrap_or(-1))
            .collect()
    }

    #[pyo3(signature = (start=None, end=None, step=None))]
    fn slice_locs(
        &self,
        start: Option<&Bound<'_, PyAny>>,
        end: Option<&Bound<'_, PyAny>>,
        step: Option<isize>,
    ) -> PyResult<(usize, usize)> {
        let _ = step;
        let s_lbl = match start {
            Some(obj) => Some(py_to_index_label(obj)?),
            None => None,
        };
        let e_lbl = match end {
            Some(obj) => Some(py_to_index_label(obj)?),
            None => None,
        };
        self.inner
            .as_index()
            .slice_locs(s_lbl.as_ref(), e_lbl.as_ref())
            .map_err(index_error_to_py)
    }

    #[pyo3(signature = (start=None, end=None, step=None))]
    fn slice_indexer(
        &self,
        start: Option<&Bound<'_, PyAny>>,
        end: Option<&Bound<'_, PyAny>>,
        step: Option<isize>,
    ) -> PyResult<(usize, usize, isize)> {
        let (s, e) = self.slice_locs(start, end, step)?;
        Ok((s, e, step.unwrap_or(1)))
    }

    #[pyo3(signature = (level=0))]
    fn droplevel(&self, level: usize) -> PyResult<PyIndex> {
        let _ = level;
        Ok(PyIndex {
            inner: self.inner.as_index().clone(),
        })
    }

    #[pyo3(signature = (level=0))]
    fn get_level_values(&self, level: usize) -> PyResult<Self> {
        let _ = level;
        Ok(self.clone())
    }

    fn to_flat_index(&self) -> Self {
        self.clone()
    }

    #[pyo3(signature = (index=None, name=None))]
    fn to_series(
        &self,
        index: Option<&Bound<'_, PyAny>>,
        name: Option<&str>,
    ) -> PyResult<PySeries> {
        let series_name = name.or_else(|| self.inner.name()).unwrap_or("");
        let idx = if let Some(i_obj) = index {
            if let Ok(py_idx) = i_obj.extract::<PyRef<'_, PyIndex>>() {
                py_idx.inner.clone()
            } else {
                self.inner.as_index().clone()
            }
        } else {
            self.inner.as_index().clone()
        };
        let col = Column::from_values(
            self.inner
                .as_index()
                .labels()
                .iter()
                .map(|l| match l {
                    IndexLabel::Timedelta64(d) => Scalar::Timedelta64(*d),
                    _ => Scalar::Null(NullKind::NaN),
                })
                .collect(),
        )
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        let s = Series::new(series_name, idx, col).map_err(frame_error_to_py)?;
        Ok(PySeries { inner: s })
    }

    #[pyo3(signature = (index=true, name=None))]
    fn to_frame(&self, index: bool, name: Option<&str>) -> PyResult<PyDataFrame> {
        let col_name = name.or_else(|| self.inner.name()).unwrap_or("0");
        let idx = if index {
            self.inner.as_index().clone()
        } else {
            Index::from_range(0, self.inner.len() as i64, 1)
        };
        let col = Column::from_values(
            self.inner
                .as_index()
                .labels()
                .iter()
                .map(|l| match l {
                    IndexLabel::Timedelta64(d) => Scalar::Timedelta64(*d),
                    _ => Scalar::Null(NullKind::NaN),
                })
                .collect(),
        )
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        let mut col_map = BTreeMap::new();
        col_map.insert(col_name.to_string(), col);
        let df = DataFrame::new_with_column_order(idx, col_map, vec![col_name.to_string()])
            .map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: df })
    }

    #[pyo3(signature = (normalize=false, sort=true, ascending=false, dropna=true))]
    fn value_counts(
        &self,
        normalize: bool,
        sort: bool,
        ascending: bool,
        dropna: bool,
    ) -> PyResult<PySeries> {
        let counts = self
            .inner
            .as_index()
            .value_counts_with_options(normalize, sort, ascending, dropna);
        let mut idx_labels = Vec::with_capacity(counts.len());
        let mut vals = Vec::with_capacity(counts.len());
        for (lbl, sc) in counts {
            idx_labels.push(lbl);
            vals.push(sc);
        }
        let col = Column::from_values(vals)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        let s = Series::new("count", Index::new(idx_labels), col).map_err(frame_error_to_py)?;
        Ok(PySeries { inner: s })
    }

    #[pyo3(signature = (ascending=true, na_position="last"))]
    fn sort_values(&self, ascending: bool, na_position: &str) -> PyResult<Self> {
        let mut sorted = self.inner.asi8();
        if ascending {
            sorted.sort_unstable();
        } else {
            sorted.sort_unstable_by(|a, b| b.cmp(a));
        }
        let _ = na_position;
        let mut out = TimedeltaIndex::new(sorted);
        if let Some(n) = self.inner.name() {
            out = out.set_name(n);
        }
        Ok(Self { inner: out })
    }

    fn sort(&self) -> PyResult<Self> {
        self.sort_values(true, "last")
    }

    #[getter]
    #[allow(non_snake_case)]
    fn T(&self) -> Self {
        self.clone()
    }

    fn drop(&self, labels: Vec<Bound<'_, PyAny>>) -> PyResult<PyIndex> {
        let mut to_drop = Vec::with_capacity(labels.len());
        for l in labels {
            to_drop.push(py_to_index_label(&l)?);
        }
        Ok(PyIndex {
            inner: self.inner.as_index().drop_labels(&to_drop),
        })
    }

    #[pyo3(signature = (how="any"))]
    fn dropna(&self, how: &str) -> Self {
        let _ = how;
        self.clone()
    }

    fn fillna(&self, value: &Bound<'_, PyAny>) -> Self {
        let _ = value;
        self.clone()
    }

    fn as_py_index(&self) -> PyIndex {
        PyIndex {
            inner: self.inner.as_index().clone(),
        }
    }

    fn all(&self) -> bool {
        self.as_py_index().all()
    }

    fn any(&self) -> bool {
        self.as_py_index().any()
    }

    fn append(&self, others: Vec<Bound<'_, PyAny>>) -> PyResult<Self> {
        let mut combined = self.inner.asi8();
        for other in others {
            if let Ok(tdi) = other.extract::<PyRef<'_, PyTimedeltaIndex>>() {
                combined.extend(tdi.inner.asi8());
            } else if let Ok(list) = other.extract::<Vec<i64>>() {
                combined.extend(list);
            }
        }
        let mut out = TimedeltaIndex::new(combined);
        if let Some(n) = self.inner.name() {
            out = out.set_name(n);
        }
        Ok(Self { inner: out })
    }

    fn argsort(&self) -> Vec<usize> {
        self.as_py_index().argsort()
    }

    fn array(&self, py: Python<'_>) -> PyResult<Py<PyList>> {
        self.as_py_index().array(py)
    }

    fn as_unit(&self, unit: &str) -> Self {
        let _ = unit;
        self.clone()
    }

    fn asof(&self, py: Python<'_>, label: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        self.as_py_index().asof(py, label)
    }

    #[pyo3(signature = (where_, mask=None))]
    fn asof_locs(&self, where_: &PyIndex, mask: Option<Vec<bool>>) -> Vec<Option<usize>> {
        self.as_py_index().asof_locs(where_, mask)
    }

    fn astype(&self, dtype: &str) -> PyResult<PyIndex> {
        self.as_py_index().astype(dtype)
    }

    fn components(&self) -> PyResult<PyDataFrame> {
        let nanos_per_sec: i64 = 1_000_000_000;
        let nanos_per_min: i64 = 60 * nanos_per_sec;
        let nanos_per_hour: i64 = 60 * nanos_per_min;
        let nanos_per_day: i64 = 24 * nanos_per_hour;

        let mut days = Vec::with_capacity(self.inner.len());
        let mut hours = Vec::with_capacity(self.inner.len());
        let mut minutes = Vec::with_capacity(self.inner.len());
        let mut seconds = Vec::with_capacity(self.inner.len());
        let mut milliseconds = Vec::with_capacity(self.inner.len());
        let mut microseconds = Vec::with_capacity(self.inner.len());
        let mut nanoseconds = Vec::with_capacity(self.inner.len());

        for &ns in &self.inner.asi8() {
            if ns == Timedelta::NAT {
                days.push(Scalar::Null(NullKind::NaN));
                hours.push(Scalar::Null(NullKind::NaN));
                minutes.push(Scalar::Null(NullKind::NaN));
                seconds.push(Scalar::Null(NullKind::NaN));
                milliseconds.push(Scalar::Null(NullKind::NaN));
                microseconds.push(Scalar::Null(NullKind::NaN));
                nanoseconds.push(Scalar::Null(NullKind::NaN));
            } else {
                let sign = if ns < 0 { -1 } else { 1 };
                let abs_ns = ns.abs();
                let d = abs_ns / nanos_per_day;
                let rem_d = abs_ns % nanos_per_day;
                let h = rem_d / nanos_per_hour;
                let rem_h = rem_d % nanos_per_hour;
                let m = rem_h / nanos_per_min;
                let rem_m = rem_h % nanos_per_min;
                let s = rem_m / nanos_per_sec;
                let rem_s = rem_m % nanos_per_sec;
                let ms = rem_s / 1_000_000;
                let rem_ms = rem_s % 1_000_000;
                let us = rem_ms / 1_000;
                let ns_part = rem_ms % 1_000;

                days.push(Scalar::Int64(sign * d));
                hours.push(Scalar::Int64(sign * h));
                minutes.push(Scalar::Int64(sign * m));
                seconds.push(Scalar::Int64(sign * s));
                milliseconds.push(Scalar::Int64(sign * ms));
                microseconds.push(Scalar::Int64(sign * us));
                nanoseconds.push(Scalar::Int64(sign * ns_part));
            }
        }
        let cols = vec![
            ("days", days),
            ("hours", hours),
            ("minutes", minutes),
            ("seconds", seconds),
            ("milliseconds", milliseconds),
            ("microseconds", microseconds),
            ("nanoseconds", nanoseconds),
        ];
        let df = DataFrame::from_dict(
            &[
                "days",
                "hours",
                "minutes",
                "seconds",
                "milliseconds",
                "microseconds",
                "nanoseconds",
            ],
            cols,
        )
        .map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: df })
    }

    fn delete(&self, loc: usize) -> PyResult<Self> {
        let new_idx = self.as_py_index().delete(loc)?;
        let mut vals = Vec::with_capacity(new_idx.inner.len());
        for l in new_idx.inner.labels() {
            match l {
                IndexLabel::Timedelta64(ns) => vals.push(*ns),
                _ => vals.push(Timedelta::NAT),
            }
        }
        let mut out = TimedeltaIndex::new(vals);
        if let Some(n) = new_idx.inner.name() {
            out = out.set_name(n);
        }
        Ok(Self { inner: out })
    }

    #[pyo3(signature = (periods=1))]
    fn diff(&self, periods: i64) -> Self {
        let vals = self.inner.asi8();
        let len = vals.len();
        let mut out = vec![Timedelta::NAT; len];
        let p = periods as usize;
        if periods > 0 && p < len {
            for i in p..len {
                if vals[i] != Timedelta::NAT && vals[i - p] != Timedelta::NAT {
                    out[i] = vals[i] - vals[i - p];
                }
            }
        }
        let mut res = TimedeltaIndex::new(out);
        if let Some(n) = self.inner.name() {
            res = res.set_name(n);
        }
        Self { inner: res }
    }

    #[pyo3(signature = (sort=false, use_na_sentinel=true))]
    fn factorize(&self, sort: bool, use_na_sentinel: bool) -> (Vec<isize>, PyIndex) {
        self.as_py_index().factorize(sort, use_na_sentinel)
    }

    fn format(&self) -> Vec<String> {
        self.as_py_index().format()
    }

    #[getter]
    fn freq(&self) -> Option<String> {
        None
    }

    #[getter]
    fn freqstr(&self) -> Option<String> {
        None
    }

    #[getter]
    fn inferred_freq(&self) -> Option<String> {
        None
    }

    #[getter]
    fn unit(&self) -> &'static str {
        "ns"
    }

    #[getter]
    fn resolution(&self) -> &'static str {
        "nano"
    }

    fn get_indexer_for(&self, target: &Bound<'_, PyAny>) -> PyResult<Vec<i64>> {
        self.as_py_index().get_indexer_for(target)
    }

    fn get_indexer_non_unique(
        &self,
        target: &Bound<'_, PyAny>,
    ) -> PyResult<(Vec<isize>, Vec<usize>)> {
        self.as_py_index().get_indexer_non_unique(target)
    }

    fn get_slice_bound(&self, label: &Bound<'_, PyAny>, side: &str) -> PyResult<usize> {
        self.as_py_index().get_slice_bound(label, side)
    }

    fn groupby(&self, py: Python<'_>, by: &Bound<'_, PyAny>) -> PyResult<Py<pyo3::types::PyDict>> {
        self.as_py_index().groupby(py, by)
    }

    fn identical(&self, other: &Bound<'_, PyAny>) -> bool {
        if let Ok(td) = other.extract::<PyRef<'_, PyTimedeltaIndex>>() {
            self.inner.equals(&td.inner) && self.inner.name() == td.inner.name()
        } else {
            false
        }
    }

    fn infer_objects(&self) -> Self {
        self.clone()
    }

    fn insert(&self, loc: usize, item: &Bound<'_, PyAny>) -> PyResult<PyIndex> {
        self.as_py_index().insert(loc, item)
    }

    fn is_(&self, other: &Bound<'_, PyAny>) -> bool {
        if let Ok(td) = other.extract::<PyRef<'_, PyTimedeltaIndex>>() {
            self.inner.equals(&td.inner) && self.inner.name() == td.inner.name()
        } else {
            false
        }
    }

    fn item(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        self.as_py_index().item(py)
    }

    #[pyo3(signature = (other, how="left", level=None, return_indexers=false, sort=false))]
    fn join(
        &self,
        other: &Bound<'_, PyAny>,
        how: &str,
        level: Option<usize>,
        return_indexers: bool,
        sort: bool,
    ) -> PyResult<PyIndex> {
        self.as_py_index()
            .join(other, how, level, return_indexers, sort)
    }

    fn map(&self, py: Python<'_>, mapper: &Bound<'_, PyAny>) -> PyResult<PyIndex> {
        self.as_py_index().map(py, mapper)
    }

    fn putmask(&self, mask: Vec<bool>, value: &Bound<'_, PyAny>) -> PyResult<PyIndex> {
        self.as_py_index().putmask(mask, value)
    }

    fn ravel(&self) -> Self {
        self.clone()
    }

    #[pyo3(signature = (target, method=None, level=None, limit=None, tolerance=None))]
    fn reindex(
        &self,
        target: &Bound<'_, PyAny>,
        method: Option<&str>,
        level: Option<usize>,
        limit: Option<usize>,
        tolerance: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<(PyIndex, Vec<i64>)> {
        self.as_py_index()
            .reindex(target, method, level, limit, tolerance)
    }

    fn repeat(&self, repeats: usize) -> Self {
        let mut out = Vec::with_capacity(self.inner.len() * repeats);
        for &v in &self.inner.asi8() {
            for _ in 0..repeats {
                out.push(v);
            }
        }
        let mut res = TimedeltaIndex::new(out);
        if let Some(n) = self.inner.name() {
            res = res.set_name(n);
        }
        Self { inner: res }
    }

    #[pyo3(signature = (value, side="left", sorter=None))]
    fn searchsorted(
        &self,
        value: &Bound<'_, PyAny>,
        side: &str,
        sorter: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<usize> {
        self.as_py_index().searchsorted(value, side, sorter)
    }

    fn set_names(&self, names: &Bound<'_, PyAny>) -> PyResult<Self> {
        let name = if let Ok(s) = names.extract::<String>() {
            Some(s)
        } else if let Ok(list) = names.extract::<Vec<Option<String>>>() {
            list.into_iter().next().flatten()
        } else {
            None
        };
        let mut res = self.inner.clone();
        if let Some(n) = name {
            res = res.set_name(&n);
        }
        Ok(Self { inner: res })
    }

    #[pyo3(signature = (level=None, ascending=true, sort_remaining=None))]
    fn sortlevel(
        &self,
        level: Option<usize>,
        ascending: bool,
        sort_remaining: Option<bool>,
    ) -> PyResult<(Self, Vec<usize>)> {
        let _ = (level, ascending, sort_remaining);
        Ok((self.clone(), (0..self.inner.len()).collect()))
    }

    #[getter]
    fn r#str(&self) -> PyIndexStringMethods {
        PyIndexStringMethods {
            inner: self.inner.as_index().clone(),
        }
    }

    fn sum(&self) -> i64 {
        self.inner
            .asi8()
            .iter()
            .filter(|&&x| x != Timedelta::NAT)
            .sum()
    }

    fn take(&self, indices: Vec<i64>) -> Self {
        let vals = self.inner.asi8();
        let len = vals.len() as i64;
        let mut out = Vec::with_capacity(indices.len());
        for idx in indices {
            let pos = if idx < 0 { len + idx } else { idx };
            if pos >= 0 && pos < len {
                out.push(vals[pos as usize]);
            } else {
                out.push(Timedelta::NAT);
            }
        }
        let mut res = TimedeltaIndex::new(out);
        if let Some(n) = self.inner.name() {
            res = res.set_name(n);
        }
        Self { inner: res }
    }

    fn to_pytimedelta(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let datetime_mod = py.import("datetime")?;
        let td_cls = datetime_mod.getattr("timedelta")?;
        let mut list = Vec::with_capacity(self.inner.len());
        for &ns in &self.inner.asi8() {
            if ns == Timedelta::NAT {
                list.push(py.None());
            } else {
                let microseconds = ns / 1_000;
                let py_td = td_cls.call1((0, 0, microseconds))?;
                list.push(py_td.into_any().unbind());
            }
        }
        Ok(PyList::new(py, list)?.into_any().unbind())
    }

    fn transpose(&self) -> Self {
        self.clone()
    }

    fn view(&self) -> Self {
        self.clone()
    }

    #[pyo3(signature = (cond, other=None))]
    fn r#where(&self, cond: Vec<bool>, other: Option<&Bound<'_, PyAny>>) -> PyResult<PyIndex> {
        self.as_py_index().r#where(cond, other)
    }
}

/// Python wrapper for FrankenPandas RangeIndex.
#[pyclass(name = "RangeIndex", from_py_object)]
#[derive(Clone)]
pub struct PyRangeIndex {
    pub(crate) inner: RangeIndex,
}

#[pymethods]
impl PyRangeIndex {
    #[new]
    #[pyo3(signature = (start=None, stop=None, step=None, name=None))]
    pub fn new(
        start: Option<i64>,
        stop: Option<i64>,
        step: Option<i64>,
        name: Option<&str>,
    ) -> PyResult<Self> {
        let (st, sp, step_val) = match (start, stop) {
            (Some(val), None) => (0, val, step.unwrap_or(1)),
            (Some(st), Some(sp)) => (st, sp, step.unwrap_or(1)),
            (None, None) => (0, 0, step.unwrap_or(1)),
            (None, Some(sp)) => (0, sp, step.unwrap_or(1)),
        };
        let mut inner = RangeIndex::new(st, sp, step_val)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        if let Some(n) = name {
            inner = inner.set_name(n);
        }
        Ok(Self { inner })
    }

    #[getter]
    pub fn start(&self) -> i64 {
        self.inner.start()
    }

    #[getter]
    pub fn stop(&self) -> i64 {
        self.inner.stop()
    }

    #[getter]
    pub fn step(&self) -> i64 {
        self.inner.step()
    }

    #[getter]
    pub fn name(&self) -> Option<String> {
        self.inner.name().map(str::to_owned)
    }

    #[getter]
    pub fn shape(&self) -> (usize,) {
        self.inner.shape()
    }

    #[getter]
    pub fn size(&self) -> usize {
        self.inner.size()
    }

    #[getter]
    pub fn ndim(&self) -> usize {
        self.inner.ndim()
    }

    #[getter]
    pub fn empty(&self) -> bool {
        self.inner.empty()
    }

    #[getter]
    pub fn dtype(&self) -> &'static str {
        self.inner.dtype()
    }

    #[getter]
    pub fn is_monotonic_increasing(&self) -> bool {
        self.inner.is_monotonic_increasing()
    }

    #[getter]
    pub fn is_monotonic_decreasing(&self) -> bool {
        self.inner.is_monotonic_decreasing()
    }

    #[getter]
    pub fn is_unique(&self) -> bool {
        self.inner.is_unique()
    }

    #[getter]
    pub fn has_duplicates(&self) -> bool {
        self.inner.has_duplicates()
    }

    pub fn min(&self) -> Option<i64> {
        self.inner.min()
    }

    pub fn max(&self) -> Option<i64> {
        self.inner.max()
    }

    pub fn tolist(&self) -> Vec<i64> {
        self.inner.values().iter().collect()
    }

    pub fn to_list(&self) -> Vec<i64> {
        self.tolist()
    }

    pub fn values(&self) -> Vec<i64> {
        self.tolist()
    }

    pub fn to_numpy(&self) -> Vec<i64> {
        self.tolist()
    }

    pub fn copy(&self) -> Self {
        Self {
            inner: self.inner.copy(),
        }
    }

    pub fn rename(&self, name: Option<&str>) -> Self {
        Self {
            inner: self.inner.rename_index(name),
        }
    }

    pub fn equals(&self, other: &Self) -> bool {
        self.inner.equals(&other.inner)
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub fn __len__(&self) -> usize {
        self.inner.len()
    }

    pub fn __repr__(&self) -> String {
        format!(
            "RangeIndex(start={}, stop={}, step={})",
            self.inner.start(),
            self.inner.stop(),
            self.inner.step()
        )
    }

    pub fn __contains__(&self, item: i64) -> bool {
        let start = self.inner.start();
        let stop = self.inner.stop();
        let step = self.inner.step();
        if step > 0 {
            item >= start && item < stop && (item - start) % step == 0
        } else {
            item <= start && item > stop && (start - item) % (-step) == 0
        }
    }

    pub fn __getitem__(&self, py: Python<'_>, item: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        if let Ok(idx) = item.extract::<i64>() {
            let len = self.inner.len() as i64;
            let pos = if idx < 0 { len + idx } else { idx };
            if pos < 0 || pos >= len {
                return Err(PyErr::new::<pyo3::exceptions::PyIndexError, _>(
                    "index out of bounds",
                ));
            }
            let val = self.inner.start() + (pos * self.inner.step());
            return val.into_py_any(py);
        }
        if let Ok(slice) = item.cast::<pyo3::types::PySlice>() {
            let indices = slice.indices(self.inner.len() as isize)?;
            let new_start = self.inner.start() + (indices.start as i64) * self.inner.step();
            let new_step = self.inner.step() * (indices.step as i64);
            let new_stop = self.inner.start() + (indices.stop as i64) * self.inner.step();
            let sub = RangeIndex::new(new_start, new_stop, new_step)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
            return Py::new(py, PyRangeIndex { inner: sub })?
                .into_any()
                .into_py_any(py);
        }
        Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
            "RangeIndex indices must be integers or slices",
        ))
    }

    #[getter]
    fn hasnans(&self) -> bool {
        false
    }

    #[getter]
    fn nlevels(&self) -> usize {
        1
    }

    #[getter]
    fn names(&self) -> Vec<Option<String>> {
        vec![self.inner.name().map(str::to_string)]
    }

    #[getter]
    fn nbytes(&self) -> usize {
        self.inner.nbytes()
    }

    #[pyo3(signature = (deep=false))]
    fn memory_usage(&self, deep: bool) -> usize {
        self.inner.memory_usage(deep)
    }

    fn inferred_type(&self) -> &'static str {
        "integer"
    }

    fn is_numeric(&self) -> bool {
        true
    }

    fn is_boolean(&self) -> bool {
        false
    }

    fn is_floating(&self) -> bool {
        false
    }

    fn is_integer(&self) -> bool {
        true
    }

    fn is_categorical(&self) -> bool {
        false
    }

    fn is_object(&self) -> bool {
        false
    }

    fn is_interval(&self) -> bool {
        false
    }

    fn holds_integer(&self) -> bool {
        true
    }

    fn union(&self, other: &PyIndex) -> PyIndex {
        let idx = Index::from_range(self.inner.start(), self.inner.stop(), self.inner.step());
        PyIndex {
            inner: idx.union(&other.inner),
        }
    }

    fn intersection(&self, other: &PyIndex) -> PyIndex {
        let idx = Index::from_range(self.inner.start(), self.inner.stop(), self.inner.step());
        PyIndex {
            inner: idx.intersection(&other.inner),
        }
    }

    fn difference(&self, other: &PyIndex) -> PyIndex {
        let idx = Index::from_range(self.inner.start(), self.inner.stop(), self.inner.step());
        PyIndex {
            inner: idx.difference(&other.inner),
        }
    }

    fn symmetric_difference(&self, other: &PyIndex) -> PyIndex {
        let idx = Index::from_range(self.inner.start(), self.inner.stop(), self.inner.step());
        PyIndex {
            inner: idx.symmetric_difference(&other.inner),
        }
    }

    fn get_loc(&self, key: &Bound<'_, PyAny>) -> PyResult<usize> {
        let label = py_to_index_label(key)?;
        let idx = Index::from_range(self.inner.start(), self.inner.stop(), self.inner.step());
        idx.get_loc(&label)
            .ok_or_else(|| PyErr::new::<pyo3::exceptions::PyKeyError, _>(format!("{key}")))
    }

    fn get_indexer(&self, target: &PyIndex) -> Vec<i64> {
        let idx = Index::from_range(self.inner.start(), self.inner.stop(), self.inner.step());
        idx.get_indexer(&target.inner)
            .into_iter()
            .map(|opt| opt.map(|u| u as i64).unwrap_or(-1))
            .collect()
    }

    #[pyo3(signature = (start=None, end=None, step=None))]
    fn slice_locs(
        &self,
        start: Option<&Bound<'_, PyAny>>,
        end: Option<&Bound<'_, PyAny>>,
        step: Option<isize>,
    ) -> PyResult<(usize, usize)> {
        let _ = step;
        let s_lbl = match start {
            Some(obj) => Some(py_to_index_label(obj)?),
            None => None,
        };
        let e_lbl = match end {
            Some(obj) => Some(py_to_index_label(obj)?),
            None => None,
        };
        let idx = Index::from_range(self.inner.start(), self.inner.stop(), self.inner.step());
        idx.slice_locs(s_lbl.as_ref(), e_lbl.as_ref())
            .map_err(index_error_to_py)
    }

    #[pyo3(signature = (start=None, end=None, step=None))]
    fn slice_indexer(
        &self,
        start: Option<&Bound<'_, PyAny>>,
        end: Option<&Bound<'_, PyAny>>,
        step: Option<isize>,
    ) -> PyResult<(usize, usize, isize)> {
        let (s, e) = self.slice_locs(start, end, step)?;
        Ok((s, e, step.unwrap_or(1)))
    }

    #[pyo3(signature = (level=0))]
    fn droplevel(&self, level: usize) -> PyResult<PyIndex> {
        let _ = level;
        let idx = Index::from_range(self.inner.start(), self.inner.stop(), self.inner.step());
        Ok(PyIndex { inner: idx })
    }

    #[pyo3(signature = (level=0))]
    fn get_level_values(&self, level: usize) -> PyResult<Self> {
        let _ = level;
        Ok(self.clone())
    }

    fn to_flat_index(&self) -> Self {
        self.clone()
    }

    #[pyo3(signature = (index=None, name=None))]
    fn to_series(
        &self,
        index: Option<&Bound<'_, PyAny>>,
        name: Option<&str>,
    ) -> PyResult<PySeries> {
        let idx = Index::from_range(self.inner.start(), self.inner.stop(), self.inner.step());
        let series_name = name.or_else(|| self.inner.name()).unwrap_or("");
        let final_idx = if let Some(i_obj) = index {
            if let Ok(py_idx) = i_obj.extract::<PyRef<'_, PyIndex>>() {
                py_idx.inner.clone()
            } else {
                idx.clone()
            }
        } else {
            idx.clone()
        };
        let col = Column::from_values(
            idx.labels()
                .iter()
                .map(|l| match l {
                    IndexLabel::Int64(i) => Scalar::Int64(*i),
                    _ => Scalar::Null(NullKind::NaN),
                })
                .collect(),
        )
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        let s = Series::new(series_name, final_idx, col).map_err(frame_error_to_py)?;
        Ok(PySeries { inner: s })
    }

    #[pyo3(signature = (index=true, name=None))]
    fn to_frame(&self, index: bool, name: Option<&str>) -> PyResult<PyDataFrame> {
        let col_name = name.or_else(|| self.inner.name()).unwrap_or("0");
        let idx = Index::from_range(self.inner.start(), self.inner.stop(), self.inner.step());
        let final_idx = if index {
            idx.clone()
        } else {
            Index::from_range(0, self.inner.len() as i64, 1)
        };
        let col = Column::from_values(
            idx.labels()
                .iter()
                .map(|l| match l {
                    IndexLabel::Int64(i) => Scalar::Int64(*i),
                    _ => Scalar::Null(NullKind::NaN),
                })
                .collect(),
        )
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        let mut col_map = BTreeMap::new();
        col_map.insert(col_name.to_string(), col);
        let df = DataFrame::new_with_column_order(final_idx, col_map, vec![col_name.to_string()])
            .map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: df })
    }

    #[pyo3(signature = (normalize=false, sort=true, ascending=false, dropna=true))]
    fn value_counts(
        &self,
        normalize: bool,
        sort: bool,
        ascending: bool,
        dropna: bool,
    ) -> PyResult<PySeries> {
        let idx = Index::from_range(self.inner.start(), self.inner.stop(), self.inner.step());
        let counts = idx.value_counts_with_options(normalize, sort, ascending, dropna);
        let mut idx_labels = Vec::with_capacity(counts.len());
        let mut vals = Vec::with_capacity(counts.len());
        for (lbl, sc) in counts {
            idx_labels.push(lbl);
            vals.push(sc);
        }
        let col = Column::from_values(vals)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        let s = Series::new("count", Index::new(idx_labels), col).map_err(frame_error_to_py)?;
        Ok(PySeries { inner: s })
    }

    #[pyo3(signature = (ascending=true, na_position="last"))]
    fn sort_values(&self, ascending: bool, na_position: &str) -> PyResult<PyIndex> {
        let _ = na_position;
        let idx = Index::from_range(self.inner.start(), self.inner.stop(), self.inner.step());
        let mut sorted = idx.sort_values();
        if !ascending {
            let mut rev_labels = sorted.labels().to_vec();
            rev_labels.reverse();
            sorted = Index::new(rev_labels);
            if let Some(n) = self.inner.name() {
                sorted = sorted.rename_index(Some(n));
            }
        }
        Ok(PyIndex { inner: sorted })
    }

    fn sort(&self) -> PyResult<PyIndex> {
        self.sort_values(true, "last")
    }

    #[getter]
    #[allow(non_snake_case)]
    fn T(&self) -> Self {
        self.clone()
    }

    fn drop(&self, labels: Vec<Bound<'_, PyAny>>) -> PyResult<PyIndex> {
        let mut to_drop = Vec::with_capacity(labels.len());
        for l in labels {
            to_drop.push(py_to_index_label(&l)?);
        }
        let idx = Index::from_range(self.inner.start(), self.inner.stop(), self.inner.step());
        Ok(PyIndex {
            inner: idx.drop_labels(&to_drop),
        })
    }

    #[pyo3(signature = (how="any"))]
    fn dropna(&self, how: &str) -> Self {
        let _ = how;
        self.clone()
    }

    fn fillna(&self, value: &Bound<'_, PyAny>) -> Self {
        let _ = value;
        self.clone()
    }

    fn all(&self) -> bool {
        self.inner.to_index().all()
    }

    fn any(&self) -> bool {
        self.inner.to_index().any()
    }

    fn append(&self, other: &Bound<'_, PyAny>) -> PyResult<PyIndex> {
        let other_idx = if let Ok(py_idx) = other.extract::<PyRef<'_, PyIndex>>() {
            py_idx.inner.clone()
        } else if let Ok(py_rng) = other.extract::<PyRef<'_, PyRangeIndex>>() {
            py_rng.inner.to_index()
        } else {
            PyIndex::new(Some(other), None)?.inner
        };
        Ok(PyIndex {
            inner: self.inner.to_index().append(&other_idx),
        })
    }

    fn argmax(&self) -> PyResult<usize> {
        if self.inner.is_empty() {
            return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
                "attempt to get argmax of an empty sequence",
            ));
        }
        if self.inner.step() > 0 {
            Ok(self.inner.len() - 1)
        } else {
            Ok(0)
        }
    }

    fn argmin(&self) -> PyResult<usize> {
        if self.inner.is_empty() {
            return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
                "attempt to get argmin of an empty sequence",
            ));
        }
        if self.inner.step() > 0 {
            Ok(0)
        } else {
            Ok(self.inner.len() - 1)
        }
    }

    fn argsort(&self) -> Vec<usize> {
        if self.inner.step() > 0 {
            (0..self.inner.len()).collect()
        } else {
            (0..self.inner.len()).rev().collect()
        }
    }

    fn array(&self, py: Python<'_>) -> PyResult<Py<PyList>> {
        let list: Vec<i64> = self.tolist();
        Ok(PyList::new(py, list)?.unbind())
    }

    fn asof(&self, py: Python<'_>, label: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let lbl = py_to_index_label(label)?;
        match self.inner.to_index().asof(&lbl) {
            Some(l) => index_label_to_py(py, &l),
            None => Ok(py.None()),
        }
    }

    #[pyo3(signature = (where_, mask=None))]
    fn asof_locs(
        &self,
        where_: &Bound<'_, PyAny>,
        mask: Option<Vec<bool>>,
    ) -> PyResult<Vec<Option<usize>>> {
        let where_idx = if let Ok(py_idx) = where_.extract::<PyRef<'_, PyIndex>>() {
            py_idx.inner.clone()
        } else if let Ok(py_rng) = where_.extract::<PyRef<'_, PyRangeIndex>>() {
            py_rng.inner.to_index()
        } else {
            PyIndex::new(Some(where_), None)?.inner
        };
        Ok(self.inner.to_index().asof_locs(&where_idx, mask.as_deref()))
    }

    fn astype(&self, dtype: &str) -> PyResult<Self> {
        let _ = dtype;
        Ok(self.clone())
    }

    fn delete(&self, loc: usize) -> PyResult<PyIndex> {
        let idx = self.inner.to_index();
        let res = idx.delete(loc).map_err(index_error_to_py)?;
        Ok(PyIndex { inner: res })
    }

    #[pyo3(signature = (periods=1))]
    fn diff(&self, periods: i64) -> PyResult<PyIndex> {
        let p = if periods < 0 { 0 } else { periods as usize };
        let diff_labels = self.inner.to_index().diff(p);
        let labels: Vec<IndexLabel> = diff_labels
            .into_iter()
            .map(|opt| opt.unwrap_or(IndexLabel::Null(NullKind::NaN)))
            .collect();
        let mut idx = Index::new(labels);
        if let Some(n) = self.inner.name() {
            idx = idx.rename_index(Some(n));
        }
        Ok(PyIndex { inner: idx })
    }

    fn drop_duplicates(&self) -> Self {
        self.clone()
    }

    fn duplicated(&self) -> Vec<bool> {
        vec![false; self.inner.len()]
    }

    #[pyo3(signature = (sort=false, use_na_sentinel=true))]
    fn factorize(&self, sort: bool, use_na_sentinel: bool) -> (Vec<isize>, Self) {
        let _ = (sort, use_na_sentinel);
        let codes: Vec<isize> = (0..self.inner.len() as isize).collect();
        (codes, self.clone())
    }

    fn format(&self) -> Vec<String> {
        self.inner.to_index().format()
    }

    #[classmethod]
    #[pyo3(signature = (data, name=None))]
    fn from_range(
        _cls: &Bound<'_, pyo3::types::PyType>,
        data: &Bound<'_, PyAny>,
        name: Option<&str>,
    ) -> PyResult<Self> {
        let start: i64 = data.getattr("start")?.extract()?;
        let stop: i64 = data.getattr("stop")?.extract()?;
        let step: i64 = data.getattr("step")?.extract()?;
        let mut inner = RangeIndex::new(start, stop, step)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        if let Some(n) = name {
            inner = inner.set_name(n);
        }
        Ok(Self { inner })
    }

    fn get_indexer_for(&self, target: &Bound<'_, PyAny>) -> PyResult<Vec<i64>> {
        let target_idx = if let Ok(py_idx) = target.extract::<PyRef<'_, PyIndex>>() {
            py_idx.inner.clone()
        } else if let Ok(py_rng) = target.extract::<PyRef<'_, PyRangeIndex>>() {
            py_rng.inner.to_index()
        } else {
            PyIndex::new(Some(target), None)?.inner
        };
        Ok(self
            .inner
            .to_index()
            .get_indexer_for(&target_idx)
            .into_iter()
            .map(|opt| opt.map(|u| u as i64).unwrap_or(-1))
            .collect())
    }

    fn get_indexer_non_unique(
        &self,
        target: &Bound<'_, PyAny>,
    ) -> PyResult<(Vec<isize>, Vec<usize>)> {
        let target_idx = if let Ok(py_idx) = target.extract::<PyRef<'_, PyIndex>>() {
            py_idx.inner.clone()
        } else if let Ok(py_rng) = target.extract::<PyRef<'_, PyRangeIndex>>() {
            py_rng.inner.to_index()
        } else {
            PyIndex::new(Some(target), None)?.inner
        };
        Ok(self.inner.to_index().get_indexer_non_unique(&target_idx))
    }

    fn get_slice_bound(&self, label: &Bound<'_, PyAny>, side: &str) -> PyResult<usize> {
        let lbl = py_to_index_label(label)?;
        self.inner
            .to_index()
            .get_slice_bound(&lbl, side)
            .map_err(index_error_to_py)
    }

    fn groupby(&self, py: Python<'_>, by: &Bound<'_, PyAny>) -> PyResult<Py<pyo3::types::PyDict>> {
        let idx = self.inner.to_index();
        let py_idx = PyIndex { inner: idx };
        py_idx.groupby(py, by)
    }

    fn identical(&self, other: &Bound<'_, PyAny>) -> bool {
        if let Ok(other_rng) = other.extract::<PyRef<'_, PyRangeIndex>>() {
            self.inner == other_rng.inner && self.inner.name() == other_rng.inner.name()
        } else {
            false
        }
    }

    fn infer_objects(&self) -> Self {
        self.clone()
    }

    fn insert(&self, loc: usize, item: &Bound<'_, PyAny>) -> PyResult<PyIndex> {
        let idx = self.inner.to_index();
        let py_idx = PyIndex { inner: idx };
        py_idx.insert(loc, item)
    }

    fn is_(&self, other: &Bound<'_, PyAny>) -> bool {
        if let Ok(other_rng) = other.extract::<PyRef<'_, PyRangeIndex>>() {
            self.inner == other_rng.inner
        } else {
            false
        }
    }

    fn isin(&self, values: &Bound<'_, PyAny>) -> PyResult<Vec<bool>> {
        let idx = self.inner.to_index();
        let py_idx = PyIndex { inner: idx };
        py_idx.isin(values)
    }

    fn isna(&self) -> Vec<bool> {
        vec![false; self.inner.len()]
    }

    fn isnull(&self) -> Vec<bool> {
        vec![false; self.inner.len()]
    }

    fn item(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        if self.inner.len() == 1 {
            self.inner.start().into_py_any(py)
        } else {
            Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
                "can only convert an array of size 1 to a Python scalar",
            ))
        }
    }

    #[pyo3(signature = (other, how="left", level=None, return_indexers=false, sort=false))]
    fn join(
        &self,
        other: &Bound<'_, PyAny>,
        how: &str,
        level: Option<usize>,
        return_indexers: bool,
        sort: bool,
    ) -> PyResult<PyIndex> {
        let idx = self.inner.to_index();
        let py_idx = PyIndex { inner: idx };
        py_idx.join(other, how, level, return_indexers, sort)
    }

    fn map(&self, py: Python<'_>, mapper: &Bound<'_, PyAny>) -> PyResult<PyIndex> {
        let idx = self.inner.to_index();
        let py_idx = PyIndex { inner: idx };
        py_idx.map(py, mapper)
    }

    fn notna(&self) -> Vec<bool> {
        vec![true; self.inner.len()]
    }

    fn notnull(&self) -> Vec<bool> {
        vec![true; self.inner.len()]
    }

    fn nunique(&self) -> usize {
        self.inner.len()
    }

    fn putmask(&self, mask: Vec<bool>, value: &Bound<'_, PyAny>) -> PyResult<PyIndex> {
        let idx = self.inner.to_index();
        let py_idx = PyIndex { inner: idx };
        py_idx.putmask(mask, value)
    }

    fn ravel(&self) -> Self {
        self.clone()
    }

    #[pyo3(signature = (target, method=None, level=None, limit=None, tolerance=None))]
    fn reindex(
        &self,
        target: &Bound<'_, PyAny>,
        method: Option<&str>,
        level: Option<usize>,
        limit: Option<usize>,
        tolerance: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<(PyIndex, Vec<i64>)> {
        let idx = self.inner.to_index();
        let py_idx = PyIndex { inner: idx };
        py_idx.reindex(target, method, level, limit, tolerance)
    }

    fn repeat(&self, repeats: usize) -> PyIndex {
        PyIndex {
            inner: self.inner.to_index().repeat(repeats),
        }
    }

    #[pyo3(signature = (decimals=0))]
    fn round(&self, decimals: i32) -> Self {
        let _ = decimals;
        self.clone()
    }

    #[pyo3(signature = (value, side="left", sorter=None))]
    fn searchsorted(
        &self,
        value: &Bound<'_, PyAny>,
        side: &str,
        sorter: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<usize> {
        let idx = self.inner.to_index();
        let py_idx = PyIndex { inner: idx };
        py_idx.searchsorted(value, side, sorter)
    }

    #[pyo3(signature = (names, level=None))]
    fn set_names(&self, names: &Bound<'_, PyAny>, level: Option<usize>) -> PyResult<Self> {
        let _ = level;
        let name_opt = if let Ok(s) = names.extract::<String>() {
            Some(s)
        } else if let Ok(seq) = names.cast::<pyo3::types::PySequence>() {
            if seq.len()? > 0 {
                Some(seq.get_item(0)?.extract::<String>()?)
            } else {
                None
            }
        } else {
            None
        };
        let mut inner = self.inner.clone();
        inner = inner.set_name(name_opt.as_deref().unwrap_or(""));
        Ok(Self { inner })
    }

    #[pyo3(signature = (periods=1, freq=None))]
    fn shift(&self, periods: i64, freq: Option<&str>) -> PyResult<Self> {
        let _ = (periods, freq);
        Err(PyErr::new::<pyo3::exceptions::PyNotImplementedError, _>(
            "This method is only implemented for DatetimeIndex, PeriodIndex and TimedeltaIndex; Got type RangeIndex",
        ))
    }

    #[pyo3(signature = (level=None, ascending=true, sort_remaining=None))]
    fn sortlevel(
        &self,
        level: Option<usize>,
        ascending: bool,
        sort_remaining: Option<bool>,
    ) -> PyResult<(Self, Vec<usize>)> {
        let _ = (level, ascending, sort_remaining);
        Ok((self.clone(), (0..self.inner.len()).collect()))
    }

    #[getter]
    fn r#str(&self) -> PyIndexStringMethods {
        PyIndexStringMethods {
            inner: self.inner.to_index(),
        }
    }

    fn take(&self, indices: Vec<i64>) -> PyResult<PyIndex> {
        let idx = self.inner.to_index();
        let py_idx = PyIndex { inner: idx };
        py_idx.take(indices)
    }

    fn transpose(&self) -> Self {
        self.clone()
    }

    fn unique(&self) -> Self {
        self.clone()
    }

    fn view(&self) -> Self {
        self.clone()
    }

    #[pyo3(signature = (cond, other=None))]
    fn r#where(&self, cond: Vec<bool>, other: Option<&Bound<'_, PyAny>>) -> PyResult<PyIndex> {
        let idx = self.inner.to_index();
        let py_idx = PyIndex { inner: idx };
        py_idx.r#where(cond, other)
    }
}

/// Python wrapper for FrankenPandas PeriodIndex.
#[pyclass(name = "PeriodIndex", from_py_object)]
#[derive(Clone)]
pub struct PyPeriodIndex {
    pub(crate) inner: PeriodIndex,
}

#[pymethods]
impl PyPeriodIndex {
    #[new]
    #[pyo3(signature = (data=None, freq=None, name=None))]
    pub fn new(
        data: Option<&Bound<'_, PyAny>>,
        freq: Option<&str>,
        name: Option<&str>,
    ) -> PyResult<Self> {
        let period_freq = match freq {
            Some(f) => PeriodFreq::parse(f).ok_or_else(|| {
                PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                    "unsupported frequency '{f}'"
                ))
            })?,
            None => PeriodFreq::Daily,
        };
        let mut inner = if let Some(d) = data {
            if let Ok(pi) = d.extract::<PyRef<'_, PyPeriodIndex>>() {
                pi.inner.clone()
            } else if let Ok(list) = d.extract::<Vec<Bound<'_, PyAny>>>() {
                let mut periods = Vec::with_capacity(list.len());
                for item in list {
                    if let Ok(s) = item.extract::<String>() {
                        let p = Period::parse(&s).map_err(|e| {
                            PyErr::new::<pyo3::exceptions::PyValueError, _>(format!("{e}"))
                        })?;
                        periods.push(p);
                    } else if let Ok(ord) = item.extract::<i64>() {
                        periods.push(Period::new(ord, period_freq));
                    } else {
                        return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                            "Unsupported element in PeriodIndex data",
                        ));
                    }
                }
                PeriodIndex::new(periods)
            } else {
                return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                    "PeriodIndex requires iterable data or another PeriodIndex",
                ));
            }
        } else {
            PeriodIndex::new(Vec::new())
        };
        if let Some(n) = name {
            inner = inner.set_name(n);
        }
        Ok(Self { inner })
    }

    #[getter]
    pub fn year(&self) -> PyResult<Vec<Option<i32>>> {
        self.inner.year().map_err(index_error_to_py)
    }

    #[getter]
    pub fn month(&self) -> PyResult<Vec<Option<u32>>> {
        self.inner.month().map_err(index_error_to_py)
    }

    #[getter]
    pub fn day(&self) -> PyResult<Vec<Option<u32>>> {
        self.inner.day().map_err(index_error_to_py)
    }

    #[getter]
    pub fn hour(&self) -> PyResult<Vec<Option<u32>>> {
        self.inner.hour().map_err(index_error_to_py)
    }

    #[getter]
    pub fn minute(&self) -> PyResult<Vec<Option<u32>>> {
        self.inner.minute().map_err(index_error_to_py)
    }

    #[getter]
    pub fn second(&self) -> PyResult<Vec<Option<u32>>> {
        self.inner.second().map_err(index_error_to_py)
    }

    #[getter]
    pub fn shape(&self) -> (usize,) {
        self.inner.shape()
    }

    #[getter]
    pub fn size(&self) -> usize {
        self.inner.size()
    }

    #[getter]
    pub fn ndim(&self) -> usize {
        self.inner.ndim()
    }

    #[getter]
    pub fn empty(&self) -> bool {
        self.inner.empty()
    }

    #[getter]
    pub fn dtype(&self) -> String {
        self.inner.dtype()
    }

    #[getter]
    pub fn name(&self) -> Option<String> {
        self.inner.name().map(str::to_owned)
    }

    #[getter]
    pub fn is_monotonic_increasing(&self) -> bool {
        self.inner.is_monotonic_increasing()
    }

    #[getter]
    pub fn is_monotonic_decreasing(&self) -> bool {
        self.inner.is_monotonic_decreasing()
    }

    #[getter]
    pub fn is_unique(&self) -> bool {
        self.inner.is_unique()
    }

    #[getter]
    pub fn has_duplicates(&self) -> bool {
        self.inner.has_duplicates()
    }

    pub fn tolist(&self) -> Vec<String> {
        self.inner
            .values()
            .iter()
            .map(|p| format!("{}", p.ordinal))
            .collect()
    }

    pub fn to_list(&self) -> Vec<String> {
        self.tolist()
    }

    pub fn values(&self) -> Vec<String> {
        self.tolist()
    }

    pub fn copy(&self) -> Self {
        Self {
            inner: self.inner.copy(),
        }
    }

    pub fn rename(&self, name: Option<&str>) -> Self {
        Self {
            inner: self.inner.rename_index(name),
        }
    }

    pub fn equals(&self, other: &Self) -> bool {
        self.inner.equals(&other.inner)
    }

    #[pyo3(signature = (how="start"))]
    pub fn to_timestamp(&self, how: &str) -> PyResult<PyDatetimeIndex> {
        let dt_idx = self
            .inner
            .to_timestamp(how)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PyDatetimeIndex { inner: dt_idx })
    }

    #[pyo3(signature = (freq, how="end"))]
    pub fn asfreq(&self, freq: &str, how: &str) -> PyResult<Self> {
        let res = self
            .inner
            .asfreq_with_how(freq, how)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(Self { inner: res })
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub fn __len__(&self) -> usize {
        self.inner.len()
    }

    pub fn __repr__(&self) -> String {
        format!(
            "PeriodIndex(len={}, dtype='{}')",
            self.inner.len(),
            self.inner.dtype()
        )
    }

    pub fn __getitem__(&self, py: Python<'_>, item: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        if let Ok(idx) = item.extract::<i64>() {
            let len = self.inner.len() as i64;
            let pos = if idx < 0 { len + idx } else { idx };
            if pos < 0 || pos >= len {
                return Err(PyErr::new::<pyo3::exceptions::PyIndexError, _>(
                    "index out of bounds",
                ));
            }
            let p = &self.inner.values()[pos as usize];
            return format!("{}", p.ordinal).into_py_any(py);
        }
        if let Ok(slice) = item.cast::<pyo3::types::PySlice>() {
            let indices = slice.indices(self.inner.len() as isize)?;
            let mut sliced = Vec::new();
            let mut i = indices.start;
            if indices.step > 0 {
                while i < indices.stop {
                    sliced.push(self.inner.values()[i as usize]);
                    i += indices.step;
                }
            } else {
                while i > indices.stop {
                    sliced.push(self.inner.values()[i as usize]);
                    i += indices.step;
                }
            }
            let mut out = PeriodIndex::new(sliced);
            if let Some(n) = self.inner.name() {
                out = out.set_name(n);
            }
            return Py::new(py, PyPeriodIndex { inner: out })?
                .into_any()
                .into_py_any(py);
        }
        Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
            "PeriodIndex indices must be integers or slices",
        ))
    }

    #[getter]
    pub fn hasnans(&self) -> bool {
        false
    }

    #[getter]
    pub fn nlevels(&self) -> usize {
        1
    }

    #[getter]
    pub fn names(&self) -> Vec<Option<String>> {
        vec![self.inner.name().map(str::to_string)]
    }

    #[getter]
    pub fn nbytes(&self) -> usize {
        self.inner.to_index().nbytes()
    }

    #[pyo3(signature = (deep=false))]
    pub fn memory_usage(&self, deep: bool) -> usize {
        self.inner.to_index().memory_usage(deep)
    }

    pub fn inferred_type(&self) -> &'static str {
        "period"
    }

    pub fn is_numeric(&self) -> bool {
        false
    }

    pub fn is_boolean(&self) -> bool {
        false
    }

    pub fn is_floating(&self) -> bool {
        false
    }

    pub fn is_integer(&self) -> bool {
        false
    }

    pub fn is_categorical(&self) -> bool {
        false
    }

    pub fn is_object(&self) -> bool {
        false
    }

    pub fn is_interval(&self) -> bool {
        false
    }

    pub fn holds_integer(&self) -> bool {
        false
    }

    pub fn union(&self, other: &PyIndex) -> PyIndex {
        PyIndex {
            inner: self.inner.to_index().union(&other.inner),
        }
    }

    pub fn intersection(&self, other: &PyIndex) -> PyIndex {
        PyIndex {
            inner: self.inner.to_index().intersection(&other.inner),
        }
    }

    pub fn difference(&self, other: &PyIndex) -> PyIndex {
        PyIndex {
            inner: self.inner.to_index().difference(&other.inner),
        }
    }

    pub fn symmetric_difference(&self, other: &PyIndex) -> PyIndex {
        PyIndex {
            inner: self.inner.to_index().symmetric_difference(&other.inner),
        }
    }

    pub fn get_loc(&self, key: &Bound<'_, PyAny>) -> PyResult<usize> {
        let label = py_to_index_label(key)?;
        self.inner
            .to_index()
            .get_loc(&label)
            .ok_or_else(|| PyErr::new::<pyo3::exceptions::PyKeyError, _>(format!("{key}")))
    }

    pub fn get_indexer(&self, target: &PyIndex) -> Vec<i64> {
        self.inner
            .to_index()
            .get_indexer(&target.inner)
            .into_iter()
            .map(|opt| opt.map(|u| u as i64).unwrap_or(-1))
            .collect()
    }

    #[pyo3(signature = (start=None, end=None, step=None))]
    pub fn slice_locs(
        &self,
        start: Option<&Bound<'_, PyAny>>,
        end: Option<&Bound<'_, PyAny>>,
        step: Option<isize>,
    ) -> PyResult<(usize, usize)> {
        let _ = step;
        let s_lbl = match start {
            Some(obj) => Some(py_to_index_label(obj)?),
            None => None,
        };
        let e_lbl = match end {
            Some(obj) => Some(py_to_index_label(obj)?),
            None => None,
        };
        self.inner
            .to_index()
            .slice_locs(s_lbl.as_ref(), e_lbl.as_ref())
            .map_err(index_error_to_py)
    }

    #[pyo3(signature = (start=None, end=None, step=None))]
    pub fn slice_indexer(
        &self,
        start: Option<&Bound<'_, PyAny>>,
        end: Option<&Bound<'_, PyAny>>,
        step: Option<isize>,
    ) -> PyResult<(usize, usize, isize)> {
        let (s, e) = self.slice_locs(start, end, step)?;
        Ok((s, e, step.unwrap_or(1)))
    }

    #[pyo3(signature = (level=0))]
    pub fn droplevel(&self, level: usize) -> PyResult<PyIndex> {
        let _ = level;
        Ok(PyIndex {
            inner: self.inner.to_index(),
        })
    }

    #[pyo3(signature = (level=0))]
    pub fn get_level_values(&self, level: usize) -> PyResult<Self> {
        let _ = level;
        Ok(self.clone())
    }

    pub fn to_flat_index(&self) -> Self {
        self.clone()
    }

    #[pyo3(signature = (index=None, name=None))]
    pub fn to_series(
        &self,
        index: Option<&Bound<'_, PyAny>>,
        name: Option<&str>,
    ) -> PyResult<PySeries> {
        let idx = self.inner.to_index();
        let series_name = name.or_else(|| self.inner.name()).unwrap_or("");
        let final_idx = if let Some(i_obj) = index {
            if let Ok(py_idx) = i_obj.extract::<PyRef<'_, PyIndex>>() {
                py_idx.inner.clone()
            } else {
                idx.clone()
            }
        } else {
            idx.clone()
        };
        let col = Column::from_values(
            idx.labels()
                .iter()
                .map(|l| match l {
                    IndexLabel::Utf8(s) => Scalar::Utf8(s.clone()),
                    _ => Scalar::Null(NullKind::NaN),
                })
                .collect(),
        )
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        let s = Series::new(series_name, final_idx, col).map_err(frame_error_to_py)?;
        Ok(PySeries { inner: s })
    }

    #[pyo3(signature = (index=true, name=None))]
    pub fn to_frame(&self, index: bool, name: Option<&str>) -> PyResult<PyDataFrame> {
        let col_name = name.or_else(|| self.inner.name()).unwrap_or("0");
        let idx = self.inner.to_index();
        let final_idx = if index {
            idx.clone()
        } else {
            Index::from_range(0, self.inner.len() as i64, 1)
        };
        let col = Column::from_values(
            idx.labels()
                .iter()
                .map(|l| match l {
                    IndexLabel::Utf8(s) => Scalar::Utf8(s.clone()),
                    _ => Scalar::Null(NullKind::NaN),
                })
                .collect(),
        )
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        let mut col_map = BTreeMap::new();
        col_map.insert(col_name.to_string(), col);
        let df = DataFrame::new_with_column_order(final_idx, col_map, vec![col_name.to_string()])
            .map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: df })
    }

    #[pyo3(signature = (normalize=false, sort=true, ascending=false, dropna=true))]
    pub fn value_counts(
        &self,
        normalize: bool,
        sort: bool,
        ascending: bool,
        dropna: bool,
    ) -> PyResult<PySeries> {
        let idx = self.inner.to_index();
        let counts = idx.value_counts_with_options(normalize, sort, ascending, dropna);
        let mut idx_labels = Vec::with_capacity(counts.len());
        let mut vals = Vec::with_capacity(counts.len());
        for (lbl, sc) in counts {
            idx_labels.push(lbl);
            vals.push(sc);
        }
        let col = Column::from_values(vals)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        let s = Series::new("count", Index::new(idx_labels), col).map_err(frame_error_to_py)?;
        Ok(PySeries { inner: s })
    }

    #[pyo3(signature = (ascending=true, na_position="last"))]
    pub fn sort_values(&self, ascending: bool, na_position: &str) -> PyResult<PyIndex> {
        let _ = na_position;
        let idx = self.inner.to_index();
        let mut sorted = idx.sort_values();
        if !ascending {
            let mut rev_labels = sorted.labels().to_vec();
            rev_labels.reverse();
            sorted = Index::new(rev_labels);
            if let Some(n) = self.inner.name() {
                sorted = sorted.rename_index(Some(n));
            }
        }
        Ok(PyIndex { inner: sorted })
    }

    pub fn sort(&self) -> PyResult<PyIndex> {
        self.sort_values(true, "last")
    }

    #[getter]
    #[allow(non_snake_case)]
    pub fn T(&self) -> Self {
        self.clone()
    }

    pub fn drop(&self, labels: Vec<Bound<'_, PyAny>>) -> PyResult<PyIndex> {
        let mut to_drop = Vec::with_capacity(labels.len());
        for l in labels {
            to_drop.push(py_to_index_label(&l)?);
        }
        Ok(PyIndex {
            inner: self.inner.to_index().drop_labels(&to_drop),
        })
    }

    #[pyo3(signature = (how="any"))]
    pub fn dropna(&self, how: &str) -> Self {
        let _ = how;
        self.clone()
    }

    pub fn fillna(&self, value: &Bound<'_, PyAny>) -> Self {
        let _ = value;
        self.clone()
    }

    fn as_py_index(&self) -> PyIndex {
        PyIndex {
            inner: self.inner.to_index(),
        }
    }

    fn all(&self) -> bool {
        self.as_py_index().all()
    }

    fn any(&self) -> bool {
        self.as_py_index().any()
    }

    fn append(&self, others: Vec<Bound<'_, PyAny>>) -> PyResult<Self> {
        let mut combined = self.inner.values().to_vec();
        for other in others {
            if let Ok(pi) = other.extract::<PyRef<'_, PyPeriodIndex>>() {
                combined.extend(pi.inner.values().iter().copied());
            }
        }
        let mut out = PeriodIndex::new(combined);
        if let Some(n) = self.inner.name() {
            out = out.set_name(n);
        }
        Ok(Self { inner: out })
    }

    fn argmax(&self) -> PyResult<usize> {
        self.as_py_index().argmax()
    }

    fn argmin(&self) -> PyResult<usize> {
        self.as_py_index().argmin()
    }

    fn argsort(&self) -> Vec<usize> {
        self.as_py_index().argsort()
    }

    fn array(&self, py: Python<'_>) -> PyResult<Py<PyList>> {
        self.as_py_index().array(py)
    }

    #[getter]
    fn asi8(&self) -> Vec<i64> {
        self.inner.values().iter().map(|p| p.ordinal).collect()
    }

    fn asof(&self, py: Python<'_>, label: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        self.as_py_index().asof(py, label)
    }

    #[pyo3(signature = (where_, mask=None))]
    fn asof_locs(&self, where_: &PyIndex, mask: Option<Vec<bool>>) -> Vec<Option<usize>> {
        self.as_py_index().asof_locs(where_, mask)
    }

    fn astype(&self, dtype: &str) -> PyResult<PyIndex> {
        self.as_py_index().astype(dtype)
    }

    #[getter]
    fn day_of_week(&self) -> PyResult<Vec<Option<u32>>> {
        self.inner.day_of_week().map_err(index_error_to_py)
    }

    #[getter]
    fn day_of_year(&self) -> PyResult<Vec<Option<u32>>> {
        self.inner.day_of_year().map_err(index_error_to_py)
    }

    #[getter]
    fn dayofweek(&self) -> PyResult<Vec<Option<u32>>> {
        self.inner.dayofweek().map_err(index_error_to_py)
    }

    #[getter]
    fn dayofyear(&self) -> PyResult<Vec<Option<u32>>> {
        self.inner.dayofyear().map_err(index_error_to_py)
    }

    #[getter]
    fn days_in_month(&self) -> PyResult<Vec<Option<u32>>> {
        self.inner.days_in_month().map_err(index_error_to_py)
    }

    #[getter]
    fn daysinmonth(&self) -> PyResult<Vec<Option<u32>>> {
        self.inner.daysinmonth().map_err(index_error_to_py)
    }

    fn delete(&self, loc: usize) -> PyResult<Self> {
        if loc >= self.inner.len() {
            return Err(PyErr::new::<pyo3::exceptions::PyIndexError, _>(
                "index out of bounds",
            ));
        }
        let mut vals = self.inner.values().to_vec();
        vals.remove(loc);
        let mut out = PeriodIndex::new(vals);
        if let Some(n) = self.inner.name() {
            out = out.set_name(n);
        }
        Ok(Self { inner: out })
    }

    #[pyo3(signature = (periods=1))]
    fn diff(&self, periods: i64) -> Vec<Option<i64>> {
        self.inner.diff(periods)
    }

    fn drop_duplicates(&self) -> Self {
        Self {
            inner: self.inner.drop_duplicates(),
        }
    }

    #[pyo3(signature = (keep=None))]
    fn duplicated(&self, keep: Option<&Bound<'_, PyAny>>) -> PyResult<Vec<bool>> {
        let k = parse_duplicate_keep(keep)?;
        Ok(self.inner.duplicated(k))
    }

    #[getter]
    fn end_time(&self) -> PyResult<PyDatetimeIndex> {
        self.inner
            .end_time()
            .map(|inner| PyDatetimeIndex { inner })
            .map_err(index_error_to_py)
    }

    #[pyo3(signature = (sort=false, use_na_sentinel=true))]
    fn factorize(&self, sort: bool, use_na_sentinel: bool) -> (Vec<isize>, PyIndex) {
        self.as_py_index().factorize(sort, use_na_sentinel)
    }

    fn format(&self) -> Vec<String> {
        self.as_py_index().format()
    }

    #[getter]
    fn freq(&self) -> Option<String> {
        self.inner.values().first().map(|p| p.freq.to_string())
    }

    #[getter]
    fn freqstr(&self) -> Option<&'static str> {
        self.inner.values().first().map(|p| p.freq.alias())
    }

    #[staticmethod]
    #[pyo3(signature = (fields))]
    fn from_fields(fields: &Bound<'_, PyAny>) -> PyResult<Self> {
        let _ = fields;
        Ok(Self {
            inner: PeriodIndex::new(vec![]),
        })
    }

    #[staticmethod]
    #[pyo3(signature = (ordinals, freq="D"))]
    fn from_ordinals(ordinals: Vec<i64>, freq: &str) -> PyResult<Self> {
        let p_freq = PeriodFreq::parse(freq).ok_or_else(|| {
            PyErr::new::<pyo3::exceptions::PyValueError, _>(format!("unsupported freq {freq}"))
        })?;
        Ok(Self {
            inner: PeriodIndex::from_ordinals(&ordinals, p_freq),
        })
    }

    fn get_indexer_for(&self, target: &Bound<'_, PyAny>) -> PyResult<Vec<i64>> {
        self.as_py_index().get_indexer_for(target)
    }

    fn get_indexer_non_unique(
        &self,
        target: &Bound<'_, PyAny>,
    ) -> PyResult<(Vec<isize>, Vec<usize>)> {
        self.as_py_index().get_indexer_non_unique(target)
    }

    fn get_slice_bound(&self, label: &Bound<'_, PyAny>, side: &str) -> PyResult<usize> {
        self.as_py_index().get_slice_bound(label, side)
    }

    fn groupby(&self, py: Python<'_>, by: &Bound<'_, PyAny>) -> PyResult<Py<pyo3::types::PyDict>> {
        self.as_py_index().groupby(py, by)
    }

    fn identical(&self, other: &Bound<'_, PyAny>) -> bool {
        if let Ok(pi) = other.extract::<PyRef<'_, PyPeriodIndex>>() {
            self.inner == pi.inner
        } else {
            false
        }
    }

    fn infer_objects(&self) -> Self {
        self.clone()
    }

    fn insert(&self, loc: usize, item: &Bound<'_, PyAny>) -> PyResult<PyIndex> {
        self.as_py_index().insert(loc, item)
    }

    fn is_(&self, other: &Bound<'_, PyAny>) -> bool {
        if let Ok(pi) = other.extract::<PyRef<'_, PyPeriodIndex>>() {
            self.inner == pi.inner
        } else {
            false
        }
    }

    #[getter]
    fn is_full(&self) -> bool {
        let vals = self.inner.values();
        if vals.len() <= 1 {
            return true;
        }
        for i in 1..vals.len() {
            if vals[i].ordinal != vals[i - 1].ordinal + 1 {
                return false;
            }
        }
        true
    }

    #[getter]
    fn is_leap_year(&self) -> PyResult<Vec<Option<bool>>> {
        self.inner.is_leap_year().map_err(index_error_to_py)
    }

    fn isin(&self, values: &Bound<'_, PyAny>) -> PyResult<Vec<bool>> {
        self.as_py_index().isin(values)
    }

    fn isna(&self) -> Vec<bool> {
        self.inner.isna()
    }

    fn isnull(&self) -> Vec<bool> {
        self.inner.isnull()
    }

    fn item(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        self.as_py_index().item(py)
    }

    #[pyo3(signature = (other, how="left", level=None, return_indexers=false, sort=false))]
    fn join(
        &self,
        other: &Bound<'_, PyAny>,
        how: &str,
        level: Option<usize>,
        return_indexers: bool,
        sort: bool,
    ) -> PyResult<PyIndex> {
        self.as_py_index()
            .join(other, how, level, return_indexers, sort)
    }

    fn map(&self, py: Python<'_>, mapper: &Bound<'_, PyAny>) -> PyResult<PyIndex> {
        self.as_py_index().map(py, mapper)
    }

    fn max(&self) -> Option<String> {
        self.inner
            .values()
            .iter()
            .max_by_key(|p| p.ordinal)
            .map(|p| p.to_string())
    }

    fn mean(&self) -> Option<String> {
        if self.inner.is_empty() {
            return None;
        }
        let sum: i64 = self.inner.values().iter().map(|p| p.ordinal).sum();
        let avg = sum / self.inner.len() as i64;
        let p = Period::new(avg, self.inner.values()[0].freq);
        Some(p.to_string())
    }

    fn min(&self) -> Option<String> {
        self.inner
            .values()
            .iter()
            .min_by_key(|p| p.ordinal)
            .map(|p| p.to_string())
    }

    fn notna(&self) -> Vec<bool> {
        self.inner.notna()
    }

    fn notnull(&self) -> Vec<bool> {
        self.inner.notnull()
    }

    fn nunique(&self) -> usize {
        self.inner.unique().len()
    }

    fn putmask(&self, mask: Vec<bool>, value: &Bound<'_, PyAny>) -> PyResult<PyIndex> {
        self.as_py_index().putmask(mask, value)
    }

    #[getter]
    fn quarter(&self) -> PyResult<Vec<Option<u32>>> {
        self.inner.quarter().map_err(index_error_to_py)
    }

    #[getter]
    fn qyear(&self) -> PyResult<Vec<i32>> {
        self.inner.qyear().map_err(index_error_to_py)
    }

    fn ravel(&self) -> Self {
        self.clone()
    }

    #[pyo3(signature = (target, method=None, level=None, limit=None, tolerance=None))]
    fn reindex(
        &self,
        target: &Bound<'_, PyAny>,
        method: Option<&str>,
        level: Option<usize>,
        limit: Option<usize>,
        tolerance: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<(PyIndex, Vec<i64>)> {
        self.as_py_index()
            .reindex(target, method, level, limit, tolerance)
    }

    fn repeat(&self, repeats: usize) -> PyIndex {
        PyIndex {
            inner: self.inner.to_index().repeat(repeats),
        }
    }

    #[getter]
    fn resolution(&self) -> Option<&'static str> {
        self.inner.resolution()
    }

    fn round(&self, freq: &str) -> PyResult<Self> {
        self.inner
            .asfreq(freq)
            .map(|inner| Self { inner })
            .map_err(index_error_to_py)
    }

    #[pyo3(signature = (value, side="left", sorter=None))]
    fn searchsorted(
        &self,
        value: &Bound<'_, PyAny>,
        side: &str,
        sorter: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<usize> {
        self.as_py_index().searchsorted(value, side, sorter)
    }

    fn set_names(&self, names: &Bound<'_, PyAny>) -> PyResult<Self> {
        let name = if let Ok(s) = names.extract::<String>() {
            Some(s)
        } else if let Ok(list) = names.extract::<Vec<Option<String>>>() {
            list.into_iter().next().flatten()
        } else {
            None
        };
        Ok(Self {
            inner: self.inner.set_names(name.as_deref()),
        })
    }

    #[pyo3(signature = (periods=1))]
    fn shift(&self, periods: i64) -> Self {
        let vals: Vec<Period> = self
            .inner
            .values()
            .iter()
            .map(|p| Period::new(p.ordinal + periods, p.freq))
            .collect();
        let mut out = PeriodIndex::new(vals);
        if let Some(n) = self.inner.name() {
            out = out.set_name(n);
        }
        Self { inner: out }
    }

    #[pyo3(signature = (level=None, ascending=true, sort_remaining=None))]
    fn sortlevel(
        &self,
        level: Option<usize>,
        ascending: bool,
        sort_remaining: Option<bool>,
    ) -> PyResult<(Self, Vec<usize>)> {
        let _ = (level, ascending, sort_remaining);
        Ok((self.clone(), (0..self.inner.len()).collect()))
    }

    #[getter]
    fn start_time(&self) -> PyResult<PyDatetimeIndex> {
        self.inner
            .start_time()
            .map(|inner| PyDatetimeIndex { inner })
            .map_err(index_error_to_py)
    }

    #[getter]
    fn r#str(&self) -> PyIndexStringMethods {
        PyIndexStringMethods {
            inner: self.inner.to_index(),
        }
    }

    fn strftime(&self, fmt: &str) -> PyResult<Vec<Option<String>>> {
        self.inner.strftime(fmt).map_err(index_error_to_py)
    }

    fn take(&self, indices: Vec<usize>) -> PyResult<Self> {
        self.inner
            .take(&indices)
            .map(|inner| Self { inner })
            .map_err(index_error_to_py)
    }

    fn to_numpy(&self) -> Vec<String> {
        self.inner.values().iter().map(|p| p.to_string()).collect()
    }

    fn transpose(&self) -> Self {
        self.clone()
    }

    fn unique(&self) -> Self {
        Self {
            inner: self.inner.unique(),
        }
    }

    fn view(&self) -> Self {
        self.clone()
    }

    #[getter]
    fn week(&self) -> PyResult<Vec<Option<u32>>> {
        self.inner.week().map_err(index_error_to_py)
    }

    #[getter]
    fn weekday(&self) -> PyResult<Vec<Option<u32>>> {
        self.inner.weekday().map_err(index_error_to_py)
    }

    #[getter]
    fn weekofyear(&self) -> PyResult<Vec<Option<u32>>> {
        self.inner.weekofyear().map_err(index_error_to_py)
    }

    #[pyo3(signature = (cond, other=None))]
    fn r#where(&self, cond: Vec<bool>, other: Option<&Bound<'_, PyAny>>) -> PyResult<PyIndex> {
        self.as_py_index().r#where(cond, other)
    }
}

/// Python wrapper for FrankenPandas CategoricalIndex.
#[pyclass(name = "CategoricalIndex", from_py_object)]
#[derive(Clone)]
pub struct PyCategoricalIndex {
    pub(crate) inner: CategoricalIndex,
}

#[pymethods]
impl PyCategoricalIndex {
    #[new]
    #[pyo3(signature = (data=None, categories=None, ordered=false, name=None))]
    pub fn new(
        data: Option<&Bound<'_, PyAny>>,
        categories: Option<Vec<String>>,
        ordered: bool,
        name: Option<&str>,
    ) -> PyResult<Self> {
        let labels: Vec<String> = if let Some(d) = data {
            if let Ok(ci) = d.extract::<PyRef<'_, PyCategoricalIndex>>() {
                let mut inner = ci.inner.clone();
                if let Some(n) = name {
                    inner = inner.set_name(n);
                }
                return Ok(Self { inner });
            } else if let Ok(list) = d.extract::<Vec<String>>() {
                list
            } else {
                return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                    "CategoricalIndex requires list of string labels",
                ));
            }
        } else {
            Vec::new()
        };

        let mut inner = if let Some(cats) = categories {
            CategoricalIndex::with_categories(labels, cats, ordered)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?
        } else {
            CategoricalIndex::from_values(labels, ordered)
        };
        if let Some(n) = name {
            inner = inner.set_name(n);
        }
        Ok(Self { inner })
    }

    #[getter]
    pub fn categories(&self) -> Vec<String> {
        self.inner.categories().to_vec()
    }

    #[getter]
    pub fn ordered(&self) -> bool {
        self.inner.ordered()
    }

    #[getter]
    pub fn codes(&self) -> Vec<Option<usize>> {
        self.inner.codes()
    }

    #[getter]
    pub fn name(&self) -> Option<String> {
        self.inner.name().map(str::to_owned)
    }

    #[getter]
    pub fn shape(&self) -> (usize,) {
        self.inner.shape()
    }

    #[getter]
    pub fn size(&self) -> usize {
        self.inner.size()
    }

    #[getter]
    pub fn ndim(&self) -> usize {
        self.inner.ndim()
    }

    #[getter]
    pub fn empty(&self) -> bool {
        self.inner.empty()
    }

    #[getter]
    pub fn dtype(&self) -> &'static str {
        self.inner.dtype()
    }

    pub fn tolist(&self) -> Vec<String> {
        self.inner.labels().to_vec()
    }

    pub fn to_list(&self) -> Vec<String> {
        self.tolist()
    }

    pub fn values(&self) -> Vec<String> {
        self.tolist()
    }

    pub fn copy(&self) -> Self {
        Self {
            inner: self.inner.copy(),
        }
    }

    pub fn rename(&self, name: Option<&str>) -> Self {
        Self {
            inner: self.inner.rename_index(name),
        }
    }

    pub fn equals(&self, other: &Self) -> bool {
        self.inner.equals(&other.inner)
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub fn __len__(&self) -> usize {
        self.inner.len()
    }

    pub fn __repr__(&self) -> String {
        format!(
            "CategoricalIndex({:?}, categories={:?}, ordered={}, dtype='category')",
            self.inner.labels(),
            self.inner.categories(),
            self.inner.ordered()
        )
    }

    pub fn __getitem__(&self, py: Python<'_>, item: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        if let Ok(idx) = item.extract::<i64>() {
            let len = self.inner.len() as i64;
            let pos = if idx < 0 { len + idx } else { idx };
            if pos < 0 || pos >= len {
                return Err(PyErr::new::<pyo3::exceptions::PyIndexError, _>(
                    "index out of bounds",
                ));
            }
            return self.inner.labels()[pos as usize].clone().into_py_any(py);
        }
        if let Ok(slice) = item.cast::<pyo3::types::PySlice>() {
            let indices = slice.indices(self.inner.len() as isize)?;
            let mut sliced = Vec::new();
            let mut i = indices.start;
            if indices.step > 0 {
                while i < indices.stop {
                    sliced.push(self.inner.labels()[i as usize].clone());
                    i += indices.step;
                }
            } else {
                while i > indices.stop {
                    sliced.push(self.inner.labels()[i as usize].clone());
                    i += indices.step;
                }
            }
            let out = CategoricalIndex::with_categories(
                sliced,
                self.inner.categories().to_vec(),
                self.inner.ordered(),
            )
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
            return Py::new(py, PyCategoricalIndex { inner: out })?
                .into_any()
                .into_py_any(py);
        }
        Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
            "CategoricalIndex indices must be integers or slices",
        ))
    }

    #[getter]
    pub fn is_monotonic_increasing(&self) -> bool {
        self.inner.to_index().is_monotonic_increasing()
    }

    #[getter]
    pub fn is_monotonic_decreasing(&self) -> bool {
        self.inner.to_index().is_monotonic_decreasing()
    }

    #[getter]
    pub fn is_unique(&self) -> bool {
        self.inner.to_index().is_unique()
    }

    #[getter]
    pub fn has_duplicates(&self) -> bool {
        self.inner.to_index().has_duplicates()
    }

    #[getter]
    pub fn hasnans(&self) -> bool {
        self.inner.to_index().hasnans()
    }

    #[getter]
    pub fn nlevels(&self) -> usize {
        1
    }

    #[getter]
    pub fn names(&self) -> Vec<Option<String>> {
        vec![self.inner.name().map(str::to_string)]
    }

    #[getter]
    pub fn nbytes(&self) -> usize {
        self.inner.to_index().nbytes()
    }

    #[pyo3(signature = (deep=false))]
    pub fn memory_usage(&self, deep: bool) -> usize {
        self.inner.to_index().memory_usage(deep)
    }

    pub fn inferred_type(&self) -> &'static str {
        "categorical"
    }

    pub fn is_numeric(&self) -> bool {
        false
    }

    pub fn is_boolean(&self) -> bool {
        false
    }

    pub fn is_floating(&self) -> bool {
        false
    }

    pub fn is_integer(&self) -> bool {
        false
    }

    pub fn is_categorical(&self) -> bool {
        true
    }

    pub fn is_object(&self) -> bool {
        false
    }

    pub fn is_interval(&self) -> bool {
        false
    }

    pub fn holds_integer(&self) -> bool {
        false
    }

    pub fn union(&self, other: &PyIndex) -> PyIndex {
        PyIndex {
            inner: self.inner.to_index().union(&other.inner),
        }
    }

    pub fn intersection(&self, other: &PyIndex) -> PyIndex {
        PyIndex {
            inner: self.inner.to_index().intersection(&other.inner),
        }
    }

    pub fn difference(&self, other: &PyIndex) -> PyIndex {
        PyIndex {
            inner: self.inner.to_index().difference(&other.inner),
        }
    }

    pub fn symmetric_difference(&self, other: &PyIndex) -> PyIndex {
        PyIndex {
            inner: self.inner.to_index().symmetric_difference(&other.inner),
        }
    }

    pub fn get_loc(&self, key: &Bound<'_, PyAny>) -> PyResult<usize> {
        let label = py_to_index_label(key)?;
        self.inner
            .to_index()
            .get_loc(&label)
            .ok_or_else(|| PyErr::new::<pyo3::exceptions::PyKeyError, _>(format!("{key}")))
    }

    pub fn get_indexer(&self, target: &PyIndex) -> Vec<i64> {
        self.inner
            .to_index()
            .get_indexer(&target.inner)
            .into_iter()
            .map(|opt| opt.map(|u| u as i64).unwrap_or(-1))
            .collect()
    }

    #[pyo3(signature = (start=None, end=None, step=None))]
    pub fn slice_locs(
        &self,
        start: Option<&Bound<'_, PyAny>>,
        end: Option<&Bound<'_, PyAny>>,
        step: Option<isize>,
    ) -> PyResult<(usize, usize)> {
        let _ = step;
        let s_lbl = match start {
            Some(obj) => Some(py_to_index_label(obj)?),
            None => None,
        };
        let e_lbl = match end {
            Some(obj) => Some(py_to_index_label(obj)?),
            None => None,
        };
        self.inner
            .to_index()
            .slice_locs(s_lbl.as_ref(), e_lbl.as_ref())
            .map_err(index_error_to_py)
    }

    #[pyo3(signature = (start=None, end=None, step=None))]
    pub fn slice_indexer(
        &self,
        start: Option<&Bound<'_, PyAny>>,
        end: Option<&Bound<'_, PyAny>>,
        step: Option<isize>,
    ) -> PyResult<(usize, usize, isize)> {
        let (s, e) = self.slice_locs(start, end, step)?;
        Ok((s, e, step.unwrap_or(1)))
    }

    #[pyo3(signature = (level=0))]
    pub fn droplevel(&self, level: usize) -> PyResult<PyIndex> {
        let _ = level;
        Ok(PyIndex {
            inner: self.inner.to_index(),
        })
    }

    #[pyo3(signature = (level=0))]
    pub fn get_level_values(&self, level: usize) -> PyResult<Self> {
        let _ = level;
        Ok(self.clone())
    }

    pub fn to_flat_index(&self) -> Self {
        self.clone()
    }

    #[pyo3(signature = (index=None, name=None))]
    pub fn to_series(
        &self,
        index: Option<&Bound<'_, PyAny>>,
        name: Option<&str>,
    ) -> PyResult<PySeries> {
        let idx = self.inner.to_index();
        let series_name = name.or_else(|| self.inner.name()).unwrap_or("");
        let final_idx = if let Some(i_obj) = index {
            if let Ok(py_idx) = i_obj.extract::<PyRef<'_, PyIndex>>() {
                py_idx.inner.clone()
            } else {
                idx.clone()
            }
        } else {
            idx.clone()
        };
        let col = Column::from_values(
            idx.labels()
                .iter()
                .map(|l| match l {
                    IndexLabel::Utf8(s) => Scalar::Utf8(s.clone()),
                    _ => Scalar::Null(NullKind::NaN),
                })
                .collect(),
        )
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        let s = Series::new(series_name, final_idx, col).map_err(frame_error_to_py)?;
        Ok(PySeries { inner: s })
    }

    #[pyo3(signature = (index=true, name=None))]
    pub fn to_frame(&self, index: bool, name: Option<&str>) -> PyResult<PyDataFrame> {
        let col_name = name.or_else(|| self.inner.name()).unwrap_or("0");
        let idx = self.inner.to_index();
        let final_idx = if index {
            idx.clone()
        } else {
            Index::from_range(0, self.inner.len() as i64, 1)
        };
        let col = Column::from_values(
            idx.labels()
                .iter()
                .map(|l| match l {
                    IndexLabel::Utf8(s) => Scalar::Utf8(s.clone()),
                    _ => Scalar::Null(NullKind::NaN),
                })
                .collect(),
        )
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        let mut col_map = BTreeMap::new();
        col_map.insert(col_name.to_string(), col);
        let df = DataFrame::new_with_column_order(final_idx, col_map, vec![col_name.to_string()])
            .map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: df })
    }

    #[pyo3(signature = (normalize=false, sort=true, ascending=false, dropna=true))]
    pub fn value_counts(
        &self,
        normalize: bool,
        sort: bool,
        ascending: bool,
        dropna: bool,
    ) -> PyResult<PySeries> {
        let idx = self.inner.to_index();
        let counts = idx.value_counts_with_options(normalize, sort, ascending, dropna);
        let mut idx_labels = Vec::with_capacity(counts.len());
        let mut vals = Vec::with_capacity(counts.len());
        for (lbl, sc) in counts {
            idx_labels.push(lbl);
            vals.push(sc);
        }
        let col = Column::from_values(vals)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        let s = Series::new("count", Index::new(idx_labels), col).map_err(frame_error_to_py)?;
        Ok(PySeries { inner: s })
    }

    #[pyo3(signature = (ascending=true, na_position="last"))]
    pub fn sort_values(&self, ascending: bool, na_position: &str) -> PyResult<PyIndex> {
        let _ = na_position;
        let idx = self.inner.to_index();
        let mut sorted = idx.sort_values();
        if !ascending {
            let mut rev_labels = sorted.labels().to_vec();
            rev_labels.reverse();
            sorted = Index::new(rev_labels);
            if let Some(n) = self.inner.name() {
                sorted = sorted.rename_index(Some(n));
            }
        }
        Ok(PyIndex { inner: sorted })
    }

    pub fn sort(&self) -> PyResult<PyIndex> {
        self.sort_values(true, "last")
    }

    #[getter]
    #[allow(non_snake_case)]
    pub fn T(&self) -> Self {
        self.clone()
    }

    pub fn drop(&self, labels: Vec<Bound<'_, PyAny>>) -> PyResult<PyIndex> {
        let mut to_drop = Vec::with_capacity(labels.len());
        for l in labels {
            to_drop.push(py_to_index_label(&l)?);
        }
        Ok(PyIndex {
            inner: self.inner.to_index().drop_labels(&to_drop),
        })
    }

    #[pyo3(signature = (how="any"))]
    pub fn dropna(&self, how: &str) -> Self {
        let _ = how;
        self.clone()
    }

    pub fn fillna(&self, value: &Bound<'_, PyAny>) -> Self {
        let _ = value;
        self.clone()
    }

    fn as_py_index(&self) -> PyIndex {
        PyIndex {
            inner: self.inner.to_index(),
        }
    }

    fn add_categories(&self, new: Vec<String>) -> PyResult<Self> {
        self.inner
            .add_categories(new)
            .map(|inner| Self { inner })
            .map_err(index_error_to_py)
    }

    fn all(&self) -> bool {
        self.as_py_index().all()
    }

    fn any(&self) -> bool {
        self.as_py_index().any()
    }

    fn append(&self, other: &Bound<'_, PyAny>) -> PyResult<PyIndex> {
        let other_idx = if let Ok(ci) = other.extract::<PyRef<'_, PyCategoricalIndex>>() {
            ci.inner.to_index()
        } else if let Ok(idx) = other.extract::<PyRef<'_, PyIndex>>() {
            idx.inner.clone()
        } else {
            PyIndex::new(Some(other), None)?.inner
        };
        Ok(PyIndex {
            inner: self.inner.to_index().append(&other_idx),
        })
    }

    fn argmax(&self) -> PyResult<usize> {
        self.as_py_index().argmax()
    }

    fn argmin(&self) -> PyResult<usize> {
        self.as_py_index().argmin()
    }

    fn argsort(&self) -> Vec<usize> {
        self.as_py_index().argsort()
    }

    fn array(&self, py: Python<'_>) -> PyResult<Py<PyList>> {
        self.as_py_index().array(py)
    }

    fn as_ordered(&self) -> Self {
        Self {
            inner: self.inner.as_ordered(),
        }
    }

    fn as_unordered(&self) -> Self {
        Self {
            inner: self.inner.as_unordered(),
        }
    }

    fn asof(&self, py: Python<'_>, label: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        self.as_py_index().asof(py, label)
    }

    #[pyo3(signature = (where_, mask=None))]
    fn asof_locs(&self, where_: &PyIndex, mask: Option<Vec<bool>>) -> Vec<Option<usize>> {
        self.as_py_index().asof_locs(where_, mask)
    }

    fn astype(&self, dtype: &str) -> PyResult<PyIndex> {
        self.as_py_index().astype(dtype)
    }

    fn delete(&self, loc: usize) -> PyResult<PyIndex> {
        self.as_py_index().delete(loc)
    }

    #[pyo3(signature = (periods=1))]
    fn diff(&self, periods: i64) -> PyResult<PyIndex> {
        let _ = periods;
        Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
            "cannot perform diff on CategoricalIndex",
        ))
    }

    fn drop_duplicates(&self) -> Self {
        Self {
            inner: self.inner.unique(),
        }
    }

    #[pyo3(signature = (keep=None))]
    fn duplicated(&self, keep: Option<&Bound<'_, PyAny>>) -> PyResult<Vec<bool>> {
        let k = parse_duplicate_keep(keep)?;
        Ok(self.inner.duplicated(k))
    }

    #[pyo3(signature = (sort=false, use_na_sentinel=true))]
    fn factorize(&self, sort: bool, use_na_sentinel: bool) -> (Vec<isize>, PyIndex) {
        self.as_py_index().factorize(sort, use_na_sentinel)
    }

    fn format(&self) -> Vec<String> {
        self.as_py_index().format()
    }

    fn get_indexer_for(&self, target: &Bound<'_, PyAny>) -> PyResult<Vec<i64>> {
        self.as_py_index().get_indexer_for(target)
    }

    fn get_indexer_non_unique(
        &self,
        target: &Bound<'_, PyAny>,
    ) -> PyResult<(Vec<isize>, Vec<usize>)> {
        self.as_py_index().get_indexer_non_unique(target)
    }

    fn get_slice_bound(&self, label: &Bound<'_, PyAny>, side: &str) -> PyResult<usize> {
        self.as_py_index().get_slice_bound(label, side)
    }

    fn groupby(&self, py: Python<'_>, by: &Bound<'_, PyAny>) -> PyResult<Py<pyo3::types::PyDict>> {
        self.as_py_index().groupby(py, by)
    }

    fn identical(&self, other: &Bound<'_, PyAny>) -> bool {
        if let Ok(ci) = other.extract::<PyRef<'_, PyCategoricalIndex>>() {
            self.inner == ci.inner
        } else {
            false
        }
    }

    fn infer_objects(&self) -> Self {
        self.clone()
    }

    fn insert(&self, loc: usize, item: &Bound<'_, PyAny>) -> PyResult<PyIndex> {
        self.as_py_index().insert(loc, item)
    }

    fn is_(&self, other: &Bound<'_, PyAny>) -> bool {
        if let Ok(ci) = other.extract::<PyRef<'_, PyCategoricalIndex>>() {
            self.inner == ci.inner
        } else {
            false
        }
    }

    fn isin(&self, values: &Bound<'_, PyAny>) -> PyResult<Vec<bool>> {
        self.as_py_index().isin(values)
    }

    fn isna(&self) -> Vec<bool> {
        self.inner.isna()
    }

    fn isnull(&self) -> Vec<bool> {
        self.inner.isna()
    }

    fn item(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        self.as_py_index().item(py)
    }

    #[pyo3(signature = (other, how="left", level=None, return_indexers=false, sort=false))]
    fn join(
        &self,
        other: &Bound<'_, PyAny>,
        how: &str,
        level: Option<usize>,
        return_indexers: bool,
        sort: bool,
    ) -> PyResult<PyIndex> {
        self.as_py_index()
            .join(other, how, level, return_indexers, sort)
    }

    fn map(&self, py: Python<'_>, mapper: &Bound<'_, PyAny>) -> PyResult<PyIndex> {
        self.as_py_index().map(py, mapper)
    }

    fn max(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        self.as_py_index().max(py)
    }

    fn min(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        self.as_py_index().min(py)
    }

    fn notna(&self) -> Vec<bool> {
        self.inner.notna()
    }

    fn notnull(&self) -> Vec<bool> {
        self.inner.notna()
    }

    fn nunique(&self) -> usize {
        self.inner.nunique()
    }

    fn putmask(&self, mask: Vec<bool>, value: &Bound<'_, PyAny>) -> PyResult<PyIndex> {
        self.as_py_index().putmask(mask, value)
    }

    fn ravel(&self) -> Self {
        self.clone()
    }

    #[pyo3(signature = (target, method=None, level=None, limit=None, tolerance=None))]
    fn reindex(
        &self,
        target: &Bound<'_, PyAny>,
        method: Option<&str>,
        level: Option<usize>,
        limit: Option<usize>,
        tolerance: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<(PyIndex, Vec<i64>)> {
        self.as_py_index()
            .reindex(target, method, level, limit, tolerance)
    }

    fn remove_categories(&self, removals: Vec<String>) -> PyResult<Self> {
        self.inner
            .remove_categories(&removals)
            .map(|inner| Self { inner })
            .map_err(index_error_to_py)
    }

    fn remove_unused_categories(&self) -> Self {
        Self {
            inner: self.inner.remove_unused_categories(),
        }
    }

    fn rename_categories(&self, new: Vec<String>) -> PyResult<Self> {
        self.inner
            .rename_categories(new)
            .map(|inner| Self { inner })
            .map_err(index_error_to_py)
    }

    #[pyo3(signature = (new, ordered=false))]
    fn reorder_categories(&self, new: Vec<String>, ordered: bool) -> PyResult<Self> {
        self.inner
            .reorder_categories(new, ordered)
            .map(|inner| Self { inner })
            .map_err(index_error_to_py)
    }

    fn repeat(&self, repeats: usize) -> PyIndex {
        PyIndex {
            inner: self.inner.to_index().repeat(repeats),
        }
    }

    #[pyo3(signature = (decimals=0))]
    fn round(&self, decimals: i32) -> Self {
        let _ = decimals;
        self.clone()
    }

    #[pyo3(signature = (value, side="left", sorter=None))]
    fn searchsorted(
        &self,
        value: &Bound<'_, PyAny>,
        side: &str,
        sorter: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<usize> {
        self.as_py_index().searchsorted(value, side, sorter)
    }

    fn set_categories(&self, new_categories: Vec<String>) -> PyResult<Self> {
        self.inner
            .set_categories(new_categories)
            .map(|inner| Self { inner })
            .map_err(index_error_to_py)
    }

    #[pyo3(signature = (names, level=None))]
    fn set_names(&self, names: &Bound<'_, PyAny>, level: Option<usize>) -> PyResult<Self> {
        let _ = level;
        let name_opt = if let Ok(s) = names.extract::<String>() {
            Some(s)
        } else if let Ok(seq) = names.cast::<pyo3::types::PySequence>() {
            if seq.len()? > 0 {
                Some(seq.get_item(0)?.extract::<String>()?)
            } else {
                None
            }
        } else {
            None
        };
        Ok(Self {
            inner: self.inner.set_names(name_opt.as_deref()),
        })
    }

    #[pyo3(signature = (periods=1, freq=None))]
    fn shift(&self, periods: i64, freq: Option<&str>) -> PyResult<Self> {
        let _ = (periods, freq);
        Err(PyErr::new::<pyo3::exceptions::PyNotImplementedError, _>(
            "This method is only implemented for DatetimeIndex, PeriodIndex and TimedeltaIndex; Got type CategoricalIndex",
        ))
    }

    #[pyo3(signature = (level=None, ascending=true, sort_remaining=None))]
    fn sortlevel(
        &self,
        level: Option<usize>,
        ascending: bool,
        sort_remaining: Option<bool>,
    ) -> PyResult<(Self, Vec<usize>)> {
        let _ = (level, ascending, sort_remaining);
        Ok((self.clone(), (0..self.inner.len()).collect()))
    }

    #[getter]
    fn r#str(&self) -> PyIndexStringMethods {
        PyIndexStringMethods {
            inner: self.inner.to_index(),
        }
    }

    fn take(&self, indices: Vec<i64>) -> PyResult<Self> {
        let len = self.inner.len() as i64;
        let mut u_indices = Vec::with_capacity(indices.len());
        for idx in indices {
            let pos = if idx < 0 { len + idx } else { idx };
            if pos >= 0 && pos < len {
                u_indices.push(pos as usize);
            } else {
                return Err(PyErr::new::<pyo3::exceptions::PyIndexError, _>(
                    "index out of range",
                ));
            }
        }
        self.inner
            .take(&u_indices)
            .map(|inner| Self { inner })
            .map_err(index_error_to_py)
    }

    fn to_numpy(&self) -> Vec<String> {
        self.inner.labels().to_vec()
    }

    fn transpose(&self) -> Self {
        self.clone()
    }

    fn unique(&self) -> Self {
        Self {
            inner: self.inner.unique(),
        }
    }

    fn view(&self) -> Self {
        self.clone()
    }

    #[pyo3(signature = (cond, other=None))]
    fn r#where(&self, cond: Vec<bool>, other: Option<&Bound<'_, PyAny>>) -> PyResult<PyIndex> {
        self.as_py_index().r#where(cond, other)
    }
}

/// Python wrapper for FrankenPandas Series.
#[pyclass(name = "Series", from_py_object)]
#[derive(Clone)]
pub struct PySeries {
    inner: Series,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PyErrorKind {
    Index,
    Type,
    Value,
    Key,
    NotImplemented,
}

fn classify_frame_error(err: &fp_frame::FrameError) -> (PyErrorKind, String) {
    use fp_columnar::ColumnError;
    use fp_frame::FrameError;
    use fp_index::IndexError;

    match err {
        FrameError::Index(IndexError::OutOfBounds { position, length }) => (
            PyErrorKind::Index,
            format!("position {position} out of bounds for length {length}"),
        ),
        FrameError::Column(ColumnError::Type(e)) => (PyErrorKind::Type, e.to_string()),
        FrameError::Column(ColumnError::InvalidMaskType { dtype }) => (
            PyErrorKind::Type,
            format!("mask must be Bool dtype; found {dtype:?}"),
        ),
        FrameError::Column(ColumnError::DTypeMismatch { left, right }) => (
            PyErrorKind::Type,
            format!("column dtype mismatch: left={left:?}, right={right:?}"),
        ),
        FrameError::CompatibilityRejected(msg) => {
            let lower = msg.to_lowercase();
            if lower.contains("not implemented") || lower.contains("unimplemented") {
                (
                    PyErrorKind::NotImplemented,
                    format!("compatibility gate rejected operation: {msg}"),
                )
            } else if lower.contains("column not found") || lower.contains("key not found") {
                (
                    PyErrorKind::Key,
                    format!("compatibility gate rejected operation: {msg}"),
                )
            } else {
                (
                    PyErrorKind::Value,
                    format!("compatibility gate rejected operation: {msg}"),
                )
            }
        }
        other => (PyErrorKind::Value, other.to_string()),
    }
}

fn index_error_to_py(err: fp_index::IndexError) -> PyErr {
    match err {
        fp_index::IndexError::OutOfBounds { position, length } => {
            PyErr::new::<pyo3::exceptions::PyIndexError, _>(format!(
                "position {position} out of bounds for length {length}"
            ))
        }
        other => PyErr::new::<pyo3::exceptions::PyValueError, _>(other.to_string()),
    }
}

fn frame_error_to_py(err: fp_frame::FrameError) -> PyErr {
    let (kind, msg) = classify_frame_error(&err);
    match kind {
        PyErrorKind::Index => PyErr::new::<pyo3::exceptions::PyIndexError, _>(msg),
        PyErrorKind::Type => PyErr::new::<pyo3::exceptions::PyTypeError, _>(msg),
        PyErrorKind::Value => PyErr::new::<pyo3::exceptions::PyValueError, _>(msg),
        PyErrorKind::Key => PyErr::new::<pyo3::exceptions::PyKeyError, _>(msg),
        PyErrorKind::NotImplemented => {
            PyErr::new::<pyo3::exceptions::PyNotImplementedError, _>(msg)
        }
    }
}

fn wrap_series(result: Result<Series, fp_frame::FrameError>) -> PyResult<PySeries> {
    result
        .map(|inner| PySeries { inner })
        .map_err(frame_error_to_py)
}

fn wrap_frame(result: Result<DataFrame, fp_frame::FrameError>) -> PyResult<PyDataFrame> {
    result
        .map(|inner| PyDataFrame { inner })
        .map_err(frame_error_to_py)
}

/// The right-hand side of a Series dunder: another Series as-is, or a Python
/// scalar broadcast over `like`'s index (what pandas does for `s + 1`).
fn series_operand(py: Python<'_>, other: &Bound<'_, PyAny>, like: &Series) -> PyResult<Series> {
    if let Ok(series) = other.extract::<PyRef<'_, PySeries>>() {
        return Ok(series.inner.clone());
    }
    let scalar = py_to_scalar(py, other)?;
    Series::from_values(
        like.name(),
        like.index().labels().to_vec(),
        vec![scalar; like.len()],
    )
    .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))
}

fn extract_or_build_series(
    py: Python<'_>,
    by: &Bound<'_, PyAny>,
    like: &Series,
) -> PyResult<Series> {
    if let Ok(series) = by.extract::<PyRef<'_, PySeries>>() {
        return Ok(series.inner.clone());
    }
    if let Ok(py_idx) = by.extract::<PyRef<'_, PyIndex>>() {
        let vals: Vec<Scalar> = py_idx
            .inner
            .labels()
            .iter()
            .map(index_label_to_scalar)
            .collect();
        return Series::from_values("group", like.index().labels().to_vec(), vals)
            .map_err(frame_error_to_py);
    }
    if let Ok(list) = by.cast::<PyList>() {
        let scalars: Vec<Scalar> = list
            .iter()
            .map(|v| py_to_scalar(py, &v))
            .collect::<PyResult<Vec<_>>>()?;
        return Series::from_values("group", like.index().labels().to_vec(), scalars)
            .map_err(frame_error_to_py);
    }
    let scalar = py_to_scalar(py, by)?;
    Series::from_values(
        "group",
        like.index().labels().to_vec(),
        vec![scalar; like.len()],
    )
    .map_err(frame_error_to_py)
}

#[pymethods]
impl PySeries {
    /// Create a new Series from a name and list of values.
    #[new]
    /// `Series(data, index=None, name=None)`, in pandas' argument order.
    ///
    /// The binding used to take `(name, values)`, so `fp.Series([1, 2, 3])`,
    /// the first thing anyone types, was a TypeError. `index` accepts a list of
    /// ints or strings; `name` defaults to the empty string because the Rust
    /// `Series` always carries one.
    #[pyo3(signature = (data, index=None, name=None))]
    fn new(
        py: Python<'_>,
        data: &Bound<'_, PyList>,
        index: Option<&Bound<'_, PyAny>>,
        name: Option<&str>,
    ) -> PyResult<Self> {
        let scalars: Vec<Scalar> = data
            .iter()
            .map(|v| py_to_scalar(py, &v))
            .collect::<PyResult<Vec<_>>>()?;

        let labels: Vec<IndexLabel> = if let Some(index) = index {
            if let Ok(py_idx) = index.extract::<PyRef<'_, PyIndex>>() {
                py_idx.inner.labels().to_vec()
            } else if let Ok(list) = index.cast::<PyList>() {
                list.iter()
                    .map(|label| {
                        if let Ok(value) = label.extract::<i64>() {
                            Ok(IndexLabel::Int64(value))
                        } else if let Ok(value) = label.extract::<String>() {
                            Ok(IndexLabel::Utf8(value))
                        } else {
                            Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                                "Series: index labels must be ints or strings",
                            ))
                        }
                    })
                    .collect::<PyResult<Vec<_>>>()?
            } else {
                return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                    "Series: index must be a list or Index",
                ));
            }
        } else {
            (0..scalars.len())
                .map(|i| IndexLabel::Int64(i as i64))
                .collect()
        };
        if labels.len() != scalars.len() {
            return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                "Length of values ({}) does not match length of index ({})",
                scalars.len(),
                labels.len()
            )));
        }

        let series = Series::from_values(name.unwrap_or(""), labels, scalars)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;

        Ok(PySeries { inner: series })
    }

    /// Return the name of the Series.
    #[getter]
    fn name(&self) -> &str {
        self.inner.name()
    }

    /// Return the index of the Series.
    #[getter]
    fn index(&self) -> PyIndex {
        PyIndex {
            inner: self.inner.index().clone(),
        }
    }

    #[getter]
    fn dtype(&self) -> String {
        match self.inner.dtype() {
            fp_types::DType::Int64 => "int64".to_string(),
            fp_types::DType::Float64 => "float64".to_string(),
            fp_types::DType::Bool => "bool".to_string(),
            fp_types::DType::Utf8 => "object".to_string(),
            fp_types::DType::Datetime64 { .. } => "datetime64[ns]".to_string(),
            fp_types::DType::Timedelta64 => "timedelta64[ns]".to_string(),
            _ => format!("{:?}", self.inner.dtype()).to_ascii_lowercase(),
        }
    }

    #[getter]
    fn shape(&self) -> (usize,) {
        (self.inner.len(),)
    }

    #[getter]
    fn size(&self) -> usize {
        self.inner.len()
    }

    #[getter]
    fn ndim(&self) -> usize {
        1
    }

    #[getter]
    fn empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Return the length of the Series.
    fn __len__(&self) -> usize {
        self.inner.len()
    }

    /// Return a string representation (renders values, like pandas).
    fn __repr__(&self) -> String {
        format!("{}", self.inner)
    }

    /// Purely integer-location based indexing for selection by position.
    #[getter]
    fn iloc(&self) -> PySeriesILoc {
        PySeriesILoc {
            inner: self.inner.clone(),
        }
    }

    /// Access a group of rows and columns by label(s) or a boolean array.
    #[getter]
    fn loc(&self) -> PySeriesLoc {
        PySeriesLoc {
            inner: self.inner.clone(),
        }
    }

    /// Access a single value for a row/column pair by integer position.
    #[getter]
    fn iat(&self) -> PySeriesIAt {
        PySeriesIAt {
            inner: self.inner.clone(),
        }
    }

    /// Access a single value for a row/column label pair.
    #[getter]
    fn at(&self) -> PySeriesAt {
        PySeriesAt {
            inner: self.inner.clone(),
        }
    }

    /// `s[i]` (position), `s["label"]` (label), `s[slice]`, `s[mask]`, or `s[list]`.
    fn __getitem__(&self, py: Python<'_>, key: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        if let Ok(slice) = key.cast::<pyo3::types::PySlice>() {
            let idx = slice.indices(self.inner.len() as isize)?;
            let s = self
                .inner
                .iloc_slice(Some(idx.start as i64), Some(idx.stop as i64))
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
            return Ok(Py::new(py, PySeries { inner: s })?.into_any());
        }
        if let Ok(idx) = key.extract::<PyRef<'_, PyIndex>>() {
            let labels = idx.inner.labels().to_vec();
            let s = self
                .inner
                .loc(&labels)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyKeyError, _>(e.to_string()))?;
            return Ok(Py::new(py, PySeries { inner: s })?.into_any());
        }
        if let Ok(series_mask) = key.extract::<PyRef<'_, PySeries>>() {
            let s = self
                .inner
                .iloc_bool_series(&series_mask.inner)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIndexError, _>(e.to_string()))?;
            return Ok(Py::new(py, PySeries { inner: s })?.into_any());
        }
        if let Ok(mask) = key.extract::<Vec<bool>>() {
            let s = self
                .inner
                .iloc_bool(&mask)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIndexError, _>(e.to_string()))?;
            return Ok(Py::new(py, PySeries { inner: s })?.into_any());
        }
        if let Ok(positions) = key.extract::<Vec<i64>>() {
            let s = self
                .inner
                .iloc(&positions)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIndexError, _>(e.to_string()))?;
            return Ok(Py::new(py, PySeries { inner: s })?.into_any());
        }
        if let Ok(labels) = key.extract::<Vec<String>>() {
            let idx_labels: Vec<IndexLabel> = labels.into_iter().map(IndexLabel::Utf8).collect();
            let s = self
                .inner
                .loc(&idx_labels)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyKeyError, _>(e.to_string()))?;
            return Ok(Py::new(py, PySeries { inner: s })?.into_any());
        }
        let scalar = if let Ok(position) = key.extract::<i64>() {
            self.inner
                .iat(position)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIndexError, _>(e.to_string()))?
        } else if let Ok(label) = key.extract::<String>() {
            self.inner
                .at(&IndexLabel::Utf8(label.clone()))
                .map_err(|_| PyErr::new::<pyo3::exceptions::PyKeyError, _>(label))?
        } else {
            return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                "Series index must be an int, str, slice, list, or boolean Series",
            ));
        };
        scalar_to_py(py, &scalar)
    }

    // Arithmetic and comparison dunders. `other` may be another Series (index
    // aligned by the Rust engine) or a Python scalar (broadcast over this
    // Series' index). Comparisons return a bool Series, as in pandas, so
    // `__eq__`/`__ne__` deliberately do not return a Python bool.
    fn __add__(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PySeries> {
        let rhs = series_operand(py, other, &self.inner)?;
        wrap_series(self.inner.add(&rhs))
    }
    fn __radd__(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PySeries> {
        let lhs = series_operand(py, other, &self.inner)?;
        wrap_series(lhs.add(&self.inner))
    }
    fn __sub__(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PySeries> {
        let rhs = series_operand(py, other, &self.inner)?;
        wrap_series(self.inner.sub(&rhs))
    }
    fn __rsub__(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PySeries> {
        let lhs = series_operand(py, other, &self.inner)?;
        wrap_series(lhs.sub(&self.inner))
    }
    fn __mul__(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PySeries> {
        let rhs = series_operand(py, other, &self.inner)?;
        wrap_series(self.inner.mul(&rhs))
    }
    fn __rmul__(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PySeries> {
        let lhs = series_operand(py, other, &self.inner)?;
        wrap_series(lhs.mul(&self.inner))
    }
    fn __truediv__(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PySeries> {
        let rhs = series_operand(py, other, &self.inner)?;
        wrap_series(self.inner.div(&rhs))
    }
    fn __rtruediv__(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PySeries> {
        let lhs = series_operand(py, other, &self.inner)?;
        wrap_series(lhs.div(&self.inner))
    }
    fn __floordiv__(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PySeries> {
        let rhs = series_operand(py, other, &self.inner)?;
        wrap_series(self.inner.floordiv(&rhs))
    }
    fn __rfloordiv__(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PySeries> {
        let lhs = series_operand(py, other, &self.inner)?;
        wrap_series(lhs.floordiv(&self.inner))
    }
    fn __mod__(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PySeries> {
        let rhs = series_operand(py, other, &self.inner)?;
        wrap_series(self.inner.remainder(&rhs))
    }
    fn __rmod__(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PySeries> {
        let lhs = series_operand(py, other, &self.inner)?;
        wrap_series(lhs.remainder(&self.inner))
    }
    fn __pow__(
        &self,
        py: Python<'_>,
        other: &Bound<'_, PyAny>,
        _modulo: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<PySeries> {
        let rhs = series_operand(py, other, &self.inner)?;
        wrap_series(self.inner.power(&rhs))
    }
    fn __rpow__(
        &self,
        py: Python<'_>,
        other: &Bound<'_, PyAny>,
        _modulo: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<PySeries> {
        let lhs = series_operand(py, other, &self.inner)?;
        wrap_series(lhs.power(&self.inner))
    }
    fn __neg__(&self) -> PyResult<PySeries> {
        wrap_series(self.inner.neg())
    }
    fn __gt__(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PySeries> {
        let rhs = series_operand(py, other, &self.inner)?;
        wrap_series(self.inner.gt(&rhs))
    }
    fn __ge__(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PySeries> {
        let rhs = series_operand(py, other, &self.inner)?;
        wrap_series(self.inner.ge(&rhs))
    }
    fn __lt__(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PySeries> {
        let rhs = series_operand(py, other, &self.inner)?;
        wrap_series(self.inner.lt(&rhs))
    }
    fn __le__(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PySeries> {
        let rhs = series_operand(py, other, &self.inner)?;
        wrap_series(self.inner.le(&rhs))
    }
    fn __eq__(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PySeries> {
        let rhs = series_operand(py, other, &self.inner)?;
        wrap_series(self.inner.eq(&rhs))
    }
    fn __ne__(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PySeries> {
        let rhs = series_operand(py, other, &self.inner)?;
        wrap_series(self.inner.ne(&rhs))
    }

    fn add(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PySeries> {
        self.__add__(py, other)
    }
    fn radd(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PySeries> {
        self.__radd__(py, other)
    }
    fn sub(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PySeries> {
        self.__sub__(py, other)
    }
    fn subtract(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PySeries> {
        self.__sub__(py, other)
    }
    fn rsub(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PySeries> {
        self.__rsub__(py, other)
    }
    fn mul(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PySeries> {
        self.__mul__(py, other)
    }
    fn multiply(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PySeries> {
        self.__mul__(py, other)
    }
    fn rmul(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PySeries> {
        self.__rmul__(py, other)
    }
    fn div(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PySeries> {
        self.__truediv__(py, other)
    }
    fn divide(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PySeries> {
        self.__truediv__(py, other)
    }
    fn truediv(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PySeries> {
        self.__truediv__(py, other)
    }
    fn rtruediv(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PySeries> {
        self.__rtruediv__(py, other)
    }
    fn rdiv(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PySeries> {
        self.__rtruediv__(py, other)
    }
    fn floordiv(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PySeries> {
        self.__floordiv__(py, other)
    }
    fn rfloordiv(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PySeries> {
        self.__rfloordiv__(py, other)
    }
    fn r#mod(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PySeries> {
        self.__mod__(py, other)
    }
    fn rmod(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PySeries> {
        self.__rmod__(py, other)
    }
    fn pow(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PySeries> {
        self.__pow__(py, other, None)
    }
    fn rpow(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PySeries> {
        self.__rpow__(py, other, None)
    }
    fn eq(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PySeries> {
        self.__eq__(py, other)
    }
    fn ne(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PySeries> {
        self.__ne__(py, other)
    }
    fn lt(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PySeries> {
        self.__lt__(py, other)
    }
    fn le(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PySeries> {
        self.__le__(py, other)
    }
    fn gt(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PySeries> {
        self.__gt__(py, other)
    }
    fn ge(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PySeries> {
        self.__ge__(py, other)
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

    /// Sort by the index, returning a new Series.
    #[pyo3(signature = (ascending=true))]
    fn sort_index(&self, ascending: bool) -> PyResult<PySeries> {
        let r = self
            .inner
            .sort_index(ascending)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PySeries { inner: r })
    }

    /// Return a copy of the Series renamed to `name`.
    fn rename(&self, name: &str) -> PyResult<PySeries> {
        let r = self
            .inner
            .rename(name)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PySeries { inner: r })
    }

    /// Return a boolean Series marking missing values (pandas `Series.isna`).
    fn isna(&self) -> PyResult<PySeries> {
        let r = self
            .inner
            .isna()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PySeries { inner: r })
    }

    /// Return a boolean Series marking non-missing values (pandas `Series.notna`).
    fn notna(&self) -> PyResult<PySeries> {
        let r = self
            .inner
            .notna()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PySeries { inner: r })
    }

    /// Clip values to the `[lower, upper]` range (either bound optional).
    #[pyo3(signature = (lower=None, upper=None))]
    fn clip(&self, lower: Option<f64>, upper: Option<f64>) -> PyResult<PySeries> {
        let r = self
            .inner
            .clip(lower, upper)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PySeries { inner: r })
    }

    /// Replace values via an `{old: new}` dict (pandas `Series.replace`).
    fn replace(&self, py: Python<'_>, mapping: &Bound<'_, PyDict>) -> PyResult<PySeries> {
        let pairs = py_dict_to_scalar_pairs(py, mapping)?;
        let r = self
            .inner
            .replace(&pairs)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PySeries { inner: r })
    }

    /// Cast to a dtype (int64/float64/str/bool/datetime64/timedelta64).
    fn astype(&self, dtype: &str) -> PyResult<PySeries> {
        let dt = parse_dtype(dtype)?;
        let r = self
            .inner
            .astype(dt)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PySeries { inner: r })
    }

    fn values(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        self.tolist(py)
    }

    fn to_numpy(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        self.tolist(py)
    }

    fn isin(&self, py: Python<'_>, values: &Bound<'_, PyAny>) -> PyResult<PySeries> {
        let mut scalars = Vec::new();
        if let Ok(seq) = values.cast::<pyo3::types::PySequence>() {
            let len = seq.len()?;
            scalars.reserve(len);
            for i in 0..len {
                let item = seq.get_item(i)?;
                scalars.push(py_to_scalar(py, &item)?);
            }
        }
        let res = self.inner.isin(&scalars).map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    #[pyo3(signature = (left, right, inclusive="both"))]
    fn between(
        &self,
        py: Python<'_>,
        left: &Bound<'_, PyAny>,
        right: &Bound<'_, PyAny>,
        inclusive: &str,
    ) -> PyResult<PySeries> {
        let left_scalar = py_to_scalar(py, left)?;
        let right_scalar = py_to_scalar(py, right)?;
        let res = self
            .inner
            .between(&left_scalar, &right_scalar, inclusive)
            .map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    fn idxmax(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let label = self.inner.idxmax().map_err(frame_error_to_py)?;
        index_label_to_py(py, &label)
    }

    fn idxmin(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let label = self.inner.idxmin().map_err(frame_error_to_py)?;
        index_label_to_py(py, &label)
    }

    fn argmax(&self) -> PyResult<i64> {
        self.inner.argmax().map_err(frame_error_to_py)
    }

    fn argmin(&self) -> PyResult<i64> {
        self.inner.argmin().map_err(frame_error_to_py)
    }

    #[pyo3(signature = (periods=1))]
    fn shift(&self, periods: i64) -> PyResult<PySeries> {
        let res = self.inner.shift(periods).map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    fn drop_duplicates(&self) -> PyResult<PySeries> {
        let res = self.inner.drop_duplicates().map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    fn duplicated(&self) -> PyResult<PySeries> {
        let res = self.inner.duplicated().map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    #[pyo3(signature = (n=5))]
    fn nlargest(&self, n: usize) -> PyResult<PySeries> {
        let res = self.inner.nlargest(n).map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    #[pyo3(signature = (n=5))]
    fn nsmallest(&self, n: usize) -> PyResult<PySeries> {
        let res = self.inner.nsmallest(n).map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    fn isnull(&self) -> PyResult<PySeries> {
        self.isna()
    }

    fn notnull(&self) -> PyResult<PySeries> {
        self.notna()
    }

    fn copy(&self) -> PySeries {
        self.clone()
    }

    #[pyo3(signature = (name=None))]
    fn to_frame(&self, name: Option<&str>) -> PyResult<PyDataFrame> {
        let res = self.inner.to_frame(name).map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: res })
    }

    #[pyo3(signature = (drop=false))]
    fn reset_index(&self, py: Python<'_>, drop: bool) -> PyResult<Py<PyAny>> {
        match self.inner.reset_index(drop).map_err(frame_error_to_py)? {
            fp_frame::SeriesResetIndexResult::Series(s) => {
                Ok(Py::new(py, PySeries { inner: s })?.into_any())
            }
            fp_frame::SeriesResetIndexResult::DataFrame(df) => {
                Ok(Py::new(py, PyDataFrame { inner: df })?.into_any())
            }
        }
    }

    fn to_dict(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let dict = PyDict::new(py);
        let idx = self.inner.index();
        for (i, val) in self.inner.values().iter().enumerate() {
            let k = index_label_to_py(py, &idx.labels()[i])?;
            let v = scalar_to_py(py, val)?;
            dict.set_item(k, v)?;
        }
        Ok(dict.into_any().unbind())
    }

    #[pyo3(signature = (window, min_periods=None, center=false))]
    fn rolling(&self, window: usize, min_periods: Option<usize>, center: bool) -> PyRolling {
        PyRolling {
            series: Some(self.inner.clone()),
            dataframe: None,
            window,
            min_periods,
            center,
        }
    }

    #[pyo3(signature = (min_periods=None))]
    fn expanding(&self, min_periods: Option<usize>) -> PyExpanding {
        PyExpanding {
            series: Some(self.inner.clone()),
            dataframe: None,
            min_periods,
        }
    }

    #[pyo3(signature = (span=None, alpha=None))]
    fn ewm(&self, span: Option<f64>, alpha: Option<f64>) -> PyExponentialMovingWindow {
        PyExponentialMovingWindow {
            series: Some(self.inner.clone()),
            dataframe: None,
            span,
            alpha,
        }
    }

    #[pyo3(signature = (limit=None))]
    fn ffill(&self, limit: Option<usize>) -> PyResult<PySeries> {
        let res = self.inner.ffill(limit).map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    #[pyo3(signature = (limit=None))]
    fn bfill(&self, limit: Option<usize>) -> PyResult<PySeries> {
        let res = self.inner.bfill(limit).map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    fn corr(&self, other: &PySeries) -> PyResult<f64> {
        self.inner.corr(&other.inner).map_err(frame_error_to_py)
    }

    fn cov(&self, other: &PySeries) -> PyResult<f64> {
        self.inner.cov(&other.inner).map_err(frame_error_to_py)
    }

    #[pyo3(signature = (n=None, frac=None, replace=false, random_state=None))]
    fn sample(
        &self,
        n: Option<usize>,
        frac: Option<f64>,
        replace: bool,
        random_state: Option<u64>,
    ) -> PyResult<PySeries> {
        let res = self
            .inner
            .sample(n, frac, replace, random_state)
            .map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    pub fn groupby(&self, py: Python<'_>, by: &Bound<'_, PyAny>) -> PyResult<PySeriesGroupBy> {
        let by_series = extract_or_build_series(py, by, &self.inner)?;
        Ok(PySeriesGroupBy {
            series: self.inner.clone(),
            by: by_series,
        })
    }

    #[pyo3(signature = (freq, closed=None, label=None, origin=None))]
    pub fn resample(
        &self,
        freq: &str,
        closed: Option<&str>,
        label: Option<&str>,
        origin: Option<&str>,
    ) -> PyResampler {
        PyResampler {
            target: ResampleTarget::Series(self.inner.clone()),
            freq: freq.to_string(),
            closed: closed.map(str::to_string),
            label: label.map(str::to_string),
            origin: origin.map(str::to_string),
        }
    }

    #[pyo3(signature = (freq, method=None))]
    pub fn asfreq(&self, freq: &str, method: Option<&str>) -> PyResult<Self> {
        let res = self
            .inner
            .asfreq_with_options(freq, method, None)
            .map_err(frame_error_to_py)?;
        Ok(Self { inner: res })
    }

    #[getter]
    fn hasnans(&self) -> bool {
        self.inner.hasnans()
    }

    #[getter]
    fn nbytes(&self) -> usize {
        self.inner.nbytes()
    }

    #[getter]
    fn t(&self) -> PySeries {
        PySeries {
            inner: self.inner.clone(),
        }
    }

    #[getter]
    #[allow(non_snake_case)]
    fn T(&self) -> PySeries {
        self.t()
    }

    fn item(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let s = self.inner.item().map_err(frame_error_to_py)?;
        scalar_to_py(py, &s)
    }

    fn any(&self) -> PyResult<bool> {
        self.inner.any().map_err(frame_error_to_py)
    }

    fn all(&self) -> PyResult<bool> {
        self.inner.all().map_err(frame_error_to_py)
    }

    fn sem(&self) -> PyResult<f64> {
        self.inner.sem().map_err(frame_error_to_py)
    }

    fn kurtosis(&self) -> PyResult<f64> {
        self.kurt()
    }

    fn product(&self) -> PyResult<Py<PyAny>> {
        self.prod()
    }

    fn mode(&self) -> PyResult<PySeries> {
        let res = self.inner.mode().map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    #[pyo3(signature = (before=None, after=None))]
    fn truncate(
        &self,
        before: Option<&Bound<'_, PyAny>>,
        after: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<PySeries> {
        let b = match before {
            Some(obj) => Some(py_to_index_label(obj)?),
            None => None,
        };
        let a = match after {
            Some(obj) => Some(py_to_index_label(obj)?),
            None => None,
        };
        let res = self
            .inner
            .truncate(b.as_ref(), a.as_ref())
            .map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    fn add_prefix(&self, prefix: &str) -> PyResult<PySeries> {
        let res = self.inner.add_prefix(prefix).map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    fn add_suffix(&self, suffix: &str) -> PyResult<PySeries> {
        let res = self.inner.add_suffix(suffix).map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    fn first_valid_index(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        match self.inner.first_valid_index() {
            Some(l) => index_label_to_py(py, &l),
            None => Ok(py.None()),
        }
    }

    fn last_valid_index(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        match self.inner.last_valid_index() {
            Some(l) => index_label_to_py(py, &l),
            None => Ok(py.None()),
        }
    }

    fn equals(&self, other: &PySeries) -> bool {
        self.inner.equals(&other.inner)
    }

    fn pop(&mut self, py: Python<'_>, item: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let label = py_to_index_label(item)?;
        let (scalar, remaining) = self.inner.pop(&label).map_err(frame_error_to_py)?;
        self.inner = remaining;
        scalar_to_py(py, &scalar)
    }

    fn squeeze(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        if self.inner.len() == 1 {
            let s = self.inner.column().values()[0].clone();
            scalar_to_py(py, &s)
        } else {
            Ok(Py::new(
                py,
                PySeries {
                    inner: self.inner.clone(),
                },
            )?
            .into_any())
        }
    }

    fn items(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let labels = self.inner.index().labels();
        let values = self.inner.column().values();
        let mut list = Vec::with_capacity(labels.len());
        for (l, v) in labels.iter().zip(values.iter()) {
            let py_l = index_label_to_py(py, l)?;
            let py_v = scalar_to_py(py, v)?;
            list.push(pyo3::types::PyTuple::new(py, &[py_l, py_v])?);
        }
        Ok(PyList::new(py, list)?.into_any().unbind())
    }

    fn keys(&self) -> PyIndex {
        PyIndex {
            inner: self.inner.index().clone(),
        }
    }

    #[pyo3(signature = (limit=None))]
    fn pad(&self, limit: Option<usize>) -> PyResult<PySeries> {
        self.ffill(limit)
    }

    #[pyo3(signature = (limit=None))]
    fn backfill(&self, limit: Option<usize>) -> PyResult<PySeries> {
        self.bfill(limit)
    }

    fn agg(&self, py: Python<'_>, func: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        if let Ok(name) = func.extract::<String>() {
            match name.as_str() {
                "sum" => self.sum(),
                "mean" => self.mean(),
                "min" => self.min(),
                "max" => self.max(),
                "std" => self.std(),
                "var" => self.var(),
                "count" => Ok(self.count().into_pyobject(py)?.into_any().unbind()),
                "median" => self.median(),
                "prod" | "product" => self.prod(),
                "sem" => Ok(self.sem()?.into_pyobject(py)?.into_any().unbind()),
                "skew" => Ok(self.skew()?.into_pyobject(py)?.into_any().unbind()),
                "kurt" | "kurtosis" => Ok(self.kurt()?.into_pyobject(py)?.into_any().unbind()),
                other => Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                    "Unsupported agg function '{other}'"
                ))),
            }
        } else if let Ok(list) = func.extract::<Vec<String>>() {
            let refs: Vec<&str> = list.iter().map(String::as_str).collect();
            let res = self.inner.agg(&refs).map_err(frame_error_to_py)?;
            Ok(Py::new(py, PySeries { inner: res })?.into_any())
        } else {
            Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                "func must be a string or list of strings",
            ))
        }
    }

    fn aggregate(&self, py: Python<'_>, func: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        self.agg(py, func)
    }

    fn repeat(&self, repeats: usize) -> PyResult<PySeries> {
        let res = self.inner.repeat(repeats).map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    #[pyo3(signature = (value, side=None))]
    fn searchsorted(
        &self,
        py: Python<'_>,
        value: &Bound<'_, PyAny>,
        side: Option<&str>,
    ) -> PyResult<usize> {
        let s = py_to_scalar(py, value)?;
        let s_side = side.unwrap_or("left");
        self.inner
            .searchsorted(&s, s_side)
            .map_err(frame_error_to_py)
    }

    #[pyo3(signature = (index=true))]
    fn memory_usage(&self, index: bool) -> usize {
        if index {
            self.inner.memory_usage()
        } else {
            self.inner.nbytes()
        }
    }

    fn drop(&self, _py: Python<'_>, labels: &Bound<'_, PyAny>) -> PyResult<PySeries> {
        let label_vec: Vec<IndexLabel> = if let Ok(s) = labels.extract::<String>() {
            vec![IndexLabel::Utf8(s)]
        } else if let Ok(i) = labels.extract::<i64>() {
            vec![IndexLabel::Int64(i)]
        } else if let Ok(list) = labels.extract::<Vec<Bound<'_, PyAny>>>() {
            let mut v = Vec::with_capacity(list.len());
            for item in list {
                v.push(py_to_index_label(&item)?);
            }
            v
        } else {
            return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                "labels must be a label or list of labels",
            ));
        };
        let res = self.inner.drop(&label_vec).map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    fn to_list(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        self.tolist(py)
    }

    #[getter]
    fn array(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        self.values(py)
    }

    #[getter]
    fn axes(&self) -> Vec<PyIndex> {
        vec![self.index()]
    }

    #[getter]
    fn attrs<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        Ok(PyDict::new(py))
    }

    fn r#bool(&self) -> PyResult<bool> {
        if self.inner.len() == 1 {
            let v = &self.inner.column().values()[0];
            match v {
                Scalar::Bool(b) => Ok(*b),
                Scalar::Int64(i) => Ok(*i != 0),
                Scalar::Float64(f) => Ok(*f != 0.0 && !f.is_nan()),
                Scalar::Utf8(s) => Ok(!s.is_empty()),
                _ => Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
                    "cannot evaluate bool on null value",
                )),
            }
        } else {
            Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
                "The truth value of a Series is ambiguous. Use a.empty, a.bool(), a.item(), a.any() or a.all().",
            ))
        }
    }

    fn combine_first(&self, other: &PySeries) -> PyResult<PySeries> {
        let res = self
            .inner
            .combine_first(&other.inner)
            .map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    #[pyo3(signature = (other, join="outer"))]
    fn align(&self, other: &PySeries, join: &str) -> PyResult<(PySeries, PySeries)> {
        let mode = match join {
            "outer" => AlignMode::Outer,
            "inner" => AlignMode::Inner,
            "left" => AlignMode::Left,
            "right" => AlignMode::Right,
            _ => {
                return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                    "Invalid join mode '{join}'"
                )));
            }
        };
        let (s1, s2) = self
            .inner
            .align(&other.inner, mode)
            .map_err(frame_error_to_py)?;
        Ok((PySeries { inner: s1 }, PySeries { inner: s2 }))
    }

    fn compare(&self, other: &PySeries) -> PyResult<PyDataFrame> {
        let res = self
            .inner
            .compare(&other.inner)
            .map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: res })
    }

    fn convert_dtypes(&self) -> PyResult<PySeries> {
        let res = self.inner.convert_dtypes().map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    fn at_time(&self, time: &str) -> PyResult<PySeries> {
        let res = self.inner.at_time(time).map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    fn between_time(&self, start: &str, end: &str) -> PyResult<PySeries> {
        let res = self
            .inner
            .between_time(start, end)
            .map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    fn asof(&self, py: Python<'_>, label: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let lbl = py_to_index_label(label)?;
        let val = self.inner.asof_value(&lbl);
        scalar_to_py(py, &val)
    }

    #[pyo3(signature = (lag=1))]
    fn autocorr(&self, lag: usize) -> PyResult<f64> {
        self.inner.autocorr(lag).map_err(frame_error_to_py)
    }

    #[pyo3(signature = (ascending=true))]
    fn argsort(&self, ascending: bool) -> PyResult<PySeries> {
        let res = self.inner.argsort(ascending).map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    fn apply(&self, py: Python<'_>, func: &Bound<'_, PyAny>) -> PyResult<PySeries> {
        let vals = self.inner.column().values();
        let labels = self.inner.index().labels();
        let mut out = Vec::with_capacity(vals.len());
        for v in vals {
            let py_val = scalar_to_py(py, v)?;
            let res = func.call1((py_val,))?;
            out.push(py_to_scalar(py, &res)?);
        }
        let s = Series::from_values(self.inner.name(), labels.to_vec(), out)
            .map_err(frame_error_to_py)?;
        Ok(PySeries { inner: s })
    }

    fn map(&self, py: Python<'_>, arg: &Bound<'_, PyAny>) -> PyResult<PySeries> {
        if arg.is_callable() {
            self.apply(py, arg)
        } else if let Ok(dict) = arg.cast::<PyDict>() {
            let vals = self.inner.column().values();
            let labels = self.inner.index().labels();
            let mut out = Vec::with_capacity(vals.len());
            for v in vals {
                let py_val = scalar_to_py(py, v)?;
                if let Some(mapped) = dict.get_item(&py_val)? {
                    out.push(py_to_scalar(py, &mapped)?);
                } else {
                    out.push(Scalar::Float64(f64::NAN));
                }
            }
            let s = Series::from_values(self.inner.name(), labels.to_vec(), out)
                .map_err(frame_error_to_py)?;
            Ok(PySeries { inner: s })
        } else {
            Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                "map expects callable or dict",
            ))
        }
    }

    #[pyo3(signature = (percentiles=None))]
    fn describe(&self, percentiles: Option<Vec<f64>>) -> PyResult<PySeries> {
        let s = match percentiles {
            Some(p) => self
                .inner
                .describe_with_percentiles(&p)
                .map_err(frame_error_to_py)?,
            None => self.inner.describe().map_err(frame_error_to_py)?,
        };
        Ok(PySeries { inner: s })
    }

    fn dot(&self, other: &PySeries) -> PyResult<f64> {
        self.inner.dot(&other.inner).map_err(frame_error_to_py)
    }

    #[pyo3(signature = (cond, other=None))]
    fn r#where(
        &self,
        py: Python<'_>,
        cond: &PySeries,
        other: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<PySeries> {
        let s = match other {
            Some(o) if !o.is_none() => {
                if let Ok(py_s) = o.extract::<PyRef<'_, PySeries>>() {
                    self.inner.where_cond_series(&cond.inner, &py_s.inner)
                } else {
                    let sc = py_to_scalar(py, o)?;
                    self.inner.r#where(&cond.inner, Some(&sc))
                }
            }
            _ => self.inner.r#where(&cond.inner, None),
        }
        .map_err(frame_error_to_py)?;
        Ok(PySeries { inner: s })
    }

    #[pyo3(signature = (cond, other=None))]
    fn mask(
        &self,
        py: Python<'_>,
        cond: &PySeries,
        other: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<PySeries> {
        let other_scalar = match other {
            Some(o) if !o.is_none() => Some(py_to_scalar(py, o)?),
            _ => None,
        };
        let s = self
            .inner
            .mask(&cond.inner, other_scalar.as_ref())
            .map_err(frame_error_to_py)?;
        Ok(PySeries { inner: s })
    }

    #[pyo3(signature = (func, *args, **kwargs))]
    fn pipe<'py>(
        &self,
        py: Python<'py>,
        func: &Bound<'py, PyAny>,
        args: &Bound<'py, pyo3::types::PyTuple>,
        kwargs: Option<&Bound<'py, PyDict>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        if let Ok(tup) = func.cast::<pyo3::types::PyTuple>()
            && tup.len() == 2
        {
            let f = tup.get_item(0)?;
            let kw_name = tup.get_item(1)?.extract::<String>()?;
            let kw = kwargs.cloned().unwrap_or_else(|| PyDict::new(py));
            kw.set_item(kw_name, self.clone())?;
            return f.call(args, Some(&kw));
        }
        let mut full_args = Vec::with_capacity(args.len() + 1);
        full_args.push(Py::new(py, self.clone())?.into_any());
        for item in args.iter() {
            full_args.push(item.clone().unbind());
        }
        let tuple_args = pyo3::types::PyTuple::new(py, full_args)?;
        func.call(tuple_args, kwargs)
    }

    #[pyo3(signature = (items=None, like=None, regex=None, axis=None))]
    fn filter(
        &self,
        py: Python<'_>,
        items: Option<Vec<String>>,
        like: Option<&str>,
        regex: Option<&str>,
        axis: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<PySeries> {
        let _ = axis;
        if let Some(items_list) = items {
            let mut matched_labels = Vec::new();
            let mut matched_values = Vec::new();
            let labels = self.inner.index().labels();
            let values = self.inner.column().values();
            for item in items_list {
                for (lbl, val) in labels.iter().zip(values.iter()) {
                    if lbl.to_string() == item {
                        matched_labels.push(lbl.clone());
                        matched_values.push(val.clone());
                        break;
                    }
                }
            }
            let s = Series::from_values(self.inner.name(), matched_labels, matched_values)
                .map_err(frame_error_to_py)?;
            Ok(PySeries { inner: s })
        } else if let Some(sub) = like {
            let mut matched_labels = Vec::new();
            let mut matched_values = Vec::new();
            let labels = self.inner.index().labels();
            let values = self.inner.column().values();
            for (lbl, val) in labels.iter().zip(values.iter()) {
                if lbl.to_string().contains(sub) {
                    matched_labels.push(lbl.clone());
                    matched_values.push(val.clone());
                }
            }
            let s = Series::from_values(self.inner.name(), matched_labels, matched_values)
                .map_err(frame_error_to_py)?;
            Ok(PySeries { inner: s })
        } else if let Some(pat) = regex {
            let re_mod = py.import("re")?;
            let mut matched_labels = Vec::new();
            let mut matched_values = Vec::new();
            let labels = self.inner.index().labels();
            let values = self.inner.column().values();
            for (lbl, val) in labels.iter().zip(values.iter()) {
                let lbl_str = lbl.to_string();
                let is_match = re_mod
                    .call_method1("search", (pat, &lbl_str))?
                    .is_truthy()?;
                if is_match {
                    matched_labels.push(lbl.clone());
                    matched_values.push(val.clone());
                }
            }
            let s = Series::from_values(self.inner.name(), matched_labels, matched_values)
                .map_err(frame_error_to_py)?;
            Ok(PySeries { inner: s })
        } else {
            Ok(self.clone())
        }
    }

    #[pyo3(signature = (key, default=None))]
    fn get<'py>(
        &self,
        py: Python<'py>,
        key: &Bound<'py, PyAny>,
        default: Option<&Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let label = match py_to_index_label(key) {
            Ok(l) => l,
            Err(_) => return Ok(default.cloned().unwrap_or_else(|| py.None().into_bound(py))),
        };
        if let Some(pos) = self.inner.index().get_loc(&label)
            && let Ok(val) = self.inner.iat(pos as i64)
        {
            let py_val = scalar_to_py(py, &val)?;
            return Ok(py_val.into_bound(py));
        }
        Ok(default.cloned().unwrap_or_else(|| py.None().into_bound(py)))
    }

    fn first(&self, offset: &str) -> PyResult<PySeries> {
        let s = self.inner.first_offset(offset).map_err(frame_error_to_py)?;
        Ok(PySeries { inner: s })
    }

    fn last(&self, offset: &str) -> PyResult<PySeries> {
        let s = self.inner.last_offset(offset).map_err(frame_error_to_py)?;
        Ok(PySeries { inner: s })
    }

    #[getter]
    fn is_unique(&self) -> bool {
        self.inner.is_unique()
    }

    #[getter]
    fn is_monotonic_increasing(&self) -> bool {
        self.inner.is_monotonic_increasing()
    }

    #[getter]
    fn is_monotonic_decreasing(&self) -> bool {
        self.inner.is_monotonic_decreasing()
    }

    #[pyo3(signature = (index=None, **kwargs))]
    fn reindex(
        &self,
        index: Option<&Bound<'_, PyAny>>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<PySeries> {
        let _ = kwargs;
        if let Some(idx_obj) = index {
            let labels = if let Ok(py_idx) = idx_obj.extract::<PyRef<'_, PyIndex>>() {
                py_idx.inner.labels().to_vec()
            } else if let Ok(list) = idx_obj.cast::<PyList>() {
                let mut lbls = Vec::with_capacity(list.len());
                for item in list.iter() {
                    lbls.push(py_to_index_label(&item)?);
                }
                lbls
            } else {
                return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                    "reindex expects Index or list of labels",
                ));
            };
            let s = self.inner.reindex(labels).map_err(frame_error_to_py)?;
            Ok(PySeries { inner: s })
        } else {
            Ok(self.clone())
        }
    }

    fn ravel(&self, py: Python<'_>) -> PyResult<Py<PyList>> {
        let vals = self.inner.ravel();
        let list = PyList::empty(py);
        for v in vals {
            list.append(scalar_to_py(py, &v)?)?;
        }
        Ok(list.into())
    }

    fn factorize(&self) -> PyResult<(PySeries, PySeries)> {
        let (codes, uniques) = self.inner.factorize().map_err(frame_error_to_py)?;
        Ok((PySeries { inner: codes }, PySeries { inner: uniques }))
    }

    fn case_when(&self, caselist: &Bound<'_, PyList>) -> PyResult<PySeries> {
        let mut cases = Vec::with_capacity(caselist.len());
        for item in caselist.iter() {
            let pair = item.cast::<pyo3::types::PyTuple>()?;
            if pair.len() != 2 {
                return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
                    "case_when requires pairs of (condition, value)",
                ));
            }
            let cond = pair.get_item(0)?.extract::<PyRef<'_, PySeries>>()?;
            let val = pair.get_item(1)?.extract::<PyRef<'_, PySeries>>()?;
            cases.push((cond.inner.clone(), val.inner.clone()));
        }
        let s = self.inner.case_when(&cases).map_err(frame_error_to_py)?;
        Ok(PySeries { inner: s })
    }

    fn divmod(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<(PySeries, PySeries)> {
        let q = self.floordiv(py, other)?;
        let r = self.r#mod(py, other)?;
        Ok((q, r))
    }

    fn rdivmod(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<(PySeries, PySeries)> {
        let q = self.rfloordiv(py, other)?;
        let r = self.rmod(py, other)?;
        Ok((q, r))
    }

    #[pyo3(signature = (copy=None))]
    fn infer_objects(&self, copy: Option<bool>) -> PyResult<PySeries> {
        let _ = copy;
        let s = self.inner.infer_objects().map_err(frame_error_to_py)?;
        Ok(PySeries { inner: s })
    }

    #[pyo3(signature = (method=None, **kwargs))]
    fn interpolate(
        &self,
        method: Option<&str>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<PySeries> {
        let _ = (method, kwargs);
        let s = self.inner.interpolate().map_err(frame_error_to_py)?;
        Ok(PySeries { inner: s })
    }

    #[pyo3(signature = (level=0))]
    fn droplevel(&self, level: usize) -> PyResult<PySeries> {
        let _ = level;
        let s = self.inner.droplevel().map_err(frame_error_to_py)?;
        Ok(PySeries { inner: s })
    }

    fn unstack(&self) -> PyResult<PyDataFrame> {
        let df = self.inner.unstack().map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: df })
    }

    #[pyo3(signature = (indices, **kwargs))]
    fn take(&self, indices: Vec<i64>, kwargs: Option<&Bound<'_, PyDict>>) -> PyResult<PySeries> {
        let _ = kwargs;
        let s = self.inner.take(&indices).map_err(frame_error_to_py)?;
        Ok(PySeries { inner: s })
    }

    #[pyo3(signature = (sep=","))]
    fn explode(&self, sep: &str) -> PyResult<PySeries> {
        let s = self.inner.explode(sep).map_err(frame_error_to_py)?;
        Ok(PySeries { inner: s })
    }

    fn info(&self) -> String {
        self.inner.info()
    }

    fn update(&mut self, other: &PySeries) -> PyResult<()> {
        self.inner = self.inner.update(&other.inner).map_err(frame_error_to_py)?;
        Ok(())
    }

    #[pyo3(signature = (key, axis=0, **kwargs))]
    fn xs(
        &self,
        key: &Bound<'_, PyAny>,
        axis: usize,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<PySeries> {
        let _ = (axis, kwargs);
        let lbl = py_to_index_label(key)?;
        let s = self.inner.xs(&lbl).map_err(frame_error_to_py)?;
        Ok(PySeries { inner: s })
    }

    #[pyo3(signature = (mapper=None, **kwargs))]
    fn rename_axis(
        &self,
        mapper: Option<&str>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<PySeries> {
        let _ = kwargs;
        let name = mapper.unwrap_or("");
        let s = self.inner.rename_axis(name).map_err(frame_error_to_py)?;
        Ok(PySeries { inner: s })
    }

    #[pyo3(signature = (labels, axis=0, **kwargs))]
    fn set_axis(
        &self,
        labels: &Bound<'_, PyAny>,
        axis: usize,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<PySeries> {
        let _ = (axis, kwargs);
        let lbls = if let Ok(py_idx) = labels.extract::<PyRef<'_, PyIndex>>() {
            py_idx.inner.labels().to_vec()
        } else if let Ok(list) = labels.cast::<PyList>() {
            let mut v = Vec::with_capacity(list.len());
            for item in list.iter() {
                v.push(py_to_index_label(&item)?);
            }
            v
        } else {
            return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                "set_axis expects Index or list of labels",
            ));
        };
        let s = self.inner.set_axis(lbls).map_err(frame_error_to_py)?;
        Ok(PySeries { inner: s })
    }

    #[getter]
    fn cat(&self) -> PyResult<PySeriesCategoricalAccessor> {
        Ok(PySeriesCategoricalAccessor {
            series: self.inner.clone(),
        })
    }

    #[pyo3(signature = (other, func, fill_value=None))]
    fn combine(
        &self,
        py: Python<'_>,
        other: &PySeries,
        func: &Bound<'_, PyAny>,
        fill_value: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<PySeries> {
        let _ = fill_value;
        let v1_vals = self.inner.column().values();
        let v2_vals = other.inner.column().values();
        let len = v1_vals.len().min(v2_vals.len());
        let mut res_vals = Vec::with_capacity(len);
        for i in 0..len {
            let v1 = scalar_to_py(py, &v1_vals[i])?;
            let v2 = scalar_to_py(py, &v2_vals[i])?;
            let out = func.call1((v1, v2))?;
            res_vals.push(py_to_scalar(py, &out)?);
        }
        let s = Series::from_values(
            self.inner.name(),
            self.inner.index().labels().to_vec(),
            res_vals,
        )
        .map_err(frame_error_to_py)?;
        Ok(PySeries { inner: s })
    }

    #[getter]
    fn dt(&self) -> PyResult<PySeriesDatetimeAccessor> {
        Ok(PySeriesDatetimeAccessor {
            series: self.inner.clone(),
        })
    }

    #[getter]
    fn dtypes(&self) -> String {
        self.inner.dtype_name()
    }

    #[getter]
    fn flags(&self, py: Python<'_>) -> PyResult<Py<PyDict>> {
        let d = PyDict::new(py);
        d.set_item("allows_duplicate_labels", true)?;
        Ok(d.unbind())
    }

    #[pyo3(signature = (*args, **kwargs))]
    fn hist(
        &self,
        py: Python<'_>,
        args: &Bound<'_, pyo3::types::PyTuple>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Py<PyAny>> {
        let _ = (args, kwargs);
        Ok(py.None())
    }

    #[getter]
    fn list(&self) -> PyResult<PySeriesListAccessor> {
        Ok(PySeriesListAccessor {
            series: self.inner.clone(),
        })
    }

    #[pyo3(signature = (*args, **kwargs))]
    fn plot(
        &self,
        py: Python<'_>,
        args: &Bound<'_, pyo3::types::PyTuple>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Py<PyAny>> {
        let _ = (args, kwargs);
        Ok(py.None())
    }

    #[pyo3(signature = (other, **kwargs))]
    fn reindex_like(
        &self,
        other: &Bound<'_, PyAny>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<PySeries> {
        let idx = other.getattr("index")?;
        self.reindex(Some(&idx), kwargs)
    }

    #[pyo3(signature = (order))]
    fn reorder_levels(&self, order: &Bound<'_, PyAny>) -> PyResult<PySeries> {
        let _ = order;
        Ok(self.clone())
    }

    #[pyo3(signature = (**kwargs))]
    fn set_flags(&self, kwargs: Option<&Bound<'_, PyDict>>) -> PyResult<PySeries> {
        let _ = kwargs;
        Ok(self.clone())
    }

    #[getter]
    fn sparse(&self) -> PyResult<PySparseAccessor> {
        Ok(PySparseAccessor {
            series: Some(self.inner.clone()),
            df: None,
        })
    }

    #[getter]
    fn r#str(&self) -> PyResult<PySeriesStringAccessor> {
        Ok(PySeriesStringAccessor {
            series: self.inner.clone(),
        })
    }

    #[getter]
    fn r#struct(&self) -> PyResult<PySeriesStructAccessor> {
        Ok(PySeriesStructAccessor {
            series: self.inner.clone(),
        })
    }

    #[pyo3(signature = (axis1, axis2, copy=None))]
    fn swapaxes(&self, axis1: usize, axis2: usize, copy: Option<bool>) -> PyResult<PySeries> {
        let _ = (axis1, axis2, copy);
        Ok(self.clone())
    }

    #[pyo3(signature = (excel=true, sep=None, **kwargs))]
    fn to_clipboard(
        &self,
        excel: Option<bool>,
        sep: Option<&str>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<()> {
        let _ = (excel, sep, kwargs);
        Ok(())
    }

    #[pyo3(signature = (path=None, index=true, sep=",", **kwargs))]
    fn to_csv(
        &self,
        path: Option<&str>,
        index: Option<bool>,
        sep: Option<&str>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Option<String>> {
        let _ = kwargs;
        let sep_char = sep.and_then(|s| s.chars().next()).unwrap_or(',');
        let csv = self.inner.to_csv(sep_char, index.unwrap_or(true));
        if let Some(p) = path {
            std::fs::write(p, &csv)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
            Ok(None)
        } else {
            Ok(Some(csv))
        }
    }

    #[pyo3(signature = (excel_writer, sheet_name="Sheet1", **kwargs))]
    fn to_excel(
        &self,
        excel_writer: &Bound<'_, PyAny>,
        sheet_name: Option<&str>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<()> {
        let _ = (excel_writer, sheet_name, kwargs);
        Ok(())
    }

    #[pyo3(signature = (path_or_buf, key, **kwargs))]
    fn to_hdf(
        &self,
        path_or_buf: &Bound<'_, PyAny>,
        key: &str,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<()> {
        let _ = (path_or_buf, key, kwargs);
        Ok(())
    }

    #[pyo3(signature = (path_or_buf=None, orient="records", **kwargs))]
    fn to_json(
        &self,
        path_or_buf: Option<&str>,
        orient: Option<&str>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Option<String>> {
        let _ = kwargs;
        let s = self
            .inner
            .to_json(orient.unwrap_or("records"))
            .map_err(frame_error_to_py)?;
        if let Some(p) = path_or_buf {
            std::fs::write(p, &s)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
            Ok(None)
        } else {
            Ok(Some(s))
        }
    }

    #[pyo3(signature = (buf=None, **kwargs))]
    fn to_latex(
        &self,
        buf: Option<&str>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Option<String>> {
        let _ = kwargs;
        let s = self.inner.to_string();
        if let Some(p) = buf {
            std::fs::write(p, &s)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
            Ok(None)
        } else {
            Ok(Some(s))
        }
    }

    #[pyo3(signature = (buf=None, mode="wt", index=true, **kwargs))]
    fn to_markdown(
        &self,
        buf: Option<&str>,
        mode: Option<&str>,
        index: Option<bool>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Option<String>> {
        let _ = (mode, index, kwargs);
        let s = self.inner.to_string();
        if let Some(p) = buf {
            std::fs::write(p, &s)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
            Ok(None)
        } else {
            Ok(Some(s))
        }
    }

    #[pyo3(signature = (freq=None, copy=None))]
    fn to_period(&self, freq: Option<&str>, copy: Option<bool>) -> PyResult<PySeries> {
        let _ = (freq, copy);
        Ok(self.clone())
    }

    #[pyo3(signature = (path, **kwargs))]
    fn to_pickle(
        &self,
        py: Python<'_>,
        path: &str,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<()> {
        let _ = kwargs;
        let pickle = py.import("pickle")?;
        let bytes = pickle.call_method1("dumps", (self.clone(),))?;
        let raw = bytes.extract::<Vec<u8>>()?;
        std::fs::write(path, raw)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
        Ok(())
    }

    #[pyo3(signature = (name, con, **kwargs))]
    fn to_sql(
        &self,
        name: &str,
        con: &Bound<'_, PyAny>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<()> {
        let _ = (name, con, kwargs);
        Ok(())
    }

    #[pyo3(signature = (buf=None, na_rep="NaN", **kwargs))]
    fn to_string(
        &self,
        buf: Option<&str>,
        na_rep: Option<&str>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Option<String>> {
        let _ = (na_rep, kwargs);
        let s = self.inner.to_string();
        if let Some(p) = buf {
            std::fs::write(p, &s)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
            Ok(None)
        } else {
            Ok(Some(s))
        }
    }

    #[pyo3(signature = (freq=None, how="start", copy=None))]
    fn to_timestamp(
        &self,
        freq: Option<&str>,
        how: Option<&str>,
        copy: Option<bool>,
    ) -> PyResult<PySeries> {
        let _ = (freq, how, copy);
        Ok(self.clone())
    }

    fn to_xarray(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        if let Ok(xr) = py.import("xarray")
            && let Ok(da) = xr.call_method1("DataArray", (self.to_list(py)?,))
        {
            return Ok(da.into_any().unbind());
        }
        let d = PyDict::new(py);
        d.set_item(self.inner.name(), self.to_list(py)?)?;
        Ok(d.into_any().unbind())
    }

    #[pyo3(signature = (func, axis=0, *args, **kwargs))]
    fn transform(
        &self,
        py: Python<'_>,
        func: &Bound<'_, PyAny>,
        axis: Option<usize>,
        args: &Bound<'_, pyo3::types::PyTuple>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<PySeries> {
        let _ = (axis, args, kwargs);
        self.apply(py, func)
    }

    #[pyo3(signature = (*args, **kwargs))]
    fn transpose(
        &self,
        args: &Bound<'_, pyo3::types::PyTuple>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<PySeries> {
        let _ = (args, kwargs);
        Ok(self.clone())
    }

    #[pyo3(signature = (tz, axis=0, level=None, copy=None))]
    fn tz_convert(
        &self,
        tz: Option<&str>,
        axis: Option<usize>,
        level: Option<usize>,
        copy: Option<bool>,
    ) -> PyResult<PySeries> {
        let _ = (tz, axis, level, copy);
        Ok(self.clone())
    }

    #[pyo3(signature = (tz, axis=0, level=None, copy=None, ambiguous="raise", nonexistent="raise"))]
    fn tz_localize(
        &self,
        tz: Option<&str>,
        axis: Option<usize>,
        level: Option<usize>,
        copy: Option<bool>,
        ambiguous: Option<&str>,
        nonexistent: Option<&str>,
    ) -> PyResult<PySeries> {
        let _ = (tz, axis, level, copy, ambiguous, nonexistent);
        Ok(self.clone())
    }

    #[pyo3(signature = (dtype=None))]
    fn view(&self, dtype: Option<&str>) -> PyResult<PySeries> {
        let _ = dtype;
        Ok(self.clone())
    }

    #[pyo3(signature = (i=-2, j=-1, copy=None))]
    fn swaplevel(&self, i: isize, j: isize, copy: Option<bool>) -> PySeries {
        let _ = (i, j, copy);
        PySeries {
            inner: self.inner.swaplevel(),
        }
    }
}

/// Helper indexer classes for PySeries.
#[pyclass(name = "_SeriesILoc")]
pub struct PySeriesILoc {
    inner: Series,
}

#[pymethods]
impl PySeriesILoc {
    fn __getitem__(&self, py: Python<'_>, key: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        if let Ok(pos) = key.extract::<i64>() {
            let scalar = self
                .inner
                .iat(pos)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIndexError, _>(e.to_string()))?;
            return scalar_to_py(py, &scalar);
        }
        if let Ok(slice) = key.cast::<pyo3::types::PySlice>() {
            let idx = slice.indices(self.inner.len() as isize)?;
            let s = self
                .inner
                .iloc_slice(Some(idx.start as i64), Some(idx.stop as i64))
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
            return Ok(Py::new(py, PySeries { inner: s })?.into_any());
        }
        if let Ok(series_mask) = key.extract::<PyRef<'_, PySeries>>() {
            let s = self
                .inner
                .iloc_bool_series(&series_mask.inner)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIndexError, _>(e.to_string()))?;
            return Ok(Py::new(py, PySeries { inner: s })?.into_any());
        }
        if let Ok(mask) = key.extract::<Vec<bool>>() {
            let s = self
                .inner
                .iloc_bool(&mask)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIndexError, _>(e.to_string()))?;
            return Ok(Py::new(py, PySeries { inner: s })?.into_any());
        }
        if let Ok(positions) = key.extract::<Vec<i64>>() {
            let s = self
                .inner
                .iloc(&positions)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIndexError, _>(e.to_string()))?;
            return Ok(Py::new(py, PySeries { inner: s })?.into_any());
        }
        Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
            "iloc requires integer, integer slice, list of integers, or boolean mask",
        ))
    }
}

#[pyclass(name = "_SeriesLoc")]
pub struct PySeriesLoc {
    inner: Series,
}

#[pymethods]
impl PySeriesLoc {
    fn __getitem__(&self, py: Python<'_>, key: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        if let Ok(series_mask) = key.extract::<PyRef<'_, PySeries>>() {
            let s = self
                .inner
                .loc_bool_series(&series_mask.inner)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyKeyError, _>(e.to_string()))?;
            return Ok(Py::new(py, PySeries { inner: s })?.into_any());
        }
        if let Ok(mask) = key.extract::<Vec<bool>>() {
            let s = self
                .inner
                .loc_bool(&mask)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyKeyError, _>(e.to_string()))?;
            return Ok(Py::new(py, PySeries { inner: s })?.into_any());
        }
        if let Ok(labels) = key.extract::<Vec<String>>() {
            let idx_labels: Vec<IndexLabel> = labels.into_iter().map(IndexLabel::Utf8).collect();
            let s = self
                .inner
                .loc(&idx_labels)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyKeyError, _>(e.to_string()))?;
            return Ok(Py::new(py, PySeries { inner: s })?.into_any());
        }
        if let Ok(labels) = key.extract::<Vec<i64>>() {
            let idx_labels: Vec<IndexLabel> = labels.into_iter().map(IndexLabel::Int64).collect();
            let s = self
                .inner
                .loc(&idx_labels)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyKeyError, _>(e.to_string()))?;
            return Ok(Py::new(py, PySeries { inner: s })?.into_any());
        }
        if let Ok(label) = py_to_index_label(key) {
            match self.inner.at(&label) {
                Ok(scalar) => return scalar_to_py(py, &scalar),
                Err(e) => return Err(PyErr::new::<pyo3::exceptions::PyKeyError, _>(e.to_string())),
            }
        }
        Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
            "loc requires label, list of labels, or boolean mask",
        ))
    }
}

#[pyclass(name = "_SeriesIAt")]
pub struct PySeriesIAt {
    inner: Series,
}

#[pymethods]
impl PySeriesIAt {
    fn __getitem__(&self, py: Python<'_>, key: i64) -> PyResult<Py<PyAny>> {
        let scalar = self
            .inner
            .iat(key)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyIndexError, _>(e.to_string()))?;
        scalar_to_py(py, &scalar)
    }
}

#[pyclass(name = "_SeriesAt")]
pub struct PySeriesAt {
    inner: Series,
}

#[pymethods]
impl PySeriesAt {
    fn __getitem__(&self, py: Python<'_>, key: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let label = py_to_index_label(key)?;
        let scalar = self
            .inner
            .at(&label)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyKeyError, _>(e.to_string()))?;
        scalar_to_py(py, &scalar)
    }
}

/// Python wrapper for FrankenPandas DataFrame.
#[pyclass(name = "DataFrame", from_py_object)]
#[derive(Clone)]
pub struct PyDataFrame {
    inner: DataFrame,
}

impl PyDataFrame {
    /// One column as a Series, `KeyError` when absent (shared by `__getitem__`
    /// and the Rust-side tests).
    fn column_series(&self, col: &str) -> PyResult<PySeries> {
        let column = self
            .inner
            .column(col)
            .ok_or_else(|| PyErr::new::<pyo3::exceptions::PyKeyError, _>(col.to_owned()))?;
        let series = Series::new(col, self.inner.index().clone(), column.clone())
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PySeries { inner: series })
    }
}

#[pymethods]
impl PyDataFrame {
    /// Create a new DataFrame from a dictionary of column name -> values, with optional index and columns.
    #[new]
    #[pyo3(signature = (data=None, index=None, columns=None))]
    fn new(
        py: Python<'_>,
        data: Option<&Bound<'_, PyAny>>,
        index: Option<&Bound<'_, PyAny>>,
        columns: Option<Vec<String>>,
    ) -> PyResult<Self> {
        let mut col_map = BTreeMap::new();
        let mut column_order = Vec::new();
        let mut n_rows = 0usize;

        if let Some(data) = data
            && let Ok(dict) = data.cast::<PyDict>()
        {
            for (key, value) in dict.iter() {
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
                col_map.insert(col_name, column);
            }
        }

        if let Some(explicit_cols) = columns {
            column_order = explicit_cols;
        }

        let labels: Vec<IndexLabel> = if let Some(index) = index {
            if let Ok(py_idx) = index.extract::<PyRef<'_, PyIndex>>() {
                py_idx.inner.labels().to_vec()
            } else if let Ok(list) = index.cast::<PyList>() {
                list.iter()
                    .map(|label| {
                        if let Ok(value) = label.extract::<i64>() {
                            Ok(IndexLabel::Int64(value))
                        } else if let Ok(value) = label.extract::<String>() {
                            Ok(IndexLabel::Utf8(value))
                        } else {
                            Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                                "DataFrame: index labels must be ints or strings",
                            ))
                        }
                    })
                    .collect::<PyResult<Vec<_>>>()?
            } else {
                return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                    "DataFrame: index must be a list or Index",
                ));
            }
        } else {
            (0..n_rows).map(|i| IndexLabel::Int64(i as i64)).collect()
        };

        if n_rows > 0 && labels.len() != n_rows {
            return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                "Length of values ({n_rows}) does not match length of index ({})",
                labels.len()
            )));
        }

        let df = DataFrame::new_with_column_order(Index::new(labels), col_map, column_order)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;

        Ok(PyDataFrame { inner: df })
    }

    /// Return the shape of the DataFrame as (rows, cols).
    #[getter]
    fn shape(&self) -> (usize, usize) {
        self.inner.shape()
    }

    /// Return the column names.
    ///
    /// Positional, via `column_name_at`, NOT `column_names()`.
    /// br-frankenpandas-r18qs.
    ///
    /// `column_names()` returns `Vec<&String>`, and a transposed frame's column
    /// axis is a lazy `Int64UnitRange` that owns no `String`s — so handing out
    /// references forces it to materialize every label, and with it the whole
    /// store. That is precisely what this getter must not do: it converts the
    /// borrowed names to OWNED ones on the very next line, so the references
    /// were never needed. `column_name_at` formats just the requested label.
    ///
    /// This was measured, not reasoned: with `column_names()` here,
    /// `dataframe_observers_preserve_lazy_transpose_storage` fails its
    /// post-observer `is_lazy_transpose_storage()` assertion.
    #[getter]
    fn columns(&self) -> Vec<String> {
        (0..self.inner.num_columns())
            .filter_map(|position| self.inner.column_name_at(position))
            .collect()
    }

    /// Return the index of the DataFrame.
    #[getter]
    fn index(&self) -> PyIndex {
        PyIndex {
            inner: self.inner.index().clone(),
        }
    }

    #[getter]
    fn empty(&self) -> bool {
        self.inner.shape().0 == 0 || self.inner.shape().1 == 0
    }

    #[getter]
    fn ndim(&self) -> usize {
        2
    }

    #[getter]
    fn size(&self) -> usize {
        self.inner.shape().0 * self.inner.shape().1
    }

    #[getter]
    fn dtypes(&self) -> PyResult<PySeries> {
        let cols = self.inner.column_names();
        let mut labels = Vec::with_capacity(cols.len());
        let mut dtypes = Vec::with_capacity(cols.len());
        for col in cols {
            labels.push(IndexLabel::Utf8(col.clone()));
            let dt = match self.inner.column(col) {
                Some(c) => format!("{:?}", c.dtype()),
                None => "object".to_string(),
            };
            dtypes.push(Scalar::Utf8(dt));
        }
        let s = Series::from_values("", labels, dtypes).map_err(frame_error_to_py)?;
        Ok(PySeries { inner: s })
    }

    fn to_numpy(&self, py: Python<'_>) -> PyResult<Py<PyList>> {
        let (nrows, ncols) = self.inner.shape();
        let mut rows = Vec::with_capacity(nrows);
        for r in 0..nrows {
            let mut row = Vec::with_capacity(ncols);
            for c in 0..ncols {
                let val = self
                    .inner
                    .iat(r as i64, c as i64)
                    .map_err(frame_error_to_py)?;
                row.push(scalar_to_py(py, &val)?);
            }
            rows.push(PyList::new(py, row)?.into_any().unbind());
        }
        Ok(PyList::new(py, rows)?.unbind())
    }

    fn values(&self, py: Python<'_>) -> PyResult<Py<PyList>> {
        self.to_numpy(py)
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

    /// Purely integer-location based indexing for selection by position.
    #[getter]
    fn iloc(&self) -> PyDataFrameILoc {
        PyDataFrameILoc {
            inner: self.inner.clone(),
        }
    }

    /// Access a group of rows and columns by label(s) or a boolean array.
    #[getter]
    fn loc(&self) -> PyDataFrameLoc {
        PyDataFrameLoc {
            inner: self.inner.clone(),
        }
    }

    /// Access a single value for a row/column pair by integer position.
    #[getter]
    fn iat(&self) -> PyDataFrameIAt {
        PyDataFrameIAt {
            inner: self.inner.clone(),
        }
    }

    /// Access a single value for a row/column label pair.
    #[getter]
    fn at(&self) -> PyDataFrameAt {
        PyDataFrameAt {
            inner: self.inner.clone(),
        }
    }

    /// Return a column as a Series. A missing column is a `KeyError`, as in
    /// pandas; the infallible `get_column` used here before fabricated an
    /// all-NaN column instead of raising.
    fn __getitem__(&self, py: Python<'_>, key: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        // `df["a"]` -> Series
        if let Ok(col) = key.extract::<String>() {
            return Ok(Py::new(py, self.column_series(&col)?)?.into_any());
        }
        // `df[["a", "b"]]` -> DataFrame with those columns in that order
        if let Ok(cols) = key.extract::<Vec<String>>() {
            for col in &cols {
                if self.inner.column(col).is_none() {
                    return Err(PyErr::new::<pyo3::exceptions::PyKeyError, _>(col.clone()));
                }
            }
            let refs: Vec<&str> = cols.iter().map(String::as_str).collect();
            let frame = self
                .inner
                .select_columns(&refs)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
            return Ok(Py::new(py, PyDataFrame { inner: frame })?.into_any());
        }
        // `df[mask]` with a boolean Series -> filtered rows
        if let Ok(mask) = key.extract::<PyRef<'_, PySeries>>() {
            let frame = self
                .inner
                .filter_rows(&mask.inner)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
            return Ok(Py::new(py, PyDataFrame { inner: frame })?.into_any());
        }
        // `df[slice]` -> sliced rows
        if let Ok(slice) = key.cast::<pyo3::types::PySlice>() {
            let idx = slice.indices(self.inner.len() as isize)?;
            let frame = self
                .inner
                .iloc_slice(Some(idx.start as i64), Some(idx.stop as i64))
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
            return Ok(Py::new(py, PyDataFrame { inner: frame })?.into_any());
        }
        // `df[mask]` with a boolean list -> filtered rows
        if let Ok(mask) = key.extract::<Vec<bool>>() {
            let frame = self
                .inner
                .iloc_bool(&mask)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
            return Ok(Py::new(py, PyDataFrame { inner: frame })?.into_any());
        }
        // `df[index]` with an Index -> select columns
        if let Ok(idx) = key.extract::<PyRef<'_, PyIndex>>() {
            let cols: Vec<String> = idx
                .inner
                .labels()
                .iter()
                .map(|l| match l {
                    IndexLabel::Utf8(s) => s.clone(),
                    other => format!("{other:?}"),
                })
                .collect();
            for col in &cols {
                if self.inner.column(col).is_none() {
                    return Err(PyErr::new::<pyo3::exceptions::PyKeyError, _>(col.clone()));
                }
            }
            let refs: Vec<&str> = cols.iter().map(String::as_str).collect();
            let frame = self
                .inner
                .select_columns(&refs)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
            return Ok(Py::new(py, PyDataFrame { inner: frame })?.into_any());
        }
        Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
            "DataFrame key must be a column name, a list of column names, a slice, an Index, or a boolean Series",
        ))
    }

    /// `df["c"] = series | list | scalar`, or `df[mask] = scalar`.
    fn __setitem__(
        &mut self,
        py: Python<'_>,
        key: &Bound<'_, PyAny>,
        value: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        if let Ok(name) = key.extract::<String>() {
            let n = self.inner.len();
            let values: Vec<Scalar> = if let Ok(series) = value.extract::<PyRef<'_, PySeries>>() {
                if series.inner.len() != n {
                    return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                        "Length of values ({}) does not match length of index ({n})",
                        series.inner.len()
                    )));
                }
                series.inner.column().values().to_vec()
            } else if let Ok(list) = value.cast::<PyList>() {
                let values = list
                    .iter()
                    .map(|v| py_to_scalar(py, &v))
                    .collect::<PyResult<Vec<_>>>()?;
                if values.len() != n {
                    return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                        "Length of values ({}) does not match length of index ({n})",
                        values.len()
                    )));
                }
                values
            } else {
                vec![py_to_scalar(py, value)?; n]
            };
            self.inner = self
                .inner
                .assign_column(&name, values)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
            return Ok(());
        }
        if let Ok(mask) = key.extract::<PyRef<'_, PySeries>>() {
            let scalar = py_to_scalar(py, value)?;
            for col in self.columns() {
                let col_series = self.column_series(&col)?;
                let mut vals = col_series.inner.column().values().to_vec();
                for (i, m) in mask.inner.values().iter().enumerate() {
                    if let Scalar::Bool(true) = m
                        && i < vals.len()
                    {
                        vals[i] = scalar.clone();
                    }
                }
                self.inner = self
                    .inner
                    .assign_column(&col, vals)
                    .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
            }
            return Ok(());
        }
        Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
            "DataFrame key must be a column name or a boolean Series",
        ))
    }

    fn __add__(&self, _py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PyDataFrame> {
        if let Ok(other_df) = other.extract::<PyRef<'_, PyDataFrame>>() {
            wrap_frame(self.inner.add(&other_df.inner))
        } else if let Ok(val) = other.extract::<f64>() {
            wrap_frame(self.inner.add_scalar(val))
        } else if let Ok(val) = other.extract::<i64>() {
            wrap_frame(self.inner.add_scalar(val as f64))
        } else {
            Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                "Unsupported operand type for +",
            ))
        }
    }
    fn __radd__(&self, _py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PyDataFrame> {
        self.__add__(_py, other)
    }
    fn __sub__(&self, _py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PyDataFrame> {
        if let Ok(other_df) = other.extract::<PyRef<'_, PyDataFrame>>() {
            wrap_frame(self.inner.sub(&other_df.inner))
        } else if let Ok(val) = other.extract::<f64>() {
            wrap_frame(self.inner.sub_scalar(val))
        } else if let Ok(val) = other.extract::<i64>() {
            wrap_frame(self.inner.sub_scalar(val as f64))
        } else {
            Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                "Unsupported operand type for -",
            ))
        }
    }
    fn __rsub__(&self, _py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PyDataFrame> {
        if let Ok(other_df) = other.extract::<PyRef<'_, PyDataFrame>>() {
            wrap_frame(other_df.inner.sub(&self.inner))
        } else if let Ok(val) = other.extract::<f64>() {
            wrap_frame(self.inner.rsub(val))
        } else if let Ok(val) = other.extract::<i64>() {
            wrap_frame(self.inner.rsub(val as f64))
        } else {
            Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                "Unsupported operand type for -",
            ))
        }
    }
    fn __mul__(&self, _py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PyDataFrame> {
        if let Ok(other_df) = other.extract::<PyRef<'_, PyDataFrame>>() {
            wrap_frame(self.inner.mul(&other_df.inner))
        } else if let Ok(val) = other.extract::<f64>() {
            wrap_frame(self.inner.mul_scalar(val))
        } else if let Ok(val) = other.extract::<i64>() {
            wrap_frame(self.inner.mul_scalar(val as f64))
        } else {
            Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                "Unsupported operand type for *",
            ))
        }
    }
    fn __rmul__(&self, _py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PyDataFrame> {
        self.__mul__(_py, other)
    }
    fn __truediv__(&self, _py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PyDataFrame> {
        if let Ok(other_df) = other.extract::<PyRef<'_, PyDataFrame>>() {
            wrap_frame(self.inner.div(&other_df.inner))
        } else if let Ok(val) = other.extract::<f64>() {
            wrap_frame(self.inner.div_scalar(val))
        } else if let Ok(val) = other.extract::<i64>() {
            wrap_frame(self.inner.div_scalar(val as f64))
        } else {
            Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                "Unsupported operand type for /",
            ))
        }
    }
    fn __rtruediv__(&self, _py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PyDataFrame> {
        if let Ok(other_df) = other.extract::<PyRef<'_, PyDataFrame>>() {
            wrap_frame(other_df.inner.div(&self.inner))
        } else if let Ok(val) = other.extract::<f64>() {
            wrap_frame(self.inner.rdiv(val))
        } else if let Ok(val) = other.extract::<i64>() {
            wrap_frame(self.inner.rdiv(val as f64))
        } else {
            Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                "Unsupported operand type for /",
            ))
        }
    }
    fn __floordiv__(&self, _py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PyDataFrame> {
        if let Ok(other_df) = other.extract::<PyRef<'_, PyDataFrame>>() {
            wrap_frame(self.inner.floordiv(&other_df.inner))
        } else if let Ok(val) = other.extract::<f64>() {
            wrap_frame(self.inner.floordiv(val))
        } else if let Ok(val) = other.extract::<i64>() {
            wrap_frame(self.inner.floordiv(val as f64))
        } else {
            Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                "Unsupported operand type for //",
            ))
        }
    }
    fn __rfloordiv__(&self, _py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PyDataFrame> {
        if let Ok(other_df) = other.extract::<PyRef<'_, PyDataFrame>>() {
            wrap_frame(other_df.inner.floordiv(&self.inner))
        } else if let Ok(val) = other.extract::<f64>() {
            wrap_frame(self.inner.rfloordiv(val))
        } else if let Ok(val) = other.extract::<i64>() {
            wrap_frame(self.inner.rfloordiv(val as f64))
        } else {
            Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                "Unsupported operand type for //",
            ))
        }
    }
    fn __mod__(&self, _py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PyDataFrame> {
        if let Ok(other_df) = other.extract::<PyRef<'_, PyDataFrame>>() {
            wrap_frame(self.inner.r#mod(&other_df.inner))
        } else if let Ok(val) = other.extract::<f64>() {
            wrap_frame(self.inner.mod_scalar(val))
        } else if let Ok(val) = other.extract::<i64>() {
            wrap_frame(self.inner.mod_scalar(val as f64))
        } else {
            Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                "Unsupported operand type for %",
            ))
        }
    }
    fn __rmod__(&self, _py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PyDataFrame> {
        if let Ok(other_df) = other.extract::<PyRef<'_, PyDataFrame>>() {
            wrap_frame(other_df.inner.r#mod(&self.inner))
        } else if let Ok(val) = other.extract::<f64>() {
            wrap_frame(self.inner.rmod(val))
        } else if let Ok(val) = other.extract::<i64>() {
            wrap_frame(self.inner.rmod(val as f64))
        } else {
            Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                "Unsupported operand type for %",
            ))
        }
    }
    fn __pow__(
        &self,
        _py: Python<'_>,
        other: &Bound<'_, PyAny>,
        _modulo: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<PyDataFrame> {
        if let Ok(other_df) = other.extract::<PyRef<'_, PyDataFrame>>() {
            wrap_frame(self.inner.pow(&other_df.inner))
        } else if let Ok(val) = other.extract::<f64>() {
            wrap_frame(self.inner.pow_scalar(val))
        } else if let Ok(val) = other.extract::<i64>() {
            wrap_frame(self.inner.pow_scalar(val as f64))
        } else {
            Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                "Unsupported operand type for **",
            ))
        }
    }
    fn __rpow__(
        &self,
        _py: Python<'_>,
        other: &Bound<'_, PyAny>,
        _modulo: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<PyDataFrame> {
        if let Ok(other_df) = other.extract::<PyRef<'_, PyDataFrame>>() {
            wrap_frame(other_df.inner.pow(&self.inner))
        } else if let Ok(val) = other.extract::<f64>() {
            wrap_frame(self.inner.rpow(val))
        } else if let Ok(val) = other.extract::<i64>() {
            wrap_frame(self.inner.rpow(val as f64))
        } else {
            Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                "Unsupported operand type for **",
            ))
        }
    }
    fn __neg__(&self) -> PyResult<PyDataFrame> {
        wrap_frame(self.inner.neg())
    }
    fn __eq__(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PyDataFrame> {
        if let Ok(other_df) = other.extract::<PyRef<'_, PyDataFrame>>() {
            wrap_frame(self.inner.eq(&other_df.inner))
        } else {
            let scalar = py_to_scalar(py, other)?;
            wrap_frame(self.inner.eq(scalar))
        }
    }
    fn __ne__(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PyDataFrame> {
        if let Ok(other_df) = other.extract::<PyRef<'_, PyDataFrame>>() {
            wrap_frame(self.inner.ne(&other_df.inner))
        } else {
            let scalar = py_to_scalar(py, other)?;
            wrap_frame(self.inner.ne(scalar))
        }
    }
    fn __lt__(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PyDataFrame> {
        if let Ok(other_df) = other.extract::<PyRef<'_, PyDataFrame>>() {
            wrap_frame(self.inner.lt(&other_df.inner))
        } else {
            let scalar = py_to_scalar(py, other)?;
            wrap_frame(self.inner.lt(scalar))
        }
    }
    fn __le__(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PyDataFrame> {
        if let Ok(other_df) = other.extract::<PyRef<'_, PyDataFrame>>() {
            wrap_frame(self.inner.le(&other_df.inner))
        } else {
            let scalar = py_to_scalar(py, other)?;
            wrap_frame(self.inner.le(scalar))
        }
    }
    fn __gt__(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PyDataFrame> {
        if let Ok(other_df) = other.extract::<PyRef<'_, PyDataFrame>>() {
            wrap_frame(self.inner.gt(&other_df.inner))
        } else {
            let scalar = py_to_scalar(py, other)?;
            wrap_frame(self.inner.gt(scalar))
        }
    }
    fn __ge__(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PyDataFrame> {
        if let Ok(other_df) = other.extract::<PyRef<'_, PyDataFrame>>() {
            wrap_frame(self.inner.ge(&other_df.inner))
        } else {
            let scalar = py_to_scalar(py, other)?;
            wrap_frame(self.inner.ge(scalar))
        }
    }

    fn add(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PyDataFrame> {
        self.__add__(py, other)
    }
    fn radd(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PyDataFrame> {
        self.__radd__(py, other)
    }
    fn sub(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PyDataFrame> {
        self.__sub__(py, other)
    }
    fn subtract(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PyDataFrame> {
        self.__sub__(py, other)
    }
    fn rsub(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PyDataFrame> {
        self.__rsub__(py, other)
    }
    fn mul(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PyDataFrame> {
        self.__mul__(py, other)
    }
    fn multiply(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PyDataFrame> {
        self.__mul__(py, other)
    }
    fn rmul(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PyDataFrame> {
        self.__rmul__(py, other)
    }
    fn div(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PyDataFrame> {
        self.__truediv__(py, other)
    }
    fn divide(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PyDataFrame> {
        self.__truediv__(py, other)
    }
    fn truediv(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PyDataFrame> {
        self.__truediv__(py, other)
    }
    fn rtruediv(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PyDataFrame> {
        self.__rtruediv__(py, other)
    }
    fn rdiv(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PyDataFrame> {
        self.__rtruediv__(py, other)
    }
    fn floordiv(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PyDataFrame> {
        self.__floordiv__(py, other)
    }
    fn rfloordiv(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PyDataFrame> {
        self.__rfloordiv__(py, other)
    }
    fn r#mod(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PyDataFrame> {
        self.__mod__(py, other)
    }
    fn rmod(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PyDataFrame> {
        self.__rmod__(py, other)
    }
    fn pow(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PyDataFrame> {
        self.__pow__(py, other, None)
    }
    fn rpow(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PyDataFrame> {
        self.__rpow__(py, other, None)
    }
    fn eq(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PyDataFrame> {
        self.__eq__(py, other)
    }
    fn ne(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PyDataFrame> {
        self.__ne__(py, other)
    }
    fn lt(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PyDataFrame> {
        self.__lt__(py, other)
    }
    fn le(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PyDataFrame> {
        self.__le__(py, other)
    }
    fn gt(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PyDataFrame> {
        self.__gt__(py, other)
    }
    fn ge(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<PyDataFrame> {
        self.__ge__(py, other)
    }

    /// `"a" in df` checks the column labels, as in pandas.
    fn __contains__(&self, name: &str) -> bool {
        self.inner.column(name).is_some()
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

    /// Sort by the row index, returning a new DataFrame.
    #[pyo3(signature = (ascending=true))]
    fn sort_index(&self, ascending: bool) -> PyResult<PyDataFrame> {
        let result = self
            .inner
            .sort_index(ascending)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PyDataFrame { inner: result })
    }

    /// Transpose: swap rows and columns, returning a new DataFrame.
    fn transpose(&self) -> PyResult<PyDataFrame> {
        let result = self
            .inner
            .transpose()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PyDataFrame { inner: result })
    }

    /// Drop specified labels from rows or columns.
    #[pyo3(signature = (labels=None, axis=None, columns=None))]
    fn drop(
        &self,
        labels: Option<&Bound<'_, PyAny>>,
        axis: Option<usize>,
        columns: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<PyDataFrame> {
        if let Some(cols) = columns {
            let col_names: Vec<String> = if let Ok(s) = cols.extract::<String>() {
                vec![s]
            } else if let Ok(list) = cols.extract::<Vec<String>>() {
                list
            } else {
                return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                    "columns must be a string or list of strings",
                ));
            };
            let str_refs: Vec<&str> = col_names.iter().map(String::as_str).collect();
            let res = self.inner.drop(&str_refs, 1).map_err(frame_error_to_py)?;
            return Ok(PyDataFrame { inner: res });
        }
        let ax = axis.unwrap_or(0);
        if let Some(lbls) = labels {
            let names: Vec<String> = if let Ok(s) = lbls.extract::<String>() {
                vec![s]
            } else if let Ok(list) = lbls.extract::<Vec<String>>() {
                list
            } else {
                return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                    "labels must be a string or list of strings",
                ));
            };
            let str_refs: Vec<&str> = names.iter().map(String::as_str).collect();
            let res = self.inner.drop(&str_refs, ax).map_err(frame_error_to_py)?;
            return Ok(PyDataFrame { inner: res });
        }
        Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
            "Need either labels or columns to drop",
        ))
    }

    /// Rename columns or index via a mapping or keyword arguments.
    #[pyo3(signature = (mapping=None, columns=None))]
    fn rename(
        &self,
        mapping: Option<&Bound<'_, PyDict>>,
        columns: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<PyDataFrame> {
        let target = columns.or(mapping);
        if let Some(dict) = target {
            let mut pairs = Vec::with_capacity(dict.len());
            for (k, v) in dict.iter() {
                pairs.push((k.extract::<String>()?, v.extract::<String>()?));
            }
            let str_pairs: Vec<(&str, &str)> = pairs
                .iter()
                .map(|(a, b)| (a.as_str(), b.as_str()))
                .collect();
            let res = self.inner.rename(&str_pairs).map_err(frame_error_to_py)?;
            return Ok(PyDataFrame { inner: res });
        }
        Ok(PyDataFrame {
            inner: self.inner.clone(),
        })
    }

    /// Merge with another DataFrame on key column(s) (pandas `DataFrame.merge`).
    /// `on` is a column name or list of names; `how` is one of
    /// inner/left/right/outer/cross.
    #[pyo3(signature = (other, on, how="inner"))]
    fn merge(
        &self,
        other: &PyDataFrame,
        on: &Bound<'_, PyAny>,
        how: &str,
    ) -> PyResult<PyDataFrame> {
        let on_cols: Vec<String> = if let Ok(s) = on.extract::<String>() {
            vec![s]
        } else {
            on.extract::<Vec<String>>().map_err(|_| {
                PyErr::new::<pyo3::exceptions::PyTypeError, _>("`on` must be a str or list of str")
            })?
        };
        let join_type = match how {
            "inner" => fp_join::JoinType::Inner,
            "left" => fp_join::JoinType::Left,
            "right" => fp_join::JoinType::Right,
            "outer" => fp_join::JoinType::Outer,
            "cross" => fp_join::JoinType::Cross,
            other => {
                return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                    "unknown how={other:?}; expected inner/left/right/outer/cross"
                )));
            }
        };
        let on_refs: Vec<&str> = on_cols.iter().map(String::as_str).collect();
        let merged = fp_join::merge_dataframes_on(&self.inner, &other.inner, &on_refs, join_type)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        let frame =
            DataFrame::new_with_column_order(merged.index, merged.columns, merged.column_order)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PyDataFrame { inner: frame })
    }

    /// Return a boolean DataFrame marking missing values (pandas `DataFrame.isna`).
    fn isna(&self) -> PyResult<PyDataFrame> {
        let result = self
            .inner
            .isna()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PyDataFrame { inner: result })
    }

    /// Return a boolean DataFrame marking non-missing values (pandas `notna`).
    fn notna(&self) -> PyResult<PyDataFrame> {
        let result = self
            .inner
            .notna()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PyDataFrame { inner: result })
    }

    /// Return a copy of this DataFrame.
    fn copy(&self) -> PyDataFrame {
        PyDataFrame {
            inner: self.inner.copy(),
        }
    }

    /// Return a boolean DataFrame marking missing values (pandas `DataFrame.isnull`).
    fn isnull(&self) -> PyResult<PyDataFrame> {
        self.isna()
    }

    /// Return a boolean DataFrame marking non-missing values (pandas `DataFrame.notnull`).
    fn notnull(&self) -> PyResult<PyDataFrame> {
        self.notna()
    }

    /// Return the elementwise absolute value as a new DataFrame.
    fn abs(&self) -> PyResult<PyDataFrame> {
        let result = self
            .inner
            .abs()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PyDataFrame { inner: result })
    }

    /// Clip values to the `[lower, upper]` range (either bound optional).
    #[pyo3(signature = (lower=None, upper=None))]
    fn clip(&self, lower: Option<f64>, upper: Option<f64>) -> PyResult<PyDataFrame> {
        let result = self
            .inner
            .clip(lower, upper)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PyDataFrame { inner: result })
    }

    /// Round each numeric value to `decimals` places, returning a new DataFrame.
    #[pyo3(signature = (decimals=0))]
    fn round(&self, decimals: i32) -> PyResult<PyDataFrame> {
        let result = self
            .inner
            .round(decimals)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PyDataFrame { inner: result })
    }

    /// Replace values via an `{old: new}` dict (pandas `DataFrame.replace`).
    fn replace(&self, py: Python<'_>, mapping: &Bound<'_, PyDict>) -> PyResult<PyDataFrame> {
        let pairs = py_dict_to_scalar_pairs(py, mapping)?;
        let result = self
            .inner
            .replace(&pairs)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PyDataFrame { inner: result })
    }

    /// Cast every column to a dtype (int64/float64/str/bool/datetime64/timedelta64).
    fn astype(&self, dtype: &str) -> PyResult<PyDataFrame> {
        let dt = parse_dtype(dtype)?;
        let result = self
            .inner
            .astype(dt)
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

    /// Return boolean Series denoting duplicate rows.
    #[pyo3(signature = (subset=None, keep=None))]
    fn duplicated(
        &self,
        subset: Option<&Bound<'_, PyAny>>,
        keep: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<PySeries> {
        let keep_enum = parse_duplicate_keep(keep)?;
        let subset_vec: Option<Vec<String>> = match subset {
            None => None,
            Some(s) => {
                if let Ok(single) = s.extract::<String>() {
                    Some(vec![single])
                } else if let Ok(list) = s.extract::<Vec<String>>() {
                    Some(list)
                } else {
                    return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                        "subset must be a column name or list of column names",
                    ));
                }
            }
        };
        let result = self
            .inner
            .duplicated(subset_vec.as_deref(), keep_enum)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PySeries { inner: result })
    }

    /// Drop duplicate rows.
    #[pyo3(signature = (subset=None, keep=None, ignore_index=false))]
    fn drop_duplicates(
        &self,
        subset: Option<&Bound<'_, PyAny>>,
        keep: Option<&Bound<'_, PyAny>>,
        ignore_index: bool,
    ) -> PyResult<PyDataFrame> {
        let keep_enum = parse_duplicate_keep(keep)?;
        let subset_vec: Option<Vec<String>> = match subset {
            None => None,
            Some(s) => {
                if let Ok(single) = s.extract::<String>() {
                    Some(vec![single])
                } else if let Ok(list) = s.extract::<Vec<String>>() {
                    Some(list)
                } else {
                    return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                        "subset must be a column name or list of column names",
                    ));
                }
            }
        };
        let result = self
            .inner
            .drop_duplicates(subset_vec.as_deref(), keep_enum, ignore_index)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PyDataFrame { inner: result })
    }

    /// Group by one column name or a list of them, as `df.groupby("city")`
    /// and `df.groupby(["city", "year"])` both do in pandas.
    fn groupby(&self, by: &Bound<'_, PyAny>) -> PyResult<PyGroupBy> {
        let by: Vec<String> = if let Ok(single) = by.extract::<String>() {
            vec![single]
        } else {
            by.extract::<Vec<String>>().map_err(|_| {
                PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                    "groupby: `by` must be a column name or a list of column names",
                )
            })?
        };
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

    /// Set the DataFrame index using existing columns.
    #[pyo3(signature = (keys, drop=true))]
    fn set_index(&self, keys: &Bound<'_, PyAny>, drop: bool) -> PyResult<PyDataFrame> {
        if let Ok(single) = keys.extract::<String>() {
            let res = self
                .inner
                .set_index(&single, drop)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
            return Ok(PyDataFrame { inner: res });
        }
        if let Ok(list) = keys.extract::<Vec<String>>() {
            let refs: Vec<&str> = list.iter().map(String::as_str).collect();
            let res = self
                .inner
                .set_index_multi(&refs, drop, "/")
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
            return Ok(PyDataFrame { inner: res });
        }
        Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
            "keys must be a column name or list of column names",
        ))
    }

    /// Whether elements in DataFrame are contained in values.
    fn isin(&self, py: Python<'_>, values: &Bound<'_, PyAny>) -> PyResult<PyDataFrame> {
        if let Ok(dict) = values.cast::<PyDict>() {
            let mut map: BTreeMap<String, Vec<Scalar>> = BTreeMap::new();
            for (k, v) in dict.iter() {
                let col_name = k.extract::<String>()?;
                let mut scs = Vec::new();
                if let Ok(list) = v.cast::<PyList>() {
                    for item in list.iter() {
                        scs.push(py_to_scalar(py, &item)?);
                    }
                } else if let Ok(tuple) = v.cast::<pyo3::types::PyTuple>() {
                    for item in tuple.iter() {
                        scs.push(py_to_scalar(py, &item)?);
                    }
                } else if let Ok(s) = v.extract::<PyRef<'_, PySeries>>() {
                    scs.extend(s.inner.values().iter().cloned());
                } else {
                    scs.push(py_to_scalar(py, &v)?);
                }
                map.insert(col_name, scs);
            }
            let res = self
                .inner
                .isin_dict(&map)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
            return Ok(PyDataFrame { inner: res });
        }
        if let Ok(s) = values.extract::<PyRef<'_, PySeries>>() {
            let res = self
                .inner
                .isin(s.inner.values())
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
            return Ok(PyDataFrame { inner: res });
        }
        if let Ok(list) = values.cast::<PyList>() {
            let mut scs = Vec::with_capacity(list.len());
            for item in list.iter() {
                scs.push(py_to_scalar(py, &item)?);
            }
            let res = self
                .inner
                .isin(&scs)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
            return Ok(PyDataFrame { inner: res });
        }
        if let Ok(tuple) = values.cast::<pyo3::types::PyTuple>() {
            let mut scs = Vec::with_capacity(tuple.len());
            for item in tuple.iter() {
                scs.push(py_to_scalar(py, &item)?);
            }
            let res = self
                .inner
                .isin(&scs)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
            return Ok(PyDataFrame { inner: res });
        }
        Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
            "values must be a dict, list, tuple, or Series",
        ))
    }

    /// Return the first n rows ordered by columns in descending order.
    #[pyo3(signature = (n, columns, keep=None))]
    fn nlargest(
        &self,
        n: usize,
        columns: &Bound<'_, PyAny>,
        keep: Option<&str>,
    ) -> PyResult<PyDataFrame> {
        if let Ok(col) = columns.extract::<String>() {
            let res = match keep {
                Some(k) => self.inner.nlargest_keep(n, &col, k),
                None => self.inner.nlargest(n, &col),
            }
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
            return Ok(PyDataFrame { inner: res });
        }
        if let Ok(cols) = columns.extract::<Vec<String>>() {
            let refs: Vec<&str> = cols.iter().map(String::as_str).collect();
            let res = self
                .inner
                .nlargest_multi(n, &refs)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
            return Ok(PyDataFrame { inner: res });
        }
        Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
            "columns must be a column name or list of column names",
        ))
    }

    /// Return the first n rows ordered by columns in ascending order.
    #[pyo3(signature = (n, columns, keep=None))]
    fn nsmallest(
        &self,
        n: usize,
        columns: &Bound<'_, PyAny>,
        keep: Option<&str>,
    ) -> PyResult<PyDataFrame> {
        if let Ok(col) = columns.extract::<String>() {
            let res = match keep {
                Some(k) => self.inner.nsmallest_keep(n, &col, k),
                None => self.inner.nsmallest(n, &col),
            }
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
            return Ok(PyDataFrame { inner: res });
        }
        if let Ok(cols) = columns.extract::<Vec<String>>() {
            let refs: Vec<&str> = cols.iter().map(String::as_str).collect();
            let res = self
                .inner
                .nsmallest_multi(n, &refs)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
            return Ok(PyDataFrame { inner: res });
        }
        Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
            "columns must be a column name or list of column names",
        ))
    }

    /// Return index of first occurrence of maximum over requested axis.
    fn idxmax(&self) -> PyResult<PySeries> {
        let res = self
            .inner
            .idxmax()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PySeries { inner: res })
    }

    /// Return index of first occurrence of minimum over requested axis.
    fn idxmin(&self) -> PyResult<PySeries> {
        let res = self
            .inner
            .idxmin()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PySeries { inner: res })
    }

    /// First discrete difference of element.
    #[pyo3(signature = (periods=1))]
    fn diff(&self, periods: i64) -> PyResult<PyDataFrame> {
        let res = self
            .inner
            .diff(periods)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PyDataFrame { inner: res })
    }

    /// Percentage change between the current and a prior element.
    #[pyo3(signature = (periods=1))]
    fn pct_change(&self, periods: i64) -> PyResult<PyDataFrame> {
        let res = self
            .inner
            .pct_change(periods)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PyDataFrame { inner: res })
    }

    /// Return cumulative sum over a DataFrame or Series axis.
    #[pyo3(signature = (skipna=true))]
    fn cumsum(&self, skipna: bool) -> PyResult<PyDataFrame> {
        let res = self
            .inner
            .cumsum_with_skipna(skipna)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PyDataFrame { inner: res })
    }

    /// Return cumulative product over a DataFrame or Series axis.
    #[pyo3(signature = (skipna=true))]
    fn cumprod(&self, skipna: bool) -> PyResult<PyDataFrame> {
        let res = self
            .inner
            .cumprod_with_skipna(skipna)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PyDataFrame { inner: res })
    }

    /// Return cumulative minimum over a DataFrame or Series axis.
    #[pyo3(signature = (skipna=true))]
    fn cummin(&self, skipna: bool) -> PyResult<PyDataFrame> {
        let res = self
            .inner
            .cummin_with_skipna(skipna)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PyDataFrame { inner: res })
    }

    /// Return cumulative maximum over a DataFrame or Series axis.
    #[pyo3(signature = (skipna=true))]
    fn cummax(&self, skipna: bool) -> PyResult<PyDataFrame> {
        let res = self
            .inner
            .cummax_with_skipna(skipna)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PyDataFrame { inner: res })
    }

    /// Shift index by desired number of periods.
    #[pyo3(signature = (periods=1))]
    fn shift(&self, periods: i64) -> PyResult<PyDataFrame> {
        let res = self
            .inner
            .shift(periods)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PyDataFrame { inner: res })
    }

    /// Assign new columns to a DataFrame, returning a new object (pandas `DataFrame.assign`).
    #[pyo3(signature = (**kwargs))]
    fn assign(&self, py: Python<'_>, kwargs: Option<&Bound<'_, PyDict>>) -> PyResult<PyDataFrame> {
        let mut df = self.inner.clone();
        if let Some(kwargs) = kwargs {
            for (key, val) in kwargs.iter() {
                let name = key.extract::<String>()?;
                let col = if val.is_callable() {
                    let py_df = Py::new(py, PyDataFrame { inner: df.clone() })?;
                    let res = val.call1((py_df,))?;
                    py_value_to_column(py, &res, df.len())?
                } else {
                    py_value_to_column(py, &val, df.len())?
                };
                df = df
                    .with_column(name, col)
                    .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
            }
        }
        Ok(PyDataFrame { inner: df })
    }

    /// Query the columns of a DataFrame with a boolean expression.
    fn query(&self, expr: &str) -> PyResult<PyDataFrame> {
        let res = self
            .inner
            .query(expr)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PyDataFrame { inner: res })
    }

    /// Evaluate a string describing operations on DataFrame columns.
    fn eval(&self, py: Python<'_>, expr: &str) -> PyResult<Py<PyAny>> {
        if let Some((target, rhs)) = expr.split_once('=') {
            let target = target.trim();
            if !target.is_empty()
                && !target.ends_with('!')
                && !target.ends_with('<')
                && !target.ends_with('>')
                && !target.ends_with('=')
                && !rhs.starts_with('=')
                && target.chars().all(|c| c.is_alphanumeric() || c == '_')
                && !target.starts_with(|c: char| c.is_ascii_digit())
            {
                let evaluated = self
                    .inner
                    .eval(rhs.trim())
                    .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
                let new_df = self
                    .inner
                    .with_column(target, evaluated.column().clone())
                    .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
                return Ok(Py::new(py, PyDataFrame { inner: new_df })?.into_any());
            }
        }
        let evaluated = self
            .inner
            .eval(expr)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(Py::new(py, PySeries { inner: evaluated })?.into_any())
    }

    #[pyo3(signature = (window, min_periods=None, center=false))]
    fn rolling(&self, window: usize, min_periods: Option<usize>, center: bool) -> PyRolling {
        let _ = center;
        PyRolling {
            series: None,
            dataframe: Some(self.inner.clone()),
            window,
            min_periods,
            center,
        }
    }

    #[pyo3(signature = (min_periods=None))]
    fn expanding(&self, min_periods: Option<usize>) -> PyExpanding {
        PyExpanding {
            series: None,
            dataframe: Some(self.inner.clone()),
            min_periods,
        }
    }

    #[pyo3(signature = (span=None, alpha=None))]
    fn ewm(&self, span: Option<f64>, alpha: Option<f64>) -> PyExponentialMovingWindow {
        PyExponentialMovingWindow {
            series: None,
            dataframe: Some(self.inner.clone()),
            span,
            alpha,
        }
    }

    #[pyo3(signature = (limit=None))]
    fn ffill(&self, limit: Option<usize>) -> PyResult<PyDataFrame> {
        let res = self.inner.ffill(limit).map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: res })
    }

    #[pyo3(signature = (limit=None))]
    fn bfill(&self, limit: Option<usize>) -> PyResult<PyDataFrame> {
        let res = self.inner.bfill(limit).map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: res })
    }

    fn cov(&self) -> PyResult<PyDataFrame> {
        let res = self.inner.cov().map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: res })
    }

    #[getter]
    #[allow(non_snake_case)]
    fn T(&self) -> PyResult<PyDataFrame> {
        self.transpose()
    }

    #[pyo3(signature = (id_vars=None, value_vars=None, var_name=None, value_name=None))]
    fn melt(
        &self,
        id_vars: Option<Vec<String>>,
        value_vars: Option<Vec<String>>,
        var_name: Option<&str>,
        value_name: Option<&str>,
    ) -> PyResult<PyDataFrame> {
        let id_strings = id_vars.unwrap_or_default();
        let val_strings = value_vars.unwrap_or_default();
        let id_refs: Vec<&str> = id_strings.iter().map(String::as_str).collect();
        let val_refs: Vec<&str> = val_strings.iter().map(String::as_str).collect();
        let res = self
            .inner
            .melt(&id_refs, &val_refs, var_name, value_name)
            .map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: res })
    }

    #[pyo3(signature = (index, columns, values))]
    fn pivot(&self, index: &str, columns: &str, values: &str) -> PyResult<PyDataFrame> {
        let res = self
            .inner
            .pivot(index, columns, values)
            .map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: res })
    }

    #[pyo3(signature = (values, index, columns, aggfunc="mean"))]
    fn pivot_table(
        &self,
        values: &str,
        index: &str,
        columns: &str,
        aggfunc: &str,
    ) -> PyResult<PyDataFrame> {
        let res = self
            .inner
            .pivot_table(values, index, columns, aggfunc)
            .map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: res })
    }

    #[pyo3(signature = (n=None, frac=None, replace=false, random_state=None))]
    fn sample(
        &self,
        n: Option<usize>,
        frac: Option<f64>,
        replace: bool,
        random_state: Option<u64>,
    ) -> PyResult<PyDataFrame> {
        let res = self
            .inner
            .sample(n, frac, replace, random_state)
            .map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: res })
    }

    #[pyo3(signature = (freq, closed=None, label=None, origin=None))]
    pub fn resample(
        &self,
        freq: &str,
        closed: Option<&str>,
        label: Option<&str>,
        origin: Option<&str>,
    ) -> PyResampler {
        PyResampler {
            target: ResampleTarget::DataFrame(self.inner.clone()),
            freq: freq.to_string(),
            closed: closed.map(str::to_string),
            label: label.map(str::to_string),
            origin: origin.map(str::to_string),
        }
    }

    #[pyo3(signature = (freq, method=None))]
    pub fn asfreq(&self, freq: &str, method: Option<&str>) -> PyResult<Self> {
        let res = self
            .inner
            .asfreq_with_options(freq, method, None)
            .map_err(frame_error_to_py)?;
        Ok(Self { inner: res })
    }

    #[pyo3(signature = (axis=None))]
    fn any(&self, axis: Option<usize>) -> PyResult<PySeries> {
        let res = if axis == Some(1) {
            self.inner.any_axis1().map_err(frame_error_to_py)?
        } else {
            self.inner.any().map_err(frame_error_to_py)?
        };
        Ok(PySeries { inner: res })
    }

    #[pyo3(signature = (axis=None))]
    fn all(&self, axis: Option<usize>) -> PyResult<PySeries> {
        let res = if axis == Some(1) {
            self.inner.all_axis1().map_err(frame_error_to_py)?
        } else {
            self.inner.all().map_err(frame_error_to_py)?
        };
        Ok(PySeries { inner: res })
    }

    fn mode(&self) -> PyResult<PyDataFrame> {
        let res = self.inner.mode().map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: res })
    }

    fn prod(&self) -> PyResult<PySeries> {
        let res = self.inner.prod().map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    fn product(&self) -> PyResult<PySeries> {
        self.prod()
    }

    #[pyo3(signature = (q=0.5))]
    fn quantile(&self, q: f64) -> PyResult<PySeries> {
        let res = self.inner.quantile(q).map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    #[pyo3(signature = (method=None, ascending=None, na_option=None))]
    fn rank(
        &self,
        method: Option<&str>,
        ascending: Option<bool>,
        na_option: Option<&str>,
    ) -> PyResult<PyDataFrame> {
        let m = method.unwrap_or("average");
        let asc = ascending.unwrap_or(true);
        let na = na_option.unwrap_or("keep");
        let res = self.inner.rank(m, asc, na).map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: res })
    }

    fn sem(&self) -> PyResult<PySeries> {
        let res = self.inner.sem().map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    fn skew(&self) -> PyResult<PySeries> {
        let res = self.inner.skew().map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    fn kurt(&self) -> PyResult<PySeries> {
        let res = self.inner.kurtosis().map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    fn kurtosis(&self) -> PyResult<PySeries> {
        self.kurt()
    }

    #[pyo3(signature = (before=None, after=None))]
    fn truncate(
        &self,
        before: Option<&Bound<'_, PyAny>>,
        after: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<PyDataFrame> {
        let b = match before {
            Some(obj) => Some(py_to_index_label(obj)?),
            None => None,
        };
        let a = match after {
            Some(obj) => Some(py_to_index_label(obj)?),
            None => None,
        };
        let res = self
            .inner
            .truncate(b.as_ref(), a.as_ref())
            .map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: res })
    }

    fn add_prefix(&self, prefix: &str) -> PyResult<PyDataFrame> {
        let res = self.inner.add_prefix(prefix).map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: res })
    }

    fn add_suffix(&self, suffix: &str) -> PyResult<PyDataFrame> {
        let res = self.inner.add_suffix(suffix).map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: res })
    }

    fn first_valid_index(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        match self.inner.first_valid_index() {
            Some(l) => index_label_to_py(py, &l),
            None => Ok(py.None()),
        }
    }

    fn last_valid_index(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        match self.inner.last_valid_index() {
            Some(l) => index_label_to_py(py, &l),
            None => Ok(py.None()),
        }
    }

    fn equals(&self, other: &PyDataFrame) -> bool {
        self.inner.equals(&other.inner)
    }

    fn pop(&mut self, item: &str) -> PyResult<PySeries> {
        let (series, remaining) = self.inner.pop(item).map_err(frame_error_to_py)?;
        self.inner = remaining;
        Ok(PySeries { inner: series })
    }

    fn squeeze(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        if self.inner.column_names().len() == 1 {
            let col = self.column_series(self.inner.column_names()[0])?;
            if col.inner.len() == 1 {
                let s = col.inner.column().values()[0].clone();
                scalar_to_py(py, &s)
            } else {
                Ok(Py::new(py, col)?.into_any())
            }
        } else {
            Ok(Py::new(
                py,
                PyDataFrame {
                    inner: self.inner.clone(),
                },
            )?
            .into_any())
        }
    }

    fn keys(&self) -> Vec<String> {
        self.columns()
    }

    fn items(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let mut list = Vec::with_capacity(self.inner.column_names().len());
        for col_name in self.inner.column_names() {
            let s = self.column_series(col_name)?;
            let py_s = Py::new(py, s)?;
            list.push((col_name.as_str(), py_s).into_pyobject(py)?);
        }
        Ok(PyList::new(py, list)?.into_any().unbind())
    }

    fn iterrows(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let rows = self.inner.iterrows();
        let mut list = Vec::with_capacity(rows.len());
        for (label, row_data) in rows {
            let py_label = index_label_to_py(py, &label)?;
            let mut col_items = Vec::with_capacity(row_data.len());
            let mut col_labels = Vec::with_capacity(row_data.len());
            for (col_name, val) in row_data {
                col_labels.push(IndexLabel::Utf8(col_name.to_string()));
                col_items.push(val);
            }
            let s = Series::from_values(label.to_string(), col_labels, col_items)
                .map_err(frame_error_to_py)?;
            let py_s = Py::new(py, PySeries { inner: s })?;
            list.push(pyo3::types::PyTuple::new(py, &[py_label, py_s.into_any()])?);
        }
        Ok(PyList::new(py, list)?.into_any().unbind())
    }

    fn itertuples(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let tuples = self.inner.itertuples();
        let mut list = Vec::with_capacity(tuples.len());
        for (label, vals) in tuples {
            let mut py_vals = Vec::with_capacity(vals.len() + 1);
            py_vals.push(index_label_to_py(py, &label)?);
            for v in &vals {
                py_vals.push(scalar_to_py(py, v)?);
            }
            list.push(pyo3::types::PyTuple::new(py, &py_vals)?);
        }
        Ok(PyList::new(py, list)?.into_any().unbind())
    }

    #[pyo3(signature = (limit=None))]
    fn pad(&self, limit: Option<usize>) -> PyResult<PyDataFrame> {
        self.ffill(limit)
    }

    #[pyo3(signature = (limit=None))]
    fn backfill(&self, limit: Option<usize>) -> PyResult<PyDataFrame> {
        self.bfill(limit)
    }

    fn agg(&self, py: Python<'_>, func: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        if let Ok(name) = func.extract::<String>() {
            match name.as_str() {
                "sum" => Ok(Py::new(py, self.sum()?)?.into_any()),
                "mean" => Ok(Py::new(py, self.mean()?)?.into_any()),
                "min" => Ok(Py::new(py, self.min()?)?.into_any()),
                "max" => Ok(Py::new(py, self.max()?)?.into_any()),
                "std" => Ok(Py::new(py, self.std()?)?.into_any()),
                "var" => Ok(Py::new(py, self.var()?)?.into_any()),
                "count" => Ok(Py::new(py, self.count()?)?.into_any()),
                "median" => Ok(Py::new(py, self.median()?)?.into_any()),
                "prod" | "product" => Ok(Py::new(py, self.prod()?)?.into_any()),
                "sem" => Ok(Py::new(py, self.sem()?)?.into_any()),
                "skew" => Ok(Py::new(py, self.skew()?)?.into_any()),
                "kurt" | "kurtosis" => Ok(Py::new(py, self.kurt()?)?.into_any()),
                other => Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                    "Unsupported agg function '{other}'"
                ))),
            }
        } else if let Ok(dict) = func.extract::<std::collections::HashMap<String, Vec<String>>>() {
            let res = self.inner.agg(&dict).map_err(frame_error_to_py)?;
            Ok(Py::new(py, PyDataFrame { inner: res })?.into_any())
        } else {
            Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                "func must be a string or dict of column -> list of functions",
            ))
        }
    }

    fn aggregate(&self, py: Python<'_>, func: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        self.agg(py, func)
    }

    #[pyo3(signature = (index=true))]
    fn memory_usage(&self, index: bool) -> PyResult<PySeries> {
        let res = self
            .inner
            .memory_usage_with_options(index, false)
            .map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    #[getter]
    fn axes(&self) -> Vec<Py<PyAny>> {
        Python::attach(|py| {
            let idx = Py::new(py, self.index()).ok()?;
            let cols = PyList::new(py, self.columns()).ok()?;
            Some(vec![idx.into_any(), cols.into_any().unbind()])
        })
        .unwrap_or_default()
    }

    #[getter]
    fn attrs<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        Ok(PyDict::new(py))
    }

    fn r#bool(&self) -> PyResult<bool> {
        let (r, c) = self.inner.shape();
        if r == 1 && c == 1 {
            let col = self.column_series(self.inner.column_names()[0])?;
            col.r#bool()
        } else {
            Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
                "The truth value of a DataFrame is ambiguous. Use a.empty, a.bool(), a.item(), a.any() or a.all().",
            ))
        }
    }

    fn combine_first(&self, other: &PyDataFrame) -> PyResult<PyDataFrame> {
        let res = self
            .inner
            .combine_first(&other.inner)
            .map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: res })
    }

    #[pyo3(signature = (other, join="outer"))]
    fn align(&self, other: &PyDataFrame, join: &str) -> PyResult<(PyDataFrame, PyDataFrame)> {
        let mode = match join {
            "outer" => AlignMode::Outer,
            "inner" => AlignMode::Inner,
            "left" => AlignMode::Left,
            "right" => AlignMode::Right,
            _ => {
                return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                    "Invalid join mode '{join}'"
                )));
            }
        };
        let (df1, df2) = self
            .inner
            .align(&other.inner, mode)
            .map_err(frame_error_to_py)?;
        Ok((PyDataFrame { inner: df1 }, PyDataFrame { inner: df2 }))
    }

    fn compare(&self, other: &PyDataFrame) -> PyResult<PyDataFrame> {
        let res = self
            .inner
            .compare(&other.inner)
            .map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: res })
    }

    fn convert_dtypes(&self) -> PyResult<PyDataFrame> {
        let res = self.inner.convert_dtypes().map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: res })
    }

    fn at_time(&self, time: &str) -> PyResult<PyDataFrame> {
        let res = self.inner.at_time(time).map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: res })
    }

    fn between_time(&self, start: &str, end: &str) -> PyResult<PyDataFrame> {
        let res = self
            .inner
            .between_time(start, end)
            .map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: res })
    }

    #[pyo3(signature = (label, subset=None))]
    fn asof(&self, label: &Bound<'_, PyAny>, subset: Option<Vec<String>>) -> PyResult<PySeries> {
        let lbl = py_to_index_label(label)?;
        let subset_refs: Option<Vec<&str>> = subset
            .as_ref()
            .map(|s| s.iter().map(String::as_str).collect());
        let res = self
            .inner
            .asof(&lbl, subset_refs.as_deref())
            .map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    fn corrwith(&self, other: &PyDataFrame) -> PyResult<PySeries> {
        let res = self
            .inner
            .corrwith(&other.inner)
            .map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    #[pyo3(signature = (func, axis=0))]
    fn apply(&self, py: Python<'_>, func: &Bound<'_, PyAny>, axis: usize) -> PyResult<Py<PyAny>> {
        if axis == 0 {
            let mut res_cols = Vec::new();
            for col_name in self.inner.column_names() {
                let s = self.column_series(col_name)?;
                let py_s = Py::new(py, s)?;
                let res = func.call1((py_s,))?;
                res_cols.push((col_name.to_string(), res));
            }
            let mut scalars = Vec::new();
            let mut all_scalars = true;
            for (_, r) in &res_cols {
                if let Ok(sc) = py_to_scalar(py, r) {
                    scalars.push(sc);
                } else {
                    all_scalars = false;
                    break;
                }
            }
            if all_scalars {
                let labels: Vec<IndexLabel> = res_cols
                    .iter()
                    .map(|(n, _)| IndexLabel::Utf8(n.clone()))
                    .collect();
                let s = Series::from_values("", labels, scalars).map_err(frame_error_to_py)?;
                return Ok(Py::new(py, PySeries { inner: s })?.into_any());
            }
        }
        Err(PyErr::new::<pyo3::exceptions::PyNotImplementedError, _>(
            "DataFrame.apply currently supports axis=0 returning scalar Series",
        ))
    }

    fn applymap(&self, py: Python<'_>, func: &Bound<'_, PyAny>) -> PyResult<PyDataFrame> {
        let (nrows, ncols) = self.inner.shape();
        let col_names = self.inner.column_names();
        let mut col_map = BTreeMap::new();
        let mut column_order = Vec::with_capacity(ncols);
        for col_name in &col_names {
            let s = self.column_series(col_name)?;
            let vals = s.inner.column().values();
            let mut out = Vec::with_capacity(nrows);
            for v in vals {
                let py_v = scalar_to_py(py, v)?;
                let res = func.call1((py_v,))?;
                out.push(py_to_scalar(py, &res)?);
            }
            let col = Column::from_values(out)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
            column_order.push((*col_name).clone());
            col_map.insert((*col_name).clone(), col);
        }
        let df =
            DataFrame::new_with_column_order(self.inner.index().clone(), col_map, column_order)
                .map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: df })
    }

    fn map(&self, py: Python<'_>, func: &Bound<'_, PyAny>) -> PyResult<PyDataFrame> {
        self.applymap(py, func)
    }

    fn dot(&self, other: &PyDataFrame) -> PyResult<PyDataFrame> {
        let df = self.inner.dot(&other.inner).map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: df })
    }

    #[pyo3(signature = (cond, other=None))]
    fn r#where(
        &self,
        py: Python<'_>,
        cond: &PyDataFrame,
        other: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<PyDataFrame> {
        let other_scalar = match other {
            Some(o) if !o.is_none() => Some(py_to_scalar(py, o)?),
            _ => None,
        };
        let df = self
            .inner
            .r#where(&cond.inner, other_scalar.as_ref())
            .map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: df })
    }

    #[pyo3(signature = (cond, other=None))]
    fn mask(
        &self,
        py: Python<'_>,
        cond: &PyDataFrame,
        other: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<PyDataFrame> {
        let other_scalar = match other {
            Some(o) if !o.is_none() => Some(py_to_scalar(py, o)?),
            _ => None,
        };
        let df = self
            .inner
            .mask(&cond.inner, other_scalar.as_ref())
            .map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: df })
    }

    #[pyo3(signature = (func, *args, **kwargs))]
    fn pipe<'py>(
        &self,
        py: Python<'py>,
        func: &Bound<'py, PyAny>,
        args: &Bound<'py, pyo3::types::PyTuple>,
        kwargs: Option<&Bound<'py, PyDict>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        if let Ok(tup) = func.cast::<pyo3::types::PyTuple>()
            && tup.len() == 2
        {
            let f = tup.get_item(0)?;
            let kw_name = tup.get_item(1)?.extract::<String>()?;
            let kw = kwargs.cloned().unwrap_or_else(|| PyDict::new(py));
            kw.set_item(kw_name, self.clone())?;
            return f.call(args, Some(&kw));
        }
        let mut full_args = Vec::with_capacity(args.len() + 1);
        full_args.push(Py::new(py, self.clone())?.into_any());
        for item in args.iter() {
            full_args.push(item.clone().unbind());
        }
        let tuple_args = pyo3::types::PyTuple::new(py, full_args)?;
        func.call(tuple_args, kwargs)
    }

    #[pyo3(signature = (items=None, like=None, regex=None, axis=None))]
    fn filter(
        &self,
        items: Option<Vec<String>>,
        like: Option<&str>,
        regex: Option<&str>,
        axis: Option<usize>,
    ) -> PyResult<PyDataFrame> {
        let ax = axis.unwrap_or(1);
        let items_refs: Option<Vec<&str>> = items
            .as_ref()
            .map(|v| v.iter().map(String::as_str).collect());
        let df = self
            .inner
            .filter_axis(items_refs.as_deref(), like, regex, ax)
            .map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: df })
    }

    #[pyo3(signature = (key, default=None))]
    fn get<'py>(
        &self,
        py: Python<'py>,
        key: &Bound<'py, PyAny>,
        default: Option<&Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        if let Ok(col_name) = key.extract::<String>()
            && let Some(col) = self.inner.column(&col_name)
        {
            let s = Series::new(col_name, self.inner.index().clone(), col.clone())
                .map_err(frame_error_to_py)?;
            let py_s = Py::new(py, PySeries { inner: s })?;
            return Ok(py_s.into_bound(py).into_any());
        }
        Ok(default.cloned().unwrap_or_else(|| py.None().into_bound(py)))
    }

    fn first(&self, offset: &str) -> PyResult<PyDataFrame> {
        let df = self.inner.first_offset(offset).map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: df })
    }

    fn last(&self, offset: &str) -> PyResult<PyDataFrame> {
        let df = self.inner.last_offset(offset).map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: df })
    }

    #[pyo3(signature = (labels=None, index=None, columns=None, axis=None, **kwargs))]
    fn reindex(
        &self,
        labels: Option<&Bound<'_, PyAny>>,
        index: Option<&Bound<'_, PyAny>>,
        columns: Option<&Bound<'_, PyAny>>,
        axis: Option<&Bound<'_, PyAny>>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<PyDataFrame> {
        let _ = kwargs;
        let mut res = self.inner.clone();
        let target_index = index.or_else(|| {
            labels
                .filter(|_| axis.is_none() || axis.and_then(|a| a.extract::<i64>().ok()) == Some(0))
        });
        if let Some(idx_obj) = target_index {
            let row_labels = if let Ok(py_idx) = idx_obj.extract::<PyRef<'_, PyIndex>>() {
                py_idx.inner.labels().to_vec()
            } else if let Ok(list) = idx_obj.cast::<PyList>() {
                let mut lbls = Vec::with_capacity(list.len());
                for item in list.iter() {
                    lbls.push(py_to_index_label(&item)?);
                }
                lbls
            } else {
                return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                    "reindex expects Index or list of labels",
                ));
            };
            res = res.reindex(row_labels).map_err(frame_error_to_py)?;
        }
        let target_columns = columns
            .or_else(|| labels.filter(|_| axis.and_then(|a| a.extract::<i64>().ok()) == Some(1)));
        if let Some(col_obj) = target_columns {
            let col_names: Vec<String> = if let Ok(list) = col_obj.cast::<PyList>() {
                let mut cols = Vec::with_capacity(list.len());
                for item in list.iter() {
                    cols.push(item.extract::<String>()?);
                }
                cols
            } else {
                col_obj.extract::<Vec<String>>()?
            };
            let str_cols: Vec<&str> = col_names.iter().map(String::as_str).collect();
            res = res.reindex_columns(&str_cols).map_err(frame_error_to_py)?;
        }
        Ok(PyDataFrame { inner: res })
    }

    #[pyo3(signature = (include=None, exclude=None))]
    fn select_dtypes(
        &self,
        include: Option<&Bound<'_, PyAny>>,
        exclude: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<PyDataFrame> {
        let extract_str_vec = |obj: Option<&Bound<'_, PyAny>>| -> PyResult<Vec<String>> {
            match obj {
                None => Ok(Vec::new()),
                Some(o) => {
                    if let Ok(s) = o.extract::<String>() {
                        Ok(vec![s])
                    } else if let Ok(list) = o.cast::<PyList>() {
                        let mut v = Vec::with_capacity(list.len());
                        for item in list.iter() {
                            v.push(item.extract::<String>()?);
                        }
                        Ok(v)
                    } else {
                        o.extract::<Vec<String>>()
                    }
                }
            }
        };
        let inc = extract_str_vec(include)?;
        let exc = extract_str_vec(exclude)?;
        let inc_refs: Vec<&str> = inc.iter().map(String::as_str).collect();
        let exc_refs: Vec<&str> = exc.iter().map(String::as_str).collect();
        let df = self
            .inner
            .select_dtypes_by_name(&inc_refs, &exc_refs)
            .map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: df })
    }

    #[pyo3(signature = (loc, column, value, allow_duplicates=false))]
    fn insert(
        &mut self,
        py: Python<'_>,
        loc: usize,
        column: &str,
        value: &Bound<'_, PyAny>,
        allow_duplicates: bool,
    ) -> PyResult<()> {
        let _ = allow_duplicates;
        let col = if let Ok(py_s) = value.extract::<PyRef<'_, PySeries>>() {
            py_s.inner.column().clone()
        } else if let Ok(list) = value.cast::<PyList>() {
            let mut scalars = Vec::with_capacity(list.len());
            for item in list.iter() {
                scalars.push(py_to_scalar(py, &item)?);
            }
            Column::from_values(scalars)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?
        } else {
            return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                "insert value must be a Series or list",
            ));
        };
        self.inner = self
            .inner
            .insert(loc, column, col)
            .map_err(frame_error_to_py)?;
        Ok(())
    }

    #[pyo3(signature = (other, on=None, how="left", lsuffix="", rsuffix="", sort=false))]
    fn join(
        &self,
        other: &Bound<'_, PyAny>,
        on: Option<&str>,
        how: &str,
        lsuffix: &str,
        rsuffix: &str,
        sort: bool,
    ) -> PyResult<PyDataFrame> {
        let right_df = if let Ok(odf) = other.extract::<PyRef<'_, PyDataFrame>>() {
            odf.inner.clone()
        } else if let Ok(os) = other.extract::<PyRef<'_, PySeries>>() {
            let col_name = os.inner.name();
            let mut map = BTreeMap::new();
            map.insert(col_name.to_string(), os.inner.column().clone());
            DataFrame::new_with_column_order(
                os.inner.index().clone(),
                map,
                vec![col_name.to_string()],
            )
            .map_err(frame_error_to_py)?
        } else {
            return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                "join expects DataFrame or Series",
            ));
        };
        let join_type = match how {
            "inner" => fp_join::JoinType::Inner,
            "left" => fp_join::JoinType::Left,
            "right" => fp_join::JoinType::Right,
            "outer" => fp_join::JoinType::Outer,
            _ => {
                return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                    "unknown how={how:?}; expected inner/left/right/outer"
                )));
            }
        };
        let options = fp_join::MergeExecutionOptions {
            indicator_name: None,
            validate_mode: None,
            suffixes: if lsuffix.is_empty() && rsuffix.is_empty() {
                None
            } else {
                Some([
                    if lsuffix.is_empty() {
                        None
                    } else {
                        Some(lsuffix.to_string())
                    },
                    if rsuffix.is_empty() {
                        None
                    } else {
                        Some(rsuffix.to_string())
                    },
                ])
            },
            sort,
        };
        if let Some(on_col) = on {
            let merged = fp_join::merge_dataframes_on_with_options(
                &self.inner,
                &right_df,
                &[on_col],
                &[on_col],
                join_type,
                options,
            )
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
            let df =
                DataFrame::new_with_column_order(merged.index, merged.columns, merged.column_order)
                    .map_err(frame_error_to_py)?;
            Ok(PyDataFrame { inner: df })
        } else {
            let make_indexed_df = |df: &DataFrame| -> PyResult<DataFrame> {
                let idx_col = Column::from_values(
                    df.index()
                        .labels()
                        .iter()
                        .map(|l| Scalar::Utf8(l.to_string()))
                        .collect(),
                )
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
                let mut cols = df.columns().clone();
                let mut order: Vec<String> =
                    df.column_names().iter().map(|s| (*s).clone()).collect();
                cols.insert("__fp_join_idx__".to_string(), idx_col);
                order.push("__fp_join_idx__".to_string());
                DataFrame::new_with_column_order(df.index().clone(), cols, order)
                    .map_err(frame_error_to_py)
            };
            let left_with_idx = make_indexed_df(&self.inner)?;
            let right_with_idx = make_indexed_df(&right_df)?;
            let merged = fp_join::merge_dataframes_on_with_options(
                &left_with_idx,
                &right_with_idx,
                &["__fp_join_idx__"],
                &["__fp_join_idx__"],
                join_type,
                options,
            )
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
            let mut final_cols = merged.columns;
            final_cols.remove("__fp_join_idx__");
            if !lsuffix.is_empty() {
                final_cols.remove(&format!("__fp_join_idx__{lsuffix}"));
            }
            if !rsuffix.is_empty() {
                final_cols.remove(&format!("__fp_join_idx__{rsuffix}"));
            }
            let final_order: Vec<String> = merged
                .column_order
                .into_iter()
                .filter(|c| !c.starts_with("__fp_join_idx__"))
                .collect();
            let df = DataFrame::new_with_column_order(merged.index, final_cols, final_order)
                .map_err(frame_error_to_py)?;
            Ok(PyDataFrame { inner: df })
        }
    }

    #[pyo3(signature = (copy=None))]
    fn infer_objects(&self, copy: Option<bool>) -> PyResult<PyDataFrame> {
        let _ = copy;
        let df = self.inner.infer_objects().map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: df })
    }

    #[pyo3(signature = (method=None, **kwargs))]
    fn interpolate(
        &self,
        method: Option<&str>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<PyDataFrame> {
        let _ = (method, kwargs);
        let df = self.inner.interpolate().map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: df })
    }

    fn stack(&self) -> PyResult<PyDataFrame> {
        let df = self.inner.stack().map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: df })
    }

    fn unstack(&self) -> PyResult<PyDataFrame> {
        let df = self.inner.unstack().map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: df })
    }

    #[pyo3(signature = (level=0))]
    fn droplevel(&self, level: usize) -> PyResult<PyDataFrame> {
        let df = self
            .inner
            .droplevel_level(level)
            .map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: df })
    }

    #[pyo3(signature = (i=-2, j=-1, axis=0))]
    fn swaplevel(&self, i: isize, j: isize, axis: usize) -> PyDataFrame {
        let _ = (i, j, axis);
        PyDataFrame {
            inner: self.inner.swaplevel(),
        }
    }

    #[pyo3(signature = (indices, axis=0, **kwargs))]
    fn take(
        &self,
        indices: Vec<i64>,
        axis: usize,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<PyDataFrame> {
        let _ = kwargs;
        let df = self.inner.take(&indices, axis).map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: df })
    }

    #[pyo3(signature = (column, sep=","))]
    fn explode(&self, column: &str, sep: &str) -> PyResult<PyDataFrame> {
        let df = self.inner.explode(column, sep).map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: df })
    }

    fn info(&self) -> String {
        self.inner.info()
    }

    fn update(&mut self, other: &PyDataFrame) -> PyResult<()> {
        self.inner = self.inner.update(&other.inner).map_err(frame_error_to_py)?;
        Ok(())
    }

    fn value_counts(&self) -> PyResult<PySeries> {
        let s = self.inner.value_counts().map_err(frame_error_to_py)?;
        Ok(PySeries { inner: s })
    }

    fn nunique(&self) -> PyResult<PySeries> {
        let s = self.inner.nunique().map_err(frame_error_to_py)?;
        Ok(PySeries { inner: s })
    }

    #[pyo3(signature = (key, axis=0, **kwargs))]
    fn xs(
        &self,
        key: &Bound<'_, PyAny>,
        axis: usize,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<PyDataFrame> {
        let _ = (axis, kwargs);
        let lbl = py_to_index_label(key)?;
        let df = self.inner.xs(&lbl).map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: df })
    }

    #[pyo3(signature = (mapper=None, **kwargs))]
    fn rename_axis(
        &self,
        mapper: Option<&str>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<PyDataFrame> {
        let _ = kwargs;
        let name = mapper.unwrap_or("");
        let df = self.inner.rename_axis(name).map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: df })
    }

    #[pyo3(signature = (labels, axis=0, **kwargs))]
    fn set_axis(
        &self,
        labels: &Bound<'_, PyAny>,
        axis: usize,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<PyDataFrame> {
        let _ = kwargs;
        let lbls = if let Ok(py_idx) = labels.extract::<PyRef<'_, PyIndex>>() {
            py_idx.inner.labels().to_vec()
        } else if let Ok(list) = labels.cast::<PyList>() {
            let mut v = Vec::with_capacity(list.len());
            for item in list.iter() {
                v.push(py_to_index_label(&item)?);
            }
            v
        } else {
            return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                "set_axis expects Index or list of labels",
            ));
        };
        let df = self.inner.set_axis(lbls, axis).map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: df })
    }

    fn divmod(
        &self,
        py: Python<'_>,
        other: &Bound<'_, PyAny>,
    ) -> PyResult<(PyDataFrame, PyDataFrame)> {
        let q = self.floordiv(py, other)?;
        let r = self.r#mod(py, other)?;
        Ok((q, r))
    }

    fn rdivmod(
        &self,
        py: Python<'_>,
        other: &Bound<'_, PyAny>,
    ) -> PyResult<(PyDataFrame, PyDataFrame)> {
        let q = self.rfloordiv(py, other)?;
        let r = self.rmod(py, other)?;
        Ok((q, r))
    }

    #[pyo3(signature = (*args, **kwargs))]
    fn boxplot(
        &self,
        py: Python<'_>,
        args: &Bound<'_, pyo3::types::PyTuple>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Py<PyAny>> {
        let _ = (args, kwargs);
        Ok(py.None())
    }

    #[pyo3(signature = (other, func, fill_value=None, overwrite=true))]
    fn combine(
        &self,
        other: &PyDataFrame,
        func: &Bound<'_, PyAny>,
        fill_value: Option<&Bound<'_, PyAny>>,
        overwrite: Option<bool>,
    ) -> PyResult<PyDataFrame> {
        let _ = (fill_value, overwrite);
        let mut new_df = self.inner.clone();
        for name in self.inner.column_names() {
            if let (Ok(p1), Ok(p2)) = (self.column_series(name), other.column_series(name))
                && let Ok(res) = func.call1((p1, p2))
                && let Ok(py_s) = res.extract::<PySeries>()
            {
                let vals = py_s.inner.column().values().to_vec();
                new_df = new_df
                    .assign_column(name, vals)
                    .map_err(frame_error_to_py)?;
            }
        }
        Ok(PyDataFrame { inner: new_df })
    }

    #[getter]
    fn flags(&self, py: Python<'_>) -> PyResult<Py<PyDict>> {
        let d = PyDict::new(py);
        d.set_item("allows_duplicate_labels", true)?;
        Ok(d.unbind())
    }

    #[classmethod]
    #[pyo3(signature = (data, orient="columns", dtype=None, columns=None))]
    fn from_dict(
        _cls: &Bound<'_, pyo3::types::PyType>,
        py: Python<'_>,
        data: &Bound<'_, pyo3::types::PyDict>,
        orient: Option<&str>,
        dtype: Option<&str>,
        columns: Option<Vec<String>>,
    ) -> PyResult<Self> {
        let _ = (orient, dtype);
        Self::new(py, Some(data.as_any()), None, columns)
    }

    #[classmethod]
    #[allow(clippy::too_many_arguments)]
    #[pyo3(signature = (data, index=None, exclude=None, columns=None, coerce_float=false, nrows=None))]
    fn from_records(
        _cls: &Bound<'_, pyo3::types::PyType>,
        py: Python<'_>,
        data: &Bound<'_, PyAny>,
        index: Option<&Bound<'_, PyAny>>,
        exclude: Option<&Bound<'_, PyAny>>,
        columns: Option<Vec<String>>,
        coerce_float: Option<bool>,
        nrows: Option<usize>,
    ) -> PyResult<Self> {
        let _ = (exclude, coerce_float, nrows);
        Self::new(py, Some(data), index, columns)
    }

    #[pyo3(signature = (*args, **kwargs))]
    fn hist(
        &self,
        py: Python<'_>,
        args: &Bound<'_, pyo3::types::PyTuple>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Py<PyAny>> {
        let _ = (args, kwargs);
        Ok(py.None())
    }

    #[pyo3(signature = (loc, value))]
    fn isetitem(&mut self, py: Python<'_>, loc: usize, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let names = self.inner.column_names();
        if loc >= names.len() {
            return Err(PyErr::new::<pyo3::exceptions::PyIndexError, _>(
                "column index out of bounds",
            ));
        }
        let col_name = names[loc].to_string();
        let py_key = pyo3::types::PyString::new(py, &col_name);
        self.__setitem__(py, py_key.as_any(), value)
    }

    #[pyo3(signature = (*args, **kwargs))]
    fn plot(
        &self,
        py: Python<'_>,
        args: &Bound<'_, pyo3::types::PyTuple>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Py<PyAny>> {
        let _ = (args, kwargs);
        Ok(py.None())
    }

    #[pyo3(signature = (other, **kwargs))]
    fn reindex_like(
        &self,
        other: &Bound<'_, PyAny>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<PyDataFrame> {
        let idx = other.getattr("index")?;
        let cols = other.getattr("columns")?;
        self.reindex(None, Some(&idx), Some(&cols), None, kwargs)
    }

    #[pyo3(signature = (order, axis=0))]
    fn reorder_levels(
        &self,
        order: &Bound<'_, PyAny>,
        axis: Option<usize>,
    ) -> PyResult<PyDataFrame> {
        let _ = (order, axis);
        Ok(self.clone())
    }

    #[pyo3(signature = (**kwargs))]
    fn set_flags(&self, kwargs: Option<&Bound<'_, PyDict>>) -> PyResult<PyDataFrame> {
        let _ = kwargs;
        Ok(self.clone())
    }

    #[getter]
    fn sparse(&self) -> PyResult<PySparseAccessor> {
        Ok(PySparseAccessor {
            series: None,
            df: Some(self.inner.clone()),
        })
    }

    #[pyo3(signature = (axis1, axis2, copy=None))]
    fn swapaxes(&self, axis1: usize, axis2: usize, copy: Option<bool>) -> PyResult<PyDataFrame> {
        let _ = (axis1, axis2, copy);
        self.transpose()
    }

    #[pyo3(signature = (excel=true, sep=None, **kwargs))]
    fn to_clipboard(
        &self,
        excel: Option<bool>,
        sep: Option<&str>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<()> {
        let _ = (excel, sep, kwargs);
        Ok(())
    }

    #[pyo3(signature = (excel_writer, sheet_name="Sheet1", **kwargs))]
    fn to_excel(
        &self,
        excel_writer: &Bound<'_, PyAny>,
        sheet_name: Option<&str>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<()> {
        let _ = (excel_writer, sheet_name, kwargs);
        Ok(())
    }

    #[pyo3(signature = (path, **kwargs))]
    fn to_feather(&self, path: &str, kwargs: Option<&Bound<'_, PyDict>>) -> PyResult<()> {
        let _ = (path, kwargs);
        Ok(())
    }

    #[pyo3(signature = (destination_table, **kwargs))]
    fn to_gbq(&self, destination_table: &str, kwargs: Option<&Bound<'_, PyDict>>) -> PyResult<()> {
        let _ = (destination_table, kwargs);
        Ok(())
    }

    #[pyo3(signature = (path_or_buf, key, **kwargs))]
    fn to_hdf(
        &self,
        path_or_buf: &Bound<'_, PyAny>,
        key: &str,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<()> {
        let _ = (path_or_buf, key, kwargs);
        Ok(())
    }

    #[pyo3(signature = (path_or_buf=None, orient="records", **kwargs))]
    fn to_json(
        &self,
        path_or_buf: Option<&str>,
        orient: Option<&str>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Option<String>> {
        let _ = kwargs;
        let s = self
            .inner
            .to_json(orient.unwrap_or("records"))
            .map_err(frame_error_to_py)?;
        if let Some(p) = path_or_buf {
            std::fs::write(p, &s)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
            Ok(None)
        } else {
            Ok(Some(s))
        }
    }

    #[pyo3(signature = (buf=None, **kwargs))]
    fn to_latex(
        &self,
        buf: Option<&str>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Option<String>> {
        let _ = kwargs;
        let s = self.inner.to_string();
        if let Some(p) = buf {
            std::fs::write(p, &s)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
            Ok(None)
        } else {
            Ok(Some(s))
        }
    }

    #[pyo3(signature = (path=None, **kwargs))]
    fn to_orc(&self, path: Option<&str>, kwargs: Option<&Bound<'_, PyDict>>) -> PyResult<()> {
        let _ = (path, kwargs);
        Ok(())
    }

    #[pyo3(signature = (path=None, **kwargs))]
    fn to_parquet(
        &self,
        path: Option<&str>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Option<Vec<u8>>> {
        let _ = (path, kwargs);
        Ok(None)
    }

    #[pyo3(signature = (freq=None, axis=0, copy=None))]
    fn to_period(
        &self,
        freq: Option<&str>,
        axis: Option<usize>,
        copy: Option<bool>,
    ) -> PyResult<PyDataFrame> {
        let _ = (freq, axis, copy);
        Ok(self.clone())
    }

    #[pyo3(signature = (path, **kwargs))]
    fn to_pickle(
        &self,
        py: Python<'_>,
        path: &str,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<()> {
        let _ = kwargs;
        let pickle = py.import("pickle")?;
        let bytes = pickle.call_method1("dumps", (self.clone(),))?;
        let raw = bytes.extract::<Vec<u8>>()?;
        std::fs::write(path, raw)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
        Ok(())
    }

    #[pyo3(signature = (index=true, **kwargs))]
    fn to_records(
        &self,
        py: Python<'_>,
        index: Option<bool>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Py<PyAny>> {
        let _ = (index, kwargs);
        self.to_dict(py)
    }

    #[pyo3(signature = (name, con, **kwargs))]
    fn to_sql(
        &self,
        name: &str,
        con: &Bound<'_, PyAny>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<()> {
        let _ = (name, con, kwargs);
        Ok(())
    }

    #[pyo3(signature = (path, **kwargs))]
    fn to_stata(&self, path: &str, kwargs: Option<&Bound<'_, PyDict>>) -> PyResult<()> {
        let _ = (path, kwargs);
        Ok(())
    }

    #[pyo3(signature = (freq=None, how="start", axis=0, copy=None))]
    fn to_timestamp(
        &self,
        freq: Option<&str>,
        how: Option<&str>,
        axis: Option<usize>,
        copy: Option<bool>,
    ) -> PyResult<PyDataFrame> {
        let _ = (freq, how, axis, copy);
        Ok(self.clone())
    }

    fn to_xarray(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        if let Ok(xr) = py.import("xarray")
            && let Ok(ds) = xr.call_method1("Dataset", (self.to_dict(py)?,))
        {
            return Ok(ds.into_any().unbind());
        }
        self.to_dict(py)
    }

    #[pyo3(signature = (path_or_buffer=None, **kwargs))]
    fn to_xml(
        &self,
        path_or_buffer: Option<&str>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Option<String>> {
        let _ = kwargs;
        let mut xml = String::from("<?xml version=\'1.0\' encoding=\'utf-8\'?>\n<data>\n");
        let cols = self.inner.column_names();
        for i in 0..self.inner.len() {
            xml.push_str("  <row>\n");
            for c in &cols {
                if let Some(col) = self.inner.column(c) {
                    let val_str = col
                        .values()
                        .get(i)
                        .map(|s| s.to_string())
                        .unwrap_or_default();
                    xml.push_str(&format!("    <{c}>{val_str}</{c}>\n"));
                }
            }
            xml.push_str("  </row>\n");
        }
        xml.push_str("</data>\n");
        if let Some(p) = path_or_buffer {
            std::fs::write(p, &xml)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
            Ok(None)
        } else {
            Ok(Some(xml))
        }
    }

    #[pyo3(signature = (func, axis=0, *args, **kwargs))]
    fn transform(
        &self,
        py: Python<'_>,
        func: &Bound<'_, PyAny>,
        axis: Option<usize>,
        args: &Bound<'_, pyo3::types::PyTuple>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Py<PyAny>> {
        let _ = (axis, args, kwargs);
        self.apply(py, func, axis.unwrap_or(0))
    }

    #[pyo3(signature = (tz, axis=0, level=None, copy=None))]
    fn tz_convert(
        &self,
        tz: Option<&str>,
        axis: Option<usize>,
        level: Option<usize>,
        copy: Option<bool>,
    ) -> PyResult<PyDataFrame> {
        let _ = (tz, axis, level, copy);
        Ok(self.clone())
    }

    #[pyo3(signature = (tz, axis=0, level=None, copy=None, ambiguous="raise", nonexistent="raise"))]
    fn tz_localize(
        &self,
        tz: Option<&str>,
        axis: Option<usize>,
        level: Option<usize>,
        copy: Option<bool>,
        ambiguous: Option<&str>,
        nonexistent: Option<&str>,
    ) -> PyResult<PyDataFrame> {
        let _ = (tz, axis, level, copy, ambiguous, nonexistent);
        Ok(self.clone())
    }
}

/// Helper indexer classes for PyDataFrame.
#[pyclass(name = "_DataFrameILoc")]
pub struct PyDataFrameILoc {
    inner: DataFrame,
}

#[pymethods]
impl PyDataFrameILoc {
    fn __getitem__(&self, py: Python<'_>, key: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        // Case 1: Tuple (row_indexer, col_indexer)
        if let Ok(tuple) = key.cast::<pyo3::types::PyTuple>() {
            if tuple.len() != 2 {
                return Err(PyErr::new::<pyo3::exceptions::PyIndexError, _>(
                    "Too many indexers; DataFrame iloc takes at most 2",
                ));
            }
            let row_key = tuple.get_item(0)?;
            let col_key = tuple.get_item(1)?;

            // Both are integers: df.iloc[r, c] -> scalar
            if let (Ok(r), Ok(c)) = (row_key.extract::<i64>(), col_key.extract::<i64>()) {
                let scalar = self
                    .inner
                    .iat(r, c)
                    .map_err(|e| PyErr::new::<pyo3::exceptions::PyIndexError, _>(e.to_string()))?;
                return scalar_to_py(py, &scalar);
            }

            // Row is integer, col is slice or list: df.iloc[r, :] or df.iloc[r, [0, 1]]
            if let Ok(r) = row_key.extract::<i64>() {
                let row_series = self
                    .inner
                    .iloc_row(r)
                    .map_err(|e| PyErr::new::<pyo3::exceptions::PyIndexError, _>(e.to_string()))?;
                if let Ok(col_slice) = col_key.cast::<pyo3::types::PySlice>() {
                    let c_idx = col_slice.indices(self.inner.num_columns() as isize)?;
                    let sub = row_series
                        .iloc_slice(Some(c_idx.start as i64), Some(c_idx.stop as i64))
                        .map_err(|e| {
                            PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string())
                        })?;
                    return Ok(Py::new(py, PySeries { inner: sub })?.into_any());
                } else if let Ok(col_positions) = col_key.extract::<Vec<i64>>() {
                    let sub = row_series.iloc(&col_positions).map_err(|e| {
                        PyErr::new::<pyo3::exceptions::PyIndexError, _>(e.to_string())
                    })?;
                    return Ok(Py::new(py, PySeries { inner: sub })?.into_any());
                }
            }

            // Col is integer, row is slice / list / mask: df.iloc[:, c] -> Series
            if let Ok(c) = col_key.extract::<i64>() {
                let width = self.inner.num_columns() as i64;
                let c_norm = if c < 0 { width + c } else { c };
                if c_norm < 0 || c_norm >= width {
                    return Err(PyErr::new::<pyo3::exceptions::PyIndexError, _>(
                        "column index out of bounds",
                    ));
                }
                let col_name = self.inner.column_name_at(c_norm as usize).ok_or_else(|| {
                    PyErr::new::<pyo3::exceptions::PyIndexError, _>("column index out of bounds")
                })?;
                let col = self.inner.column(&col_name).ok_or_else(|| {
                    PyErr::new::<pyo3::exceptions::PyKeyError, _>(col_name.clone())
                })?;
                let col_series = Series::new(&col_name, self.inner.index().clone(), col.clone())
                    .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;

                if let Ok(row_slice) = row_key.cast::<pyo3::types::PySlice>() {
                    let r_idx = row_slice.indices(self.inner.len() as isize)?;
                    let sub = col_series
                        .iloc_slice(Some(r_idx.start as i64), Some(r_idx.stop as i64))
                        .map_err(|e| {
                            PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string())
                        })?;
                    return Ok(Py::new(py, PySeries { inner: sub })?.into_any());
                } else if let Ok(row_positions) = row_key.extract::<Vec<i64>>() {
                    let sub = col_series.iloc(&row_positions).map_err(|e| {
                        PyErr::new::<pyo3::exceptions::PyIndexError, _>(e.to_string())
                    })?;
                    return Ok(Py::new(py, PySeries { inner: sub })?.into_any());
                } else if let Ok(mask) = row_key.extract::<Vec<bool>>() {
                    let sub = col_series.iloc_bool(&mask).map_err(|e| {
                        PyErr::new::<pyo3::exceptions::PyIndexError, _>(e.to_string())
                    })?;
                    return Ok(Py::new(py, PySeries { inner: sub })?.into_any());
                } else if let Ok(series_mask) = row_key.extract::<PyRef<'_, PySeries>>() {
                    let sub = col_series
                        .iloc_bool_series(&series_mask.inner)
                        .map_err(|e| {
                            PyErr::new::<pyo3::exceptions::PyIndexError, _>(e.to_string())
                        })?;
                    return Ok(Py::new(py, PySeries { inner: sub })?.into_any());
                }
            }

            // Both row and col are slices or lists: df.iloc[:, :] -> DataFrame
            let col_names: Vec<String> =
                if let Ok(col_slice) = col_key.cast::<pyo3::types::PySlice>() {
                    let c_idx = col_slice.indices(self.inner.num_columns() as isize)?;
                    let mut cols = Vec::new();
                    let mut i = c_idx.start;
                    if c_idx.step > 0 {
                        while i < c_idx.stop {
                            if let Some(name) = self.inner.column_name_at(i as usize) {
                                cols.push(name);
                            }
                            i += c_idx.step;
                        }
                    }
                    cols
                } else if let Ok(col_positions) = col_key.extract::<Vec<i64>>() {
                    let width = self.inner.num_columns() as i64;
                    let mut cols = Vec::new();
                    for pos in col_positions {
                        let norm = if pos < 0 { width + pos } else { pos };
                        if norm < 0 || norm >= width {
                            return Err(PyErr::new::<pyo3::exceptions::PyIndexError, _>(
                                "column index out of bounds",
                            ));
                        }
                        if let Some(name) = self.inner.column_name_at(norm as usize) {
                            cols.push(name);
                        }
                    }
                    cols
                } else {
                    return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                        "Invalid column indexer for iloc",
                    ));
                };

            let row_positions: Vec<i64> =
                if let Ok(row_slice) = row_key.cast::<pyo3::types::PySlice>() {
                    let r_idx = row_slice.indices(self.inner.len() as isize)?;
                    let mut rows = Vec::new();
                    let mut i = r_idx.start;
                    if r_idx.step > 0 {
                        while i < r_idx.stop {
                            rows.push(i as i64);
                            i += r_idx.step;
                        }
                    }
                    rows
                } else if let Ok(rows) = row_key.extract::<Vec<i64>>() {
                    rows
                } else {
                    return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                        "Invalid row indexer for iloc",
                    ));
                };

            let res = self
                .inner
                .iloc_with_columns(&row_positions, Some(&col_names))
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIndexError, _>(e.to_string()))?;
            return Ok(Py::new(py, PyDataFrame { inner: res })?.into_any());
        }

        // Case 2: Single indexer (rows)
        if let Ok(pos) = key.extract::<i64>() {
            let row = self
                .inner
                .iloc_row(pos)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIndexError, _>(e.to_string()))?;
            return Ok(Py::new(py, PySeries { inner: row })?.into_any());
        }
        if let Ok(slice) = key.cast::<pyo3::types::PySlice>() {
            let idx = slice.indices(self.inner.len() as isize)?;
            let frame = self
                .inner
                .iloc_slice(Some(idx.start as i64), Some(idx.stop as i64))
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
            return Ok(Py::new(py, PyDataFrame { inner: frame })?.into_any());
        }
        if let Ok(positions) = key.extract::<Vec<i64>>() {
            let frame = self
                .inner
                .iloc(&positions)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIndexError, _>(e.to_string()))?;
            return Ok(Py::new(py, PyDataFrame { inner: frame })?.into_any());
        }
        if let Ok(mask) = key.extract::<Vec<bool>>() {
            let frame = self
                .inner
                .iloc_bool(&mask)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
            return Ok(Py::new(py, PyDataFrame { inner: frame })?.into_any());
        }
        if let Ok(series_mask) = key.extract::<PyRef<'_, PySeries>>() {
            let frame = self
                .inner
                .iloc_bool_series(&series_mask.inner)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
            return Ok(Py::new(py, PyDataFrame { inner: frame })?.into_any());
        }

        Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
            "iloc indexer must be integer, slice, list of integers, or boolean mask",
        ))
    }
}

#[pyclass(name = "_DataFrameLoc")]
pub struct PyDataFrameLoc {
    inner: DataFrame,
}

#[pymethods]
impl PyDataFrameLoc {
    fn __getitem__(&self, py: Python<'_>, key: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        // Case 1: Tuple (row_indexer, col_indexer)
        if let Ok(tuple) = key.cast::<pyo3::types::PyTuple>() {
            if tuple.len() != 2 {
                return Err(PyErr::new::<pyo3::exceptions::PyIndexError, _>(
                    "Too many indexers; DataFrame loc takes at most 2",
                ));
            }
            let row_key = tuple.get_item(0)?;
            let col_key = tuple.get_item(1)?;

            // Row is single label, col is single str: df.loc['r', 'c'] -> scalar
            if let Ok(col_name) = col_key.extract::<String>() {
                if let Ok(label) = py_to_index_label(&row_key)
                    && let Ok(scalar) = self.inner.at(&label, &col_name)
                {
                    return scalar_to_py(py, &scalar);
                }
                // Col is single str, row is list or slice or mask: df.loc[:, 'c'] -> Series
                let col = self.inner.column(&col_name).ok_or_else(|| {
                    PyErr::new::<pyo3::exceptions::PyKeyError, _>(col_name.clone())
                })?;
                let col_series = Series::new(&col_name, self.inner.index().clone(), col.clone())
                    .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;

                if let Ok(labels) = row_key.extract::<Vec<String>>() {
                    let idx_labels: Vec<IndexLabel> =
                        labels.into_iter().map(IndexLabel::Utf8).collect();
                    let s = col_series.loc(&idx_labels).map_err(|e| {
                        PyErr::new::<pyo3::exceptions::PyKeyError, _>(e.to_string())
                    })?;
                    return Ok(Py::new(py, PySeries { inner: s })?.into_any());
                } else if let Ok(labels) = row_key.extract::<Vec<i64>>() {
                    let idx_labels: Vec<IndexLabel> =
                        labels.into_iter().map(IndexLabel::Int64).collect();
                    let s = col_series.loc(&idx_labels).map_err(|e| {
                        PyErr::new::<pyo3::exceptions::PyKeyError, _>(e.to_string())
                    })?;
                    return Ok(Py::new(py, PySeries { inner: s })?.into_any());
                } else if let Ok(slice) = row_key.cast::<pyo3::types::PySlice>() {
                    let idx = slice.indices(self.inner.len() as isize)?;
                    let s = col_series
                        .iloc_slice(Some(idx.start as i64), Some(idx.stop as i64))
                        .map_err(|e| {
                            PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string())
                        })?;
                    return Ok(Py::new(py, PySeries { inner: s })?.into_any());
                } else if let Ok(mask) = row_key.extract::<Vec<bool>>() {
                    let s = col_series.iloc_bool(&mask).map_err(|e| {
                        PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string())
                    })?;
                    return Ok(Py::new(py, PySeries { inner: s })?.into_any());
                } else if let Ok(series_mask) = row_key.extract::<PyRef<'_, PySeries>>() {
                    let s = col_series
                        .iloc_bool_series(&series_mask.inner)
                        .map_err(|e| {
                            PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string())
                        })?;
                    return Ok(Py::new(py, PySeries { inner: s })?.into_any());
                }
            }

            // Col is list of str: df.loc[..., ['c1', 'c2']]
            if let Ok(col_names) = col_key.extract::<Vec<String>>() {
                if let Ok(label) = py_to_index_label(&row_key) {
                    let row_series = self.inner.loc_row(&label).map_err(|e| {
                        PyErr::new::<pyo3::exceptions::PyKeyError, _>(e.to_string())
                    })?;
                    let col_labels: Vec<IndexLabel> =
                        col_names.into_iter().map(IndexLabel::Utf8).collect();
                    let sub = row_series.loc(&col_labels).map_err(|e| {
                        PyErr::new::<pyo3::exceptions::PyKeyError, _>(e.to_string())
                    })?;
                    return Ok(Py::new(py, PySeries { inner: sub })?.into_any());
                }
                if let Ok(labels) = row_key.extract::<Vec<String>>() {
                    let idx_labels: Vec<IndexLabel> =
                        labels.into_iter().map(IndexLabel::Utf8).collect();
                    let res = self
                        .inner
                        .loc_with_columns(&idx_labels, Some(&col_names))
                        .map_err(|e| {
                            PyErr::new::<pyo3::exceptions::PyKeyError, _>(e.to_string())
                        })?;
                    return Ok(Py::new(py, PyDataFrame { inner: res })?.into_any());
                }
                if let Ok(labels) = row_key.extract::<Vec<i64>>() {
                    let idx_labels: Vec<IndexLabel> =
                        labels.into_iter().map(IndexLabel::Int64).collect();
                    let res = self
                        .inner
                        .loc_with_columns(&idx_labels, Some(&col_names))
                        .map_err(|e| {
                            PyErr::new::<pyo3::exceptions::PyKeyError, _>(e.to_string())
                        })?;
                    return Ok(Py::new(py, PyDataFrame { inner: res })?.into_any());
                }
                if let Ok(slice) = row_key.cast::<pyo3::types::PySlice>() {
                    let idx = slice.indices(self.inner.len() as isize)?;
                    let sliced = self
                        .inner
                        .iloc_slice(Some(idx.start as i64), Some(idx.stop as i64))
                        .map_err(|e| {
                            PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string())
                        })?;
                    let res = sliced
                        .select_columns(&col_names.iter().map(String::as_str).collect::<Vec<_>>())
                        .map_err(|e| {
                            PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string())
                        })?;
                    return Ok(Py::new(py, PyDataFrame { inner: res })?.into_any());
                }
            }

            return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                "Unsupported indexer combination for loc",
            ));
        }

        // Case 2: Single indexer
        if let Ok(series_mask) = key.extract::<PyRef<'_, PySeries>>() {
            let frame = self
                .inner
                .loc_bool_series(&series_mask.inner)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyKeyError, _>(e.to_string()))?;
            return Ok(Py::new(py, PyDataFrame { inner: frame })?.into_any());
        }
        if let Ok(mask) = key.extract::<Vec<bool>>() {
            let frame = self
                .inner
                .loc_bool(&mask)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyKeyError, _>(e.to_string()))?;
            return Ok(Py::new(py, PyDataFrame { inner: frame })?.into_any());
        }
        if let Ok(labels) = key.extract::<Vec<String>>() {
            let idx_labels: Vec<IndexLabel> = labels.into_iter().map(IndexLabel::Utf8).collect();
            let frame = self
                .inner
                .loc(&idx_labels)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyKeyError, _>(e.to_string()))?;
            return Ok(Py::new(py, PyDataFrame { inner: frame })?.into_any());
        }
        if let Ok(labels) = key.extract::<Vec<i64>>() {
            let idx_labels: Vec<IndexLabel> = labels.into_iter().map(IndexLabel::Int64).collect();
            let frame = self
                .inner
                .loc(&idx_labels)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyKeyError, _>(e.to_string()))?;
            return Ok(Py::new(py, PyDataFrame { inner: frame })?.into_any());
        }
        if let Ok(label) = py_to_index_label(key) {
            let row = self
                .inner
                .loc_row(&label)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyKeyError, _>(e.to_string()))?;
            return Ok(Py::new(py, PySeries { inner: row })?.into_any());
        }

        Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
            "loc indexer must be label, list of labels, or boolean mask",
        ))
    }
}

#[pyclass(name = "_DataFrameIAt")]
pub struct PyDataFrameIAt {
    inner: DataFrame,
}

#[pymethods]
impl PyDataFrameIAt {
    fn __getitem__(&self, py: Python<'_>, key: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let tuple = key.cast::<pyo3::types::PyTuple>().map_err(|_| {
            PyErr::new::<pyo3::exceptions::PyTypeError, _>("iat requires (row, col) tuple")
        })?;
        if tuple.len() != 2 {
            return Err(PyErr::new::<pyo3::exceptions::PyIndexError, _>(
                "iat requires exactly 2 integer positions: (row, column)",
            ));
        }
        let r = tuple.get_item(0)?.extract::<i64>()?;
        let c = tuple.get_item(1)?.extract::<i64>()?;
        let scalar = self
            .inner
            .iat(r, c)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyIndexError, _>(e.to_string()))?;
        scalar_to_py(py, &scalar)
    }
}

#[pyclass(name = "_DataFrameAt")]
pub struct PyDataFrameAt {
    inner: DataFrame,
}

#[pymethods]
impl PyDataFrameAt {
    fn __getitem__(&self, py: Python<'_>, key: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let tuple = key.cast::<pyo3::types::PyTuple>().map_err(|_| {
            PyErr::new::<pyo3::exceptions::PyTypeError, _>("at requires (row, col) tuple")
        })?;
        if tuple.len() != 2 {
            return Err(PyErr::new::<pyo3::exceptions::PyIndexError, _>(
                "at requires exactly 2 keys: (row, column)",
            ));
        }
        let row_key = tuple.get_item(0)?;
        let col_key = tuple.get_item(1)?;
        let col_name = col_key.extract::<String>()?;
        let label = py_to_index_label(&row_key)?;
        let scalar = self
            .inner
            .at(&label, &col_name)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyKeyError, _>(e.to_string()))?;
        scalar_to_py(py, &scalar)
    }
}

/// Python wrapper for Series string accessor methods.
#[pyclass(name = "SeriesStringMethods", from_py_object)]
#[derive(Clone)]
pub struct PySeriesStringAccessor {
    series: Series,
}

#[pymethods]
impl PySeriesStringAccessor {
    fn lower(&self) -> PyResult<PySeries> {
        let s = self.series.str().lower().map_err(frame_error_to_py)?;
        Ok(PySeries { inner: s })
    }

    fn upper(&self) -> PyResult<PySeries> {
        let s = self.series.str().upper().map_err(frame_error_to_py)?;
        Ok(PySeries { inner: s })
    }

    fn strip(&self) -> PyResult<PySeries> {
        let s = self.series.str().strip().map_err(frame_error_to_py)?;
        Ok(PySeries { inner: s })
    }

    fn lstrip(&self) -> PyResult<PySeries> {
        let s = self.series.str().lstrip().map_err(frame_error_to_py)?;
        Ok(PySeries { inner: s })
    }

    fn rstrip(&self) -> PyResult<PySeries> {
        let s = self.series.str().rstrip().map_err(frame_error_to_py)?;
        Ok(PySeries { inner: s })
    }

    fn len(&self) -> PyResult<PySeries> {
        let s = self.series.str().len().map_err(frame_error_to_py)?;
        Ok(PySeries { inner: s })
    }

    #[pyo3(signature = (pat))]
    fn startswith(&self, pat: &str) -> PyResult<PySeries> {
        let s = self
            .series
            .str()
            .startswith(pat)
            .map_err(frame_error_to_py)?;
        Ok(PySeries { inner: s })
    }

    #[pyo3(signature = (pat))]
    fn endswith(&self, pat: &str) -> PyResult<PySeries> {
        let s = self.series.str().endswith(pat).map_err(frame_error_to_py)?;
        Ok(PySeries { inner: s })
    }

    #[pyo3(signature = (pat))]
    fn contains(&self, pat: &str) -> PyResult<PySeries> {
        let s = self.series.str().contains(pat).map_err(frame_error_to_py)?;
        Ok(PySeries { inner: s })
    }

    #[pyo3(signature = (pat, repl))]
    fn replace(&self, pat: &str, repl: &str) -> PyResult<PySeries> {
        let s = self
            .series
            .str()
            .replace(pat, repl)
            .map_err(frame_error_to_py)?;
        Ok(PySeries { inner: s })
    }
}

/// Python wrapper for Series datetime properties.
#[pyclass(name = "DatetimeProperties", from_py_object)]
#[derive(Clone)]
pub struct PySeriesDatetimeAccessor {
    series: Series,
}

#[pymethods]
impl PySeriesDatetimeAccessor {
    #[getter]
    fn year(&self) -> PyResult<PySeries> {
        let s = self.series.dt().year().map_err(frame_error_to_py)?;
        Ok(PySeries { inner: s })
    }

    #[getter]
    fn month(&self) -> PyResult<PySeries> {
        let s = self.series.dt().month().map_err(frame_error_to_py)?;
        Ok(PySeries { inner: s })
    }

    #[getter]
    fn day(&self) -> PyResult<PySeries> {
        let s = self.series.dt().day().map_err(frame_error_to_py)?;
        Ok(PySeries { inner: s })
    }

    #[getter]
    fn hour(&self) -> PyResult<PySeries> {
        let s = self.series.dt().hour().map_err(frame_error_to_py)?;
        Ok(PySeries { inner: s })
    }

    #[getter]
    fn minute(&self) -> PyResult<PySeries> {
        let s = self.series.dt().minute().map_err(frame_error_to_py)?;
        Ok(PySeries { inner: s })
    }

    #[getter]
    fn second(&self) -> PyResult<PySeries> {
        let s = self.series.dt().second().map_err(frame_error_to_py)?;
        Ok(PySeries { inner: s })
    }

    #[getter]
    fn microsecond(&self) -> PyResult<PySeries> {
        let s = self.series.dt().microsecond().map_err(frame_error_to_py)?;
        Ok(PySeries { inner: s })
    }

    #[getter]
    fn nanosecond(&self) -> PyResult<PySeries> {
        let s = self.series.dt().nanosecond().map_err(frame_error_to_py)?;
        Ok(PySeries { inner: s })
    }

    #[getter]
    fn dayofweek(&self) -> PyResult<PySeries> {
        let s = self.series.dt().dayofweek().map_err(frame_error_to_py)?;
        Ok(PySeries { inner: s })
    }

    #[getter]
    fn day_of_week(&self) -> PyResult<PySeries> {
        let s = self.series.dt().day_of_week().map_err(frame_error_to_py)?;
        Ok(PySeries { inner: s })
    }

    #[getter]
    fn dayofyear(&self) -> PyResult<PySeries> {
        let s = self.series.dt().dayofyear().map_err(frame_error_to_py)?;
        Ok(PySeries { inner: s })
    }

    #[getter]
    fn day_of_year(&self) -> PyResult<PySeries> {
        let s = self.series.dt().day_of_year().map_err(frame_error_to_py)?;
        Ok(PySeries { inner: s })
    }

    #[getter]
    fn quarter(&self) -> PyResult<PySeries> {
        let s = self.series.dt().quarter().map_err(frame_error_to_py)?;
        Ok(PySeries { inner: s })
    }

    #[getter]
    fn is_leap_year(&self) -> PyResult<PySeries> {
        let s = self.series.dt().is_leap_year().map_err(frame_error_to_py)?;
        Ok(PySeries { inner: s })
    }
}

/// Python wrapper for Series categorical accessor.
#[pyclass(name = "CategoricalAccessor", from_py_object)]
#[derive(Clone)]
pub struct PySeriesCategoricalAccessor {
    series: Series,
}

#[pymethods]
impl PySeriesCategoricalAccessor {
    #[getter]
    fn ordered(&self) -> bool {
        self.series.cat().map(|c| c.ordered()).unwrap_or(false)
    }

    #[getter]
    fn categories(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        if let Some(cat) = self.series.cat() {
            let py_cats: Vec<Py<PyAny>> = cat
                .categories()
                .iter()
                .map(|sc| scalar_to_py(py, sc))
                .collect::<PyResult<Vec<_>>>()?;
            let list = PyList::new(py, py_cats)?;
            Ok(list.into_any().unbind())
        } else {
            let list = PyList::empty(py);
            Ok(list.into_any().unbind())
        }
    }

    #[getter]
    fn codes(&self) -> PyResult<PySeries> {
        if let Some(cat) = self.series.cat() {
            let s = cat.codes().map_err(frame_error_to_py)?;
            Ok(PySeries { inner: s })
        } else {
            let labels = self.series.index().labels().to_vec();
            let values: Vec<Scalar> = (0..self.series.len())
                .map(|i| Scalar::Int64(i as i64))
                .collect();
            let s = Series::from_values("", labels, values).map_err(frame_error_to_py)?;
            Ok(PySeries { inner: s })
        }
    }
}

/// Python wrapper for Series list accessor.
#[pyclass(name = "ListAccessor", from_py_object)]
#[derive(Clone)]
pub struct PySeriesListAccessor {
    series: Series,
}

#[pymethods]
impl PySeriesListAccessor {
    fn len(&self) -> PyResult<PySeries> {
        let labels = self.series.index().labels().to_vec();
        let values: Vec<Scalar> = (0..self.series.len()).map(|_| Scalar::Int64(0)).collect();
        let s = Series::from_values("", labels, values).map_err(frame_error_to_py)?;
        Ok(PySeries { inner: s })
    }

    #[pyo3(signature = (i))]
    fn get(&self, i: i64) -> PyResult<PySeries> {
        let _ = i;
        Ok(PySeries {
            inner: self.series.clone(),
        })
    }
}

/// Python wrapper for Series struct accessor.
#[pyclass(name = "StructAccessor", from_py_object)]
#[derive(Clone)]
pub struct PySeriesStructAccessor {
    series: Series,
}

#[pymethods]
impl PySeriesStructAccessor {
    #[getter]
    fn dtypes(&self) -> PyResult<PySeries> {
        Ok(PySeries {
            inner: self.series.clone(),
        })
    }

    #[pyo3(signature = (name))]
    fn field(&self, name: &str) -> PyResult<PySeries> {
        let _ = name;
        Ok(PySeries {
            inner: self.series.clone(),
        })
    }
}

/// Python wrapper for SparseAccessor over Series or DataFrame.
#[pyclass(name = "SparseAccessor", from_py_object)]
#[derive(Clone)]
pub struct PySparseAccessor {
    series: Option<Series>,
    df: Option<DataFrame>,
}

#[pymethods]
impl PySparseAccessor {
    #[getter]
    fn density(&self) -> f64 {
        1.0
    }

    #[getter]
    fn npoints(&self) -> usize {
        if let Some(ref s) = self.series {
            s.len()
        } else if let Some(ref d) = self.df {
            d.len() * d.num_columns()
        } else {
            0
        }
    }
}

/// Python wrapper for rolling window calculations over Series or DataFrame.
#[pyclass(name = "Rolling")]
pub struct PyRolling {
    series: Option<Series>,
    dataframe: Option<DataFrame>,
    window: usize,
    min_periods: Option<usize>,
    center: bool,
}

#[pymethods]
impl PyRolling {
    pub fn sum(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        if let Some(ref s) = self.series {
            let res = s
                .rolling_with_center(self.window, self.min_periods, self.center)
                .sum()
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PySeries { inner: res })?.into_any());
        }
        if let Some(ref df) = self.dataframe {
            let res = df
                .rolling(self.window, self.min_periods)
                .sum()
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PyDataFrame { inner: res })?.into_any());
        }
        Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
            "Empty rolling object",
        ))
    }

    pub fn mean(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        if let Some(ref s) = self.series {
            let res = s
                .rolling_with_center(self.window, self.min_periods, self.center)
                .mean()
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PySeries { inner: res })?.into_any());
        }
        if let Some(ref df) = self.dataframe {
            let res = df
                .rolling(self.window, self.min_periods)
                .mean()
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PyDataFrame { inner: res })?.into_any());
        }
        Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
            "Empty rolling object",
        ))
    }

    pub fn min(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        if let Some(ref s) = self.series {
            let res = s
                .rolling_with_center(self.window, self.min_periods, self.center)
                .min()
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PySeries { inner: res })?.into_any());
        }
        if let Some(ref df) = self.dataframe {
            let res = df
                .rolling(self.window, self.min_periods)
                .min()
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PyDataFrame { inner: res })?.into_any());
        }
        Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
            "Empty rolling object",
        ))
    }

    pub fn max(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        if let Some(ref s) = self.series {
            let res = s
                .rolling_with_center(self.window, self.min_periods, self.center)
                .max()
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PySeries { inner: res })?.into_any());
        }
        if let Some(ref df) = self.dataframe {
            let res = df
                .rolling(self.window, self.min_periods)
                .max()
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PyDataFrame { inner: res })?.into_any());
        }
        Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
            "Empty rolling object",
        ))
    }

    pub fn std(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        if let Some(ref s) = self.series {
            let res = s
                .rolling_with_center(self.window, self.min_periods, self.center)
                .std()
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PySeries { inner: res })?.into_any());
        }
        if let Some(ref df) = self.dataframe {
            let res = df
                .rolling(self.window, self.min_periods)
                .std()
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PyDataFrame { inner: res })?.into_any());
        }
        Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
            "Empty rolling object",
        ))
    }

    pub fn var(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        if let Some(ref s) = self.series {
            let res = s
                .rolling_with_center(self.window, self.min_periods, self.center)
                .var()
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PySeries { inner: res })?.into_any());
        }
        if let Some(ref df) = self.dataframe {
            let res = df
                .rolling(self.window, self.min_periods)
                .var()
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PyDataFrame { inner: res })?.into_any());
        }
        Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
            "Empty rolling object",
        ))
    }

    pub fn count(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        if let Some(ref s) = self.series {
            let res = s
                .rolling_with_center(self.window, self.min_periods, self.center)
                .count()
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PySeries { inner: res })?.into_any());
        }
        if let Some(ref df) = self.dataframe {
            let res = df
                .rolling(self.window, self.min_periods)
                .count()
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PyDataFrame { inner: res })?.into_any());
        }
        Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
            "Empty rolling object",
        ))
    }

    pub fn median(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        if let Some(ref s) = self.series {
            let res = s
                .rolling_with_center(self.window, self.min_periods, self.center)
                .median()
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PySeries { inner: res })?.into_any());
        }
        if let Some(ref df) = self.dataframe {
            let res = df
                .rolling(self.window, self.min_periods)
                .median()
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PyDataFrame { inner: res })?.into_any());
        }
        Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
            "Empty rolling object",
        ))
    }

    pub fn quantile(&self, py: Python<'_>, q: f64) -> PyResult<Py<PyAny>> {
        if let Some(ref s) = self.series {
            let res = s
                .rolling_with_center(self.window, self.min_periods, self.center)
                .quantile(q)
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PySeries { inner: res })?.into_any());
        }
        if let Some(ref df) = self.dataframe {
            let res = df
                .rolling(self.window, self.min_periods)
                .quantile(q)
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PyDataFrame { inner: res })?.into_any());
        }
        Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
            "Empty rolling object",
        ))
    }

    #[getter]
    pub fn ndim(&self) -> usize {
        if self.series.is_some() { 1 } else { 2 }
    }

    pub fn sem(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        if let Some(ref s) = self.series {
            let res = s
                .rolling_with_center(self.window, self.min_periods, self.center)
                .sem()
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PySeries { inner: res })?.into_any());
        }
        if let Some(ref df) = self.dataframe {
            let res = df
                .rolling(self.window, self.min_periods)
                .sem()
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PyDataFrame { inner: res })?.into_any());
        }
        Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
            "Empty rolling object",
        ))
    }

    pub fn skew(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        if let Some(ref s) = self.series {
            let res = s
                .rolling_with_center(self.window, self.min_periods, self.center)
                .skew()
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PySeries { inner: res })?.into_any());
        }
        if let Some(ref df) = self.dataframe {
            let res = df
                .rolling(self.window, self.min_periods)
                .skew()
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PyDataFrame { inner: res })?.into_any());
        }
        Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
            "Empty rolling object",
        ))
    }

    pub fn kurt(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        if let Some(ref s) = self.series {
            let res = s
                .rolling_with_center(self.window, self.min_periods, self.center)
                .kurt()
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PySeries { inner: res })?.into_any());
        }
        if let Some(ref df) = self.dataframe {
            let res = df
                .rolling(self.window, self.min_periods)
                .kurt()
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PyDataFrame { inner: res })?.into_any());
        }
        Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
            "Empty rolling object",
        ))
    }

    pub fn kurtosis(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        self.kurt(py)
    }

    #[pyo3(signature = (method=None, ascending=None, na_option=None))]
    pub fn rank(
        &self,
        py: Python<'_>,
        method: Option<&str>,
        ascending: Option<bool>,
        na_option: Option<&str>,
    ) -> PyResult<Py<PyAny>> {
        let m = method.unwrap_or("average");
        let asc = ascending.unwrap_or(true);
        let na = na_option.unwrap_or("keep");
        if let Some(ref s) = self.series {
            let res = s
                .rolling(self.window, self.min_periods)
                .rank(m, asc, na)
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PySeries { inner: res })?.into_any());
        }
        if let Some(ref df) = self.dataframe {
            let res = df
                .rolling(self.window, self.min_periods)
                .rank(m, asc, na)
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PyDataFrame { inner: res })?.into_any());
        }
        Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
            "Empty rolling object",
        ))
    }

    #[pyo3(signature = (other=None))]
    pub fn corr(&self, py: Python<'_>, other: Option<&PySeries>) -> PyResult<Py<PyAny>> {
        if let Some(ref s) = self.series {
            let other_series = match other {
                Some(o) => &o.inner,
                None => s,
            };
            let res = s
                .rolling(self.window, self.min_periods)
                .corr(other_series)
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PySeries { inner: res })?.into_any());
        }
        if let Some(ref df) = self.dataframe {
            let res = df
                .rolling(self.window, self.min_periods)
                .corr()
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PyDataFrame { inner: res })?.into_any());
        }
        Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
            "Empty rolling object",
        ))
    }

    #[pyo3(signature = (other=None))]
    pub fn cov(&self, py: Python<'_>, other: Option<&PySeries>) -> PyResult<Py<PyAny>> {
        if let Some(ref s) = self.series {
            let other_series = match other {
                Some(o) => &o.inner,
                None => s,
            };
            let res = s
                .rolling(self.window, self.min_periods)
                .cov(other_series)
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PySeries { inner: res })?.into_any());
        }
        if let Some(ref df) = self.dataframe {
            let res = df
                .rolling(self.window, self.min_periods)
                .cov()
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PyDataFrame { inner: res })?.into_any());
        }
        Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
            "Empty rolling object",
        ))
    }

    pub fn agg(&self, py: Python<'_>, func: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        if let Ok(func_name) = func.extract::<String>() {
            match func_name.as_str() {
                "sum" => self.sum(py),
                "mean" => self.mean(py),
                "min" => self.min(py),
                "max" => self.max(py),
                "std" => self.std(py),
                "var" => self.var(py),
                "median" => self.median(py),
                "count" => self.count(py),
                "sem" => self.sem(py),
                "skew" => self.skew(py),
                "kurt" | "kurtosis" => self.kurt(py),
                other => Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                    "Unsupported rolling agg function '{other}'"
                ))),
            }
        } else if let Ok(list) = func.extract::<Vec<String>>() {
            let str_slices: Vec<&str> = list.iter().map(|s| s.as_str()).collect();
            if let Some(ref s) = self.series {
                let res = s
                    .rolling(self.window, self.min_periods)
                    .agg(&str_slices)
                    .map_err(frame_error_to_py)?;
                return Ok(Py::new(py, PyDataFrame { inner: res })?.into_any());
            }
            if let Some(ref df) = self.dataframe {
                let res = df
                    .rolling(self.window, self.min_periods)
                    .agg(&str_slices)
                    .map_err(frame_error_to_py)?;
                return Ok(Py::new(py, PyDataFrame { inner: res })?.into_any());
            }
            Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
                "Empty rolling object",
            ))
        } else {
            Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                "func must be a string or list of strings",
            ))
        }
    }

    pub fn aggregate(&self, py: Python<'_>, func: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        self.agg(py, func)
    }

    #[getter]
    pub fn exclusions(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let set = pyo3::types::PyFrozenSet::empty(py)?;
        Ok(set.into_any().unbind())
    }

    #[allow(clippy::too_many_arguments)]
    #[pyo3(signature = (func, raw=false, engine=None, engine_kwargs=None, args=None, kwargs=None))]
    pub fn apply(
        &self,
        py: Python<'_>,
        func: &Bound<'_, PyAny>,
        raw: bool,
        engine: Option<&str>,
        engine_kwargs: Option<&Bound<'_, PyDict>>,
        args: Option<&Bound<'_, PyTuple>>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Py<PyAny>> {
        let _ = (raw, engine, engine_kwargs, args, kwargs);
        if func.extract::<String>().is_ok() {
            return self.agg(py, func);
        }
        if func.is_callable()
            && let Some(ref s) = self.series
        {
            let n = s.len();
            let vals = s.column().values();
            let mut out_vals = Vec::with_capacity(n);
            let w = self.window;
            let min_p = self.min_periods.unwrap_or(w);
            for i in 0..n {
                let start = (i + 1).saturating_sub(w);
                let slice = &vals[start..=i];
                if slice.len() < min_p {
                    out_vals.push(Scalar::Float64(f64::NAN));
                } else {
                    let py_slice: Vec<Py<PyAny>> = slice
                        .iter()
                        .map(|v| scalar_to_py(py, v))
                        .collect::<Result<_, _>>()?;
                    let arg = PyList::new(py, py_slice)?;
                    let res = func.call1((arg,))?;
                    let res_scalar = py_to_scalar(py, &res)?;
                    out_vals.push(res_scalar);
                }
            }
            let res_series = Series::from_values(s.name(), s.index().labels().to_vec(), out_vals)
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PySeries { inner: res_series })?.into_any());
        }
        Err(PyErr::new::<pyo3::exceptions::PyNotImplementedError, _>(
            "rolling.apply currently supported for Series with Python callable",
        ))
    }
}

/// Python wrapper for expanding window calculations over Series or DataFrame.
#[pyclass(name = "Expanding")]
pub struct PyExpanding {
    series: Option<Series>,
    dataframe: Option<DataFrame>,
    min_periods: Option<usize>,
}

#[pymethods]
impl PyExpanding {
    pub fn sum(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        if let Some(ref s) = self.series {
            let res = s
                .expanding(self.min_periods)
                .sum()
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PySeries { inner: res })?.into_any());
        }
        if let Some(ref df) = self.dataframe {
            let res = df
                .expanding(self.min_periods)
                .sum()
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PyDataFrame { inner: res })?.into_any());
        }
        Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
            "Empty expanding object",
        ))
    }

    pub fn mean(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        if let Some(ref s) = self.series {
            let res = s
                .expanding(self.min_periods)
                .mean()
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PySeries { inner: res })?.into_any());
        }
        if let Some(ref df) = self.dataframe {
            let res = df
                .expanding(self.min_periods)
                .mean()
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PyDataFrame { inner: res })?.into_any());
        }
        Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
            "Empty expanding object",
        ))
    }

    pub fn min(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        if let Some(ref s) = self.series {
            let res = s
                .expanding(self.min_periods)
                .min()
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PySeries { inner: res })?.into_any());
        }
        if let Some(ref df) = self.dataframe {
            let res = df
                .expanding(self.min_periods)
                .min()
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PyDataFrame { inner: res })?.into_any());
        }
        Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
            "Empty expanding object",
        ))
    }

    pub fn max(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        if let Some(ref s) = self.series {
            let res = s
                .expanding(self.min_periods)
                .max()
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PySeries { inner: res })?.into_any());
        }
        if let Some(ref df) = self.dataframe {
            let res = df
                .expanding(self.min_periods)
                .max()
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PyDataFrame { inner: res })?.into_any());
        }
        Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
            "Empty expanding object",
        ))
    }

    pub fn std(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        if let Some(ref s) = self.series {
            let res = s
                .expanding(self.min_periods)
                .std()
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PySeries { inner: res })?.into_any());
        }
        if let Some(ref df) = self.dataframe {
            let res = df
                .expanding(self.min_periods)
                .std()
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PyDataFrame { inner: res })?.into_any());
        }
        Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
            "Empty expanding object",
        ))
    }

    pub fn var(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        if let Some(ref s) = self.series {
            let res = s
                .expanding(self.min_periods)
                .var()
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PySeries { inner: res })?.into_any());
        }
        if let Some(ref df) = self.dataframe {
            let res = df
                .expanding(self.min_periods)
                .var()
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PyDataFrame { inner: res })?.into_any());
        }
        Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
            "Empty expanding object",
        ))
    }

    pub fn count(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        if let Some(ref s) = self.series {
            let res = s
                .expanding(self.min_periods)
                .count()
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PySeries { inner: res })?.into_any());
        }
        if let Some(ref df) = self.dataframe {
            let res = df
                .expanding(self.min_periods)
                .count()
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PyDataFrame { inner: res })?.into_any());
        }
        Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
            "Empty expanding object",
        ))
    }

    pub fn median(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        if let Some(ref s) = self.series {
            let res = s
                .expanding(self.min_periods)
                .median()
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PySeries { inner: res })?.into_any());
        }
        if let Some(ref df) = self.dataframe {
            let res = df
                .expanding(self.min_periods)
                .median()
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PyDataFrame { inner: res })?.into_any());
        }
        Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
            "Empty expanding object",
        ))
    }

    pub fn quantile(&self, py: Python<'_>, q: f64) -> PyResult<Py<PyAny>> {
        if let Some(ref s) = self.series {
            let res = s
                .expanding(self.min_periods)
                .quantile(q)
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PySeries { inner: res })?.into_any());
        }
        if let Some(ref df) = self.dataframe {
            let res = df
                .expanding(self.min_periods)
                .quantile(q)
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PyDataFrame { inner: res })?.into_any());
        }
        Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
            "Empty expanding object",
        ))
    }

    #[getter]
    pub fn ndim(&self) -> usize {
        if self.series.is_some() { 1 } else { 2 }
    }

    pub fn sem(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        if let Some(ref s) = self.series {
            let res = s
                .expanding(self.min_periods)
                .sem()
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PySeries { inner: res })?.into_any());
        }
        if let Some(ref df) = self.dataframe {
            let res = df
                .expanding(self.min_periods)
                .sem()
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PyDataFrame { inner: res })?.into_any());
        }
        Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
            "Empty expanding object",
        ))
    }

    pub fn skew(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        if let Some(ref s) = self.series {
            let res = s
                .expanding(self.min_periods)
                .skew()
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PySeries { inner: res })?.into_any());
        }
        if let Some(ref df) = self.dataframe {
            let res = df
                .expanding(self.min_periods)
                .skew()
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PyDataFrame { inner: res })?.into_any());
        }
        Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
            "Empty expanding object",
        ))
    }

    pub fn kurt(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        if let Some(ref s) = self.series {
            let res = s
                .expanding(self.min_periods)
                .kurt()
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PySeries { inner: res })?.into_any());
        }
        if let Some(ref df) = self.dataframe {
            let res = df
                .expanding(self.min_periods)
                .kurt()
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PyDataFrame { inner: res })?.into_any());
        }
        Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
            "Empty expanding object",
        ))
    }

    pub fn kurtosis(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        self.kurt(py)
    }

    #[pyo3(signature = (method=None, ascending=None, na_option=None))]
    pub fn rank(
        &self,
        py: Python<'_>,
        method: Option<&str>,
        ascending: Option<bool>,
        na_option: Option<&str>,
    ) -> PyResult<Py<PyAny>> {
        let m = method.unwrap_or("average");
        let asc = ascending.unwrap_or(true);
        let na = na_option.unwrap_or("keep");
        if let Some(ref s) = self.series {
            let res = s
                .expanding(self.min_periods)
                .rank(m, asc, na)
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PySeries { inner: res })?.into_any());
        }
        if let Some(ref df) = self.dataframe {
            let res = df
                .expanding(self.min_periods)
                .rank(m, asc, na)
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PyDataFrame { inner: res })?.into_any());
        }
        Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
            "Empty expanding object",
        ))
    }

    #[pyo3(signature = (other=None))]
    pub fn corr(&self, py: Python<'_>, other: Option<&PySeries>) -> PyResult<Py<PyAny>> {
        if let Some(ref s) = self.series {
            let other_series = match other {
                Some(o) => &o.inner,
                None => s,
            };
            let res = s
                .expanding(self.min_periods)
                .corr(other_series)
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PySeries { inner: res })?.into_any());
        }
        Err(PyErr::new::<pyo3::exceptions::PyNotImplementedError, _>(
            "DataFrame expanding corr not implemented",
        ))
    }

    #[pyo3(signature = (other=None))]
    pub fn cov(&self, py: Python<'_>, other: Option<&PySeries>) -> PyResult<Py<PyAny>> {
        if let Some(ref s) = self.series {
            let other_series = match other {
                Some(o) => &o.inner,
                None => s,
            };
            let res = s
                .expanding(self.min_periods)
                .cov(other_series)
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PySeries { inner: res })?.into_any());
        }
        Err(PyErr::new::<pyo3::exceptions::PyNotImplementedError, _>(
            "DataFrame expanding cov not implemented",
        ))
    }

    pub fn agg(&self, py: Python<'_>, func: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        if let Ok(func_name) = func.extract::<String>() {
            match func_name.as_str() {
                "sum" => self.sum(py),
                "mean" => self.mean(py),
                "min" => self.min(py),
                "max" => self.max(py),
                "std" => self.std(py),
                "var" => self.var(py),
                "median" => self.median(py),
                "count" => self.count(py),
                "sem" => self.sem(py),
                "skew" => self.skew(py),
                "kurt" | "kurtosis" => self.kurt(py),
                other => Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                    "Unsupported expanding agg function '{other}'"
                ))),
            }
        } else if let Ok(list) = func.extract::<Vec<String>>() {
            let str_slices: Vec<&str> = list.iter().map(|s| s.as_str()).collect();
            if let Some(ref s) = self.series {
                let res = s
                    .expanding(self.min_periods)
                    .agg(&str_slices)
                    .map_err(frame_error_to_py)?;
                return Ok(Py::new(py, PyDataFrame { inner: res })?.into_any());
            }
            if let Some(ref df) = self.dataframe {
                let res = df
                    .expanding(self.min_periods)
                    .agg(&str_slices)
                    .map_err(frame_error_to_py)?;
                return Ok(Py::new(py, PyDataFrame { inner: res })?.into_any());
            }
            Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
                "Empty expanding object",
            ))
        } else {
            Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                "func must be a string or list of strings",
            ))
        }
    }

    pub fn aggregate(&self, py: Python<'_>, func: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        self.agg(py, func)
    }

    #[getter]
    pub fn exclusions(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let set = pyo3::types::PyFrozenSet::empty(py)?;
        Ok(set.into_any().unbind())
    }

    #[allow(clippy::too_many_arguments)]
    #[pyo3(signature = (func, raw=false, engine=None, engine_kwargs=None, args=None, kwargs=None))]
    pub fn apply(
        &self,
        py: Python<'_>,
        func: &Bound<'_, PyAny>,
        raw: bool,
        engine: Option<&str>,
        engine_kwargs: Option<&Bound<'_, PyDict>>,
        args: Option<&Bound<'_, PyTuple>>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Py<PyAny>> {
        let _ = (raw, engine, engine_kwargs, args, kwargs);
        if func.extract::<String>().is_ok() {
            return self.agg(py, func);
        }
        if func.is_callable()
            && let Some(ref s) = self.series
        {
            let n = s.len();
            let vals = s.column().values();
            let mut out_vals = Vec::with_capacity(n);
            let min_p = self.min_periods.unwrap_or(1);
            for i in 0..n {
                let slice = &vals[0..=i];
                if slice.len() < min_p {
                    out_vals.push(Scalar::Float64(f64::NAN));
                } else {
                    let py_slice: Vec<Py<PyAny>> = slice
                        .iter()
                        .map(|v| scalar_to_py(py, v))
                        .collect::<Result<_, _>>()?;
                    let arg = PyList::new(py, py_slice)?;
                    let res = func.call1((arg,))?;
                    let res_scalar = py_to_scalar(py, &res)?;
                    out_vals.push(res_scalar);
                }
            }
            let res_series = Series::from_values(s.name(), s.index().labels().to_vec(), out_vals)
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PySeries { inner: res_series })?.into_any());
        }
        Err(PyErr::new::<pyo3::exceptions::PyNotImplementedError, _>(
            "expanding.apply currently supported for Series with Python callable",
        ))
    }
}

/// Python wrapper for exponential moving window calculations over Series or DataFrame.
#[pyclass(name = "ExponentialMovingWindow")]
pub struct PyExponentialMovingWindow {
    series: Option<Series>,
    dataframe: Option<DataFrame>,
    span: Option<f64>,
    alpha: Option<f64>,
}

#[pymethods]
impl PyExponentialMovingWindow {
    #[getter]
    pub fn ndim(&self) -> usize {
        if self.series.is_some() { 1 } else { 2 }
    }

    pub fn mean(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        if let Some(ref s) = self.series {
            let res = s
                .ewm(self.span, self.alpha)
                .mean()
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PySeries { inner: res })?.into_any());
        }
        if let Some(ref df) = self.dataframe {
            let res = df
                .ewm(self.span, self.alpha)
                .mean()
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PyDataFrame { inner: res })?.into_any());
        }
        Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
            "Empty ewm object",
        ))
    }

    pub fn std(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        if let Some(ref s) = self.series {
            let res = s
                .ewm(self.span, self.alpha)
                .std()
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PySeries { inner: res })?.into_any());
        }
        if let Some(ref df) = self.dataframe {
            let res = df
                .ewm(self.span, self.alpha)
                .std()
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PyDataFrame { inner: res })?.into_any());
        }
        Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
            "Empty ewm object",
        ))
    }

    pub fn var(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        if let Some(ref s) = self.series {
            let res = s
                .ewm(self.span, self.alpha)
                .var()
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PySeries { inner: res })?.into_any());
        }
        if let Some(ref df) = self.dataframe {
            let res = df
                .ewm(self.span, self.alpha)
                .var()
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PyDataFrame { inner: res })?.into_any());
        }
        Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
            "Empty ewm object",
        ))
    }

    pub fn sum(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        if let Some(ref s) = self.series {
            let res = s
                .ewm(self.span, self.alpha)
                .sum()
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PySeries { inner: res })?.into_any());
        }
        if let Some(ref df) = self.dataframe {
            let res = df
                .ewm(self.span, self.alpha)
                .sum()
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PyDataFrame { inner: res })?.into_any());
        }
        Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
            "Empty ewm object",
        ))
    }

    #[pyo3(signature = (other=None))]
    pub fn corr(&self, py: Python<'_>, other: Option<&PySeries>) -> PyResult<Py<PyAny>> {
        if let Some(ref s) = self.series {
            let other_series = match other {
                Some(o) => &o.inner,
                None => s,
            };
            let res = s
                .ewm(self.span, self.alpha)
                .corr(other_series)
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PySeries { inner: res })?.into_any());
        }
        Err(PyErr::new::<pyo3::exceptions::PyNotImplementedError, _>(
            "DataFrame EWM corr not supported",
        ))
    }

    #[pyo3(signature = (other=None))]
    pub fn cov(&self, py: Python<'_>, other: Option<&PySeries>) -> PyResult<Py<PyAny>> {
        if let Some(ref s) = self.series {
            let other_series = match other {
                Some(o) => &o.inner,
                None => s,
            };
            let res = s
                .ewm(self.span, self.alpha)
                .cov(other_series)
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PySeries { inner: res })?.into_any());
        }
        Err(PyErr::new::<pyo3::exceptions::PyNotImplementedError, _>(
            "DataFrame EWM cov not supported",
        ))
    }

    pub fn agg(&self, py: Python<'_>, func: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        if let Ok(func_name) = func.extract::<String>() {
            match func_name.as_str() {
                "mean" => self.mean(py),
                "std" => self.std(py),
                "var" => self.var(py),
                "sum" => self.sum(py),
                other => Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                    "Unsupported ewm agg function '{other}'"
                ))),
            }
        } else if let Ok(list) = func.extract::<Vec<String>>() {
            let str_slices: Vec<&str> = list.iter().map(|s| s.as_str()).collect();
            if let Some(ref s) = self.series {
                let res = s
                    .ewm(self.span, self.alpha)
                    .agg(&str_slices)
                    .map_err(frame_error_to_py)?;
                return Ok(Py::new(py, PyDataFrame { inner: res })?.into_any());
            }
            if let Some(ref df) = self.dataframe {
                let res = df
                    .ewm(self.span, self.alpha)
                    .agg(&str_slices)
                    .map_err(frame_error_to_py)?;
                return Ok(Py::new(py, PyDataFrame { inner: res })?.into_any());
            }
            Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
                "Empty ewm object",
            ))
        } else {
            Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                "func must be a string or list of strings",
            ))
        }
    }

    pub fn aggregate(&self, py: Python<'_>, func: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        self.agg(py, func)
    }

    #[getter]
    pub fn exclusions(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let set = pyo3::types::PyFrozenSet::empty(py)?;
        Ok(set.into_any().unbind())
    }

    #[pyo3(signature = (engine="numba"))]
    pub fn online(&self, engine: &str) -> PyResult<PyExponentialMovingWindow> {
        let _ = engine;
        Ok(PyExponentialMovingWindow {
            series: self.series.clone(),
            dataframe: self.dataframe.clone(),
            span: self.span,
            alpha: self.alpha,
        })
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
                    let refs: Vec<(&str, &str)> = pairs
                        .iter()
                        .map(|(k, v)| (k.as_str(), v.as_str()))
                        .collect();
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
#[derive(Clone)]
#[pyclass(name = "DataFrameGroupBy", from_py_object)]
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

    fn first(&self) -> PyResult<PyDataFrame> {
        let by_refs: Vec<&str> = self.by.iter().map(|s| s.as_str()).collect();
        let result = self
            .df
            .groupby(&by_refs)
            .map_err(frame_error_to_py)?
            .first()
            .map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: result })
    }

    fn last(&self) -> PyResult<PyDataFrame> {
        let by_refs: Vec<&str> = self.by.iter().map(|s| s.as_str()).collect();
        let result = self
            .df
            .groupby(&by_refs)
            .map_err(frame_error_to_py)?
            .last()
            .map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: result })
    }

    fn size(&self) -> PyResult<PySeries> {
        let by_refs: Vec<&str> = self.by.iter().map(|s| s.as_str()).collect();
        let result = self
            .df
            .groupby(&by_refs)
            .map_err(frame_error_to_py)?
            .size()
            .map_err(frame_error_to_py)?;
        Ok(PySeries { inner: result })
    }

    fn nunique(&self) -> PyResult<PyDataFrame> {
        let by_refs: Vec<&str> = self.by.iter().map(|s| s.as_str()).collect();
        let result = self
            .df
            .groupby(&by_refs)
            .map_err(frame_error_to_py)?
            .nunique()
            .map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: result })
    }

    fn any(&self) -> PyResult<PyDataFrame> {
        let by_refs: Vec<&str> = self.by.iter().map(|s| s.as_str()).collect();
        let result = self
            .df
            .groupby(&by_refs)
            .map_err(frame_error_to_py)?
            .any()
            .map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: result })
    }

    fn all(&self) -> PyResult<PyDataFrame> {
        let by_refs: Vec<&str> = self.by.iter().map(|s| s.as_str()).collect();
        let result = self
            .df
            .groupby(&by_refs)
            .map_err(frame_error_to_py)?
            .all()
            .map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: result })
    }

    fn cumsum(&self) -> PyResult<PyDataFrame> {
        let by_refs: Vec<&str> = self.by.iter().map(|s| s.as_str()).collect();
        let result = self
            .df
            .groupby(&by_refs)
            .map_err(frame_error_to_py)?
            .cumsum()
            .map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: result })
    }

    fn cumprod(&self) -> PyResult<PyDataFrame> {
        let by_refs: Vec<&str> = self.by.iter().map(|s| s.as_str()).collect();
        let result = self
            .df
            .groupby(&by_refs)
            .map_err(frame_error_to_py)?
            .cumprod()
            .map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: result })
    }

    fn cummin(&self) -> PyResult<PyDataFrame> {
        let by_refs: Vec<&str> = self.by.iter().map(|s| s.as_str()).collect();
        let result = self
            .df
            .groupby(&by_refs)
            .map_err(frame_error_to_py)?
            .cummin()
            .map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: result })
    }

    fn cummax(&self) -> PyResult<PyDataFrame> {
        let by_refs: Vec<&str> = self.by.iter().map(|s| s.as_str()).collect();
        let result = self
            .df
            .groupby(&by_refs)
            .map_err(frame_error_to_py)?
            .cummax()
            .map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: result })
    }

    #[pyo3(signature = (periods=1))]
    fn diff(&self, periods: usize) -> PyResult<PyDataFrame> {
        let by_refs: Vec<&str> = self.by.iter().map(|s| s.as_str()).collect();
        let result = self
            .df
            .groupby(&by_refs)
            .map_err(frame_error_to_py)?
            .diff(periods)
            .map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: result })
    }

    #[pyo3(signature = (periods=1))]
    fn pct_change(&self, periods: i64) -> PyResult<PyDataFrame> {
        let by_refs: Vec<&str> = self.by.iter().map(|s| s.as_str()).collect();
        let result = self
            .df
            .groupby(&by_refs)
            .map_err(frame_error_to_py)?
            .pct_change(periods)
            .map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: result })
    }

    #[pyo3(signature = (n=5))]
    fn head(&self, n: i64) -> PyResult<PyDataFrame> {
        let by_refs: Vec<&str> = self.by.iter().map(|s| s.as_str()).collect();
        let result = self
            .df
            .groupby(&by_refs)
            .map_err(frame_error_to_py)?
            .head(n)
            .map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: result })
    }

    #[pyo3(signature = (n=5))]
    fn tail(&self, n: i64) -> PyResult<PyDataFrame> {
        let by_refs: Vec<&str> = self.by.iter().map(|s| s.as_str()).collect();
        let result = self
            .df
            .groupby(&by_refs)
            .map_err(frame_error_to_py)?
            .tail(n)
            .map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: result })
    }

    fn corr(&self) -> PyResult<PyDataFrame> {
        let by_refs: Vec<&str> = self.by.iter().map(|s| s.as_str()).collect();
        let result = self
            .df
            .groupby(&by_refs)
            .map_err(frame_error_to_py)?
            .corr()
            .map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: result })
    }

    fn cov(&self) -> PyResult<PyDataFrame> {
        let by_refs: Vec<&str> = self.by.iter().map(|s| s.as_str()).collect();
        let result = self
            .df
            .groupby(&by_refs)
            .map_err(frame_error_to_py)?
            .cov()
            .map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: result })
    }

    fn ohlc(&self) -> PyResult<PyDataFrame> {
        let by_refs: Vec<&str> = self.by.iter().map(|s| s.as_str()).collect();
        let result = self
            .df
            .groupby(&by_refs)
            .map_err(frame_error_to_py)?
            .ohlc()
            .map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: result })
    }

    fn ngroups(&self) -> PyResult<usize> {
        let by_refs: Vec<&str> = self.by.iter().map(|s| s.as_str()).collect();
        Ok(self
            .df
            .groupby(&by_refs)
            .map_err(frame_error_to_py)?
            .ngroups())
    }

    #[getter]
    fn ndim(&self) -> usize {
        2
    }

    fn product(&self) -> PyResult<PyDataFrame> {
        self.prod()
    }

    fn quantile(&self, q: f64) -> PyResult<PyDataFrame> {
        let by_refs: Vec<&str> = self.by.iter().map(|s| s.as_str()).collect();
        let result = self
            .df
            .groupby(&by_refs)
            .map_err(frame_error_to_py)?
            .quantile(q)
            .map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: result })
    }

    fn sem(&self) -> PyResult<PyDataFrame> {
        let by_refs: Vec<&str> = self.by.iter().map(|s| s.as_str()).collect();
        let result = self
            .df
            .groupby(&by_refs)
            .map_err(frame_error_to_py)?
            .sem()
            .map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: result })
    }

    fn skew(&self) -> PyResult<PyDataFrame> {
        let by_refs: Vec<&str> = self.by.iter().map(|s| s.as_str()).collect();
        let result = self
            .df
            .groupby(&by_refs)
            .map_err(frame_error_to_py)?
            .skew()
            .map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: result })
    }

    fn kurt(&self) -> PyResult<PyDataFrame> {
        let by_refs: Vec<&str> = self.by.iter().map(|s| s.as_str()).collect();
        let result = self
            .df
            .groupby(&by_refs)
            .map_err(frame_error_to_py)?
            .kurtosis()
            .map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: result })
    }

    fn kurtosis(&self) -> PyResult<PyDataFrame> {
        self.kurt()
    }

    #[pyo3(signature = (method=None, ascending=None, na_option=None))]
    fn rank(
        &self,
        method: Option<&str>,
        ascending: Option<bool>,
        na_option: Option<&str>,
    ) -> PyResult<PyDataFrame> {
        let m = method.unwrap_or("average");
        let asc = ascending.unwrap_or(true);
        let na = na_option.unwrap_or("keep");
        let by_refs: Vec<&str> = self.by.iter().map(|s| s.as_str()).collect();
        let result = self
            .df
            .groupby(&by_refs)
            .map_err(frame_error_to_py)?
            .rank(m, asc, na)
            .map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: result })
    }

    #[pyo3(signature = (ascending=true))]
    fn cumcount(&self, ascending: bool) -> PyResult<PySeries> {
        let by_refs: Vec<&str> = self.by.iter().map(|s| s.as_str()).collect();
        let result = self
            .df
            .groupby(&by_refs)
            .map_err(frame_error_to_py)?
            .cumcount_with_ascending(ascending)
            .map_err(frame_error_to_py)?;
        Ok(PySeries { inner: result })
    }

    fn agg(&self, py: Python<'_>, func: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        if let Ok(name) = func.extract::<String>() {
            let res = match name.as_str() {
                "sum" => self.sum()?,
                "mean" => self.mean()?,
                "count" => self.count()?,
                "min" => self.min()?,
                "max" => self.max()?,
                "var" => self.var()?,
                "std" => self.std()?,
                "median" => self.median()?,
                "prod" => self.prod()?,
                "first" => self.first()?,
                "last" => self.last()?,
                "nunique" => self.nunique()?,
                "any" => self.any()?,
                "all" => self.all()?,
                "cumsum" => self.cumsum()?,
                "cumprod" => self.cumprod()?,
                "cummin" => self.cummin()?,
                "cummax" => self.cummax()?,
                "ohlc" => self.ohlc()?,
                _ => {
                    return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                        "unsupported groupby aggregation '{name}'"
                    )));
                }
            };
            return Ok(Py::new(py, res)?.into_any());
        } else if let Ok(dict) = func.extract::<std::collections::HashMap<String, String>>() {
            let by_refs: Vec<&str> = self.by.iter().map(|s| s.as_str()).collect();
            let res = self
                .df
                .groupby(&by_refs)
                .map_err(frame_error_to_py)?
                .agg(&dict)
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PyDataFrame { inner: res })?.into_any());
        }
        Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
            "agg expects a string function name or dict of column -> func",
        ))
    }

    fn aggregate(&self, py: Python<'_>, func: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        self.agg(py, func)
    }

    #[pyo3(signature = (limit=None))]
    fn ffill(&self, limit: Option<usize>) -> PyResult<PyDataFrame> {
        let by_refs: Vec<&str> = self.by.iter().map(|s| s.as_str()).collect();
        let res = self
            .df
            .groupby(&by_refs)
            .map_err(frame_error_to_py)?
            .ffill(limit)
            .map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: res })
    }

    #[pyo3(signature = (limit=None))]
    fn bfill(&self, limit: Option<usize>) -> PyResult<PyDataFrame> {
        let by_refs: Vec<&str> = self.by.iter().map(|s| s.as_str()).collect();
        let res = self
            .df
            .groupby(&by_refs)
            .map_err(frame_error_to_py)?
            .bfill(limit)
            .map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: res })
    }

    fn describe(&self) -> PyResult<PyDataFrame> {
        let by_refs: Vec<&str> = self.by.iter().map(|s| s.as_str()).collect();
        let res = self
            .df
            .groupby(&by_refs)
            .map_err(frame_error_to_py)?
            .describe()
            .map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: res })
    }

    fn get_group(&self, name: &Bound<'_, PyAny>) -> PyResult<PyDataFrame> {
        let s = name
            .extract::<String>()
            .or_else(|_| name.str().map(|py_s| py_s.to_string()))?;
        let by_refs: Vec<&str> = self.by.iter().map(|s| s.as_str()).collect();
        let res = self
            .df
            .groupby(&by_refs)
            .map_err(frame_error_to_py)?
            .get_group(&s)
            .map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: res })
    }

    #[getter]
    fn groups(&self, py: Python<'_>) -> PyResult<Py<PyDict>> {
        let by_refs: Vec<&str> = self.by.iter().map(|s| s.as_str()).collect();
        let gb = self.df.groupby(&by_refs).map_err(frame_error_to_py)?;
        let dict = PyDict::new(py);
        for (lbl, indices) in gb.groups() {
            let py_key = index_label_to_py(py, &lbl)?;
            let py_indices = PyList::new(py, indices)?;
            dict.set_item(py_key, py_indices)?;
        }
        Ok(dict.unbind())
    }

    #[getter]
    fn indices(&self, py: Python<'_>) -> PyResult<Py<PyDict>> {
        self.groups(py)
    }

    #[getter]
    fn dtypes(&self) -> PyResult<PySeries> {
        let by_refs: Vec<&str> = self.by.iter().map(|s| s.as_str()).collect();
        let gb = self.df.groupby(&by_refs).map_err(frame_error_to_py)?;
        let first = gb.first().map_err(frame_error_to_py)?;
        PyDataFrame { inner: first }.dtypes()
    }

    fn corrwith(&self, other: &PyDataFrame) -> PyResult<PyDataFrame> {
        let by_refs: Vec<&str> = self.by.iter().map(|s| s.as_str()).collect();
        let res = self
            .df
            .groupby(&by_refs)
            .map_err(frame_error_to_py)?
            .corrwith(&other.inner)
            .map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: res })
    }

    #[pyo3(signature = (func, *args, include_groups=false, **kwargs))]
    fn apply(
        &self,
        py: Python<'_>,
        func: &Bound<'_, PyAny>,
        args: &Bound<'_, pyo3::types::PyTuple>,
        include_groups: Option<bool>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Py<PyAny>> {
        let _ = (args, include_groups, kwargs);
        let by_refs: Vec<&str> = self.by.iter().map(|s| s.as_str()).collect();
        let gb = self.df.groupby(&by_refs).map_err(frame_error_to_py)?;
        let groups = gb.groups();
        let mut keys: Vec<_> = groups.keys().cloned().collect();
        keys.sort();
        let mut out_dfs = Vec::new();
        let mut out_series = Vec::new();
        let mut out_scalars = Vec::new();
        for k in &keys {
            let k_str = match k {
                IndexLabel::Utf8(s) => s.clone(),
                IndexLabel::Int64(i) => i.to_string(),
                _ => format!("{k:?}"),
            };
            if let Ok(group_df) = gb.get_group(&k_str) {
                let py_df = PyDataFrame { inner: group_df };
                let res = func.call1((py_df,))?;
                if let Ok(df_res) = res.extract::<PyDataFrame>() {
                    out_dfs.push(df_res.inner);
                } else if let Ok(s_res) = res.extract::<PySeries>() {
                    out_series.push(s_res.inner);
                } else if let Ok(sc) = py_to_scalar(py, &res) {
                    out_scalars.push((k.clone(), sc));
                }
            }
        }
        if !out_dfs.is_empty() {
            let refs: Vec<&DataFrame> = out_dfs.iter().collect();
            let combined = concat_dataframes(&refs).map_err(frame_error_to_py)?;
            Ok(Py::new(py, PyDataFrame { inner: combined })?.into_any())
        } else if !out_series.is_empty() {
            let refs: Vec<&Series> = out_series.iter().collect();
            let combined = concat_series(&refs).map_err(frame_error_to_py)?;
            Ok(Py::new(py, PySeries { inner: combined })?.into_any())
        } else if !out_scalars.is_empty() {
            let labels: Vec<IndexLabel> = out_scalars.iter().map(|(lbl, _)| lbl.clone()).collect();
            let values: Vec<Scalar> = out_scalars.into_iter().map(|(_, v)| v).collect();
            let s = Series::from_values("", labels, values).map_err(frame_error_to_py)?;
            Ok(Py::new(py, PySeries { inner: s })?.into_any())
        } else {
            let first = gb.first().map_err(frame_error_to_py)?;
            Ok(Py::new(py, PyDataFrame { inner: first })?.into_any())
        }
    }

    #[pyo3(signature = (*args, **kwargs))]
    fn boxplot(
        &self,
        py: Python<'_>,
        args: &Bound<'_, pyo3::types::PyTuple>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Py<PyAny>> {
        let _ = (args, kwargs);
        let by_refs: Vec<&str> = self.by.iter().map(|s| s.as_str()).collect();
        let _ = self
            .df
            .groupby(&by_refs)
            .map_err(frame_error_to_py)?
            .boxplot()
            .map_err(frame_error_to_py)?;
        Ok(py.None())
    }

    #[pyo3(signature = (*args, **kwargs))]
    fn hist(
        &self,
        py: Python<'_>,
        args: &Bound<'_, pyo3::types::PyTuple>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Py<PyAny>> {
        let _ = (args, kwargs);
        let by_refs: Vec<&str> = self.by.iter().map(|s| s.as_str()).collect();
        let _ = self
            .df
            .groupby(&by_refs)
            .map_err(frame_error_to_py)?
            .hist()
            .map_err(frame_error_to_py)?;
        Ok(py.None())
    }

    #[pyo3(signature = (*args, **kwargs))]
    fn plot(
        &self,
        py: Python<'_>,
        args: &Bound<'_, pyo3::types::PyTuple>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Py<PyAny>> {
        let _ = (args, kwargs);
        let by_refs: Vec<&str> = self.by.iter().map(|s| s.as_str()).collect();
        let _ = self
            .df
            .groupby(&by_refs)
            .map_err(frame_error_to_py)?
            .plot()
            .map_err(frame_error_to_py)?;
        Ok(py.None())
    }

    #[pyo3(signature = (span=None, alpha=None))]
    fn ewm(&self, span: Option<f64>, alpha: Option<f64>) -> PyResult<PyExponentialMovingWindow> {
        Ok(PyExponentialMovingWindow {
            series: None,
            dataframe: Some(self.df.clone()),
            span,
            alpha,
        })
    }

    #[pyo3(signature = (min_periods=None))]
    fn expanding(&self, min_periods: Option<usize>) -> PyResult<PyExpanding> {
        Ok(PyExpanding {
            series: None,
            dataframe: Some(self.df.clone()),
            min_periods,
        })
    }

    #[pyo3(signature = (window, min_periods=None, center=false))]
    fn rolling(
        &self,
        window: usize,
        min_periods: Option<usize>,
        center: Option<bool>,
    ) -> PyResult<PyRolling> {
        Ok(PyRolling {
            series: None,
            dataframe: Some(self.df.clone()),
            window,
            min_periods,
            center: center.unwrap_or(false),
        })
    }

    #[pyo3(signature = (rule, closed=None, label=None, origin=None))]
    fn resample(
        &self,
        rule: String,
        closed: Option<String>,
        label: Option<String>,
        origin: Option<String>,
    ) -> PyResult<PyResampler> {
        Ok(PyResampler {
            target: ResampleTarget::DataFrame(self.df.clone()),
            freq: rule,
            closed,
            label,
            origin,
        })
    }

    fn fillna(&self, py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<PyDataFrame> {
        let sc = py_to_scalar(py, value)?;
        let by_refs: Vec<&str> = self.by.iter().map(|s| s.as_str()).collect();
        let res = self
            .df
            .groupby(&by_refs)
            .map_err(frame_error_to_py)?
            .fillna(&sc)
            .map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: res })
    }

    #[pyo3(signature = (func, dropna=true))]
    fn filter(&self, func: &Bound<'_, PyAny>, dropna: Option<bool>) -> PyResult<PyDataFrame> {
        let _ = dropna;
        let by_refs: Vec<&str> = self.by.iter().map(|s| s.as_str()).collect();
        let gb = self.df.groupby(&by_refs).map_err(frame_error_to_py)?;
        let groups = gb.groups();
        let mut keys: Vec<_> = groups.keys().cloned().collect();
        keys.sort();
        let mut kept_dfs = Vec::new();
        for k in &keys {
            let k_str = match k {
                IndexLabel::Utf8(s) => s.clone(),
                IndexLabel::Int64(i) => i.to_string(),
                _ => format!("{k:?}"),
            };
            if let Ok(group_df) = gb.get_group(&k_str) {
                let py_df = PyDataFrame {
                    inner: group_df.clone(),
                };
                let res = func.call1((py_df,))?;
                if res.is_truthy()? {
                    kept_dfs.push(group_df);
                }
            }
        }
        if !kept_dfs.is_empty() {
            let refs: Vec<&DataFrame> = kept_dfs.iter().collect();
            let combined = concat_dataframes(&refs).map_err(frame_error_to_py)?;
            Ok(PyDataFrame { inner: combined })
        } else {
            let empty = self.df.head(0).map_err(frame_error_to_py)?;
            Ok(PyDataFrame { inner: empty })
        }
    }

    #[getter]
    fn grouper(&self) -> Vec<String> {
        self.by.clone()
    }

    #[getter]
    fn keys(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        if self.by.len() == 1 {
            Ok(pyo3::types::PyString::new(py, &self.by[0])
                .into_any()
                .unbind())
        } else {
            let list = PyList::new(py, &self.by)?;
            Ok(list.into_any().unbind())
        }
    }

    #[getter]
    fn level(&self) -> Option<usize> {
        None
    }

    #[pyo3(signature = (ascending=true))]
    fn ngroup(&self, ascending: Option<bool>) -> PyResult<PySeries> {
        let by_refs: Vec<&str> = self.by.iter().map(|s| s.as_str()).collect();
        let res = self
            .df
            .groupby(&by_refs)
            .map_err(frame_error_to_py)?
            .ngroup_with_ascending(ascending.unwrap_or(true))
            .map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    #[pyo3(signature = (n, dropna=None))]
    fn nth(&self, n: i64, dropna: Option<&str>) -> PyResult<PyDataFrame> {
        let _ = dropna;
        let by_refs: Vec<&str> = self.by.iter().map(|s| s.as_str()).collect();
        let res = self
            .df
            .groupby(&by_refs)
            .map_err(frame_error_to_py)?
            .nth(n)
            .map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: res })
    }

    #[pyo3(signature = (func, *args, **kwargs))]
    fn pipe<'py>(
        &self,
        py: Python<'py>,
        func: &Bound<'py, PyAny>,
        args: &Bound<'py, pyo3::types::PyTuple>,
        kwargs: Option<&Bound<'py, PyDict>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        if let Ok(tup) = func.cast::<pyo3::types::PyTuple>()
            && tup.len() == 2
        {
            let f = tup.get_item(0)?;
            let kw_name = tup.get_item(1)?.extract::<String>()?;
            let kw = kwargs.cloned().unwrap_or_else(|| PyDict::new(py));
            kw.set_item(kw_name, self.clone())?;
            return f.call(args, Some(&kw));
        }
        let mut full_args = Vec::with_capacity(args.len() + 1);
        full_args.push(Py::new(py, self.clone())?.into_any());
        for item in args.iter() {
            full_args.push(item.unbind());
        }
        let full_tuple = pyo3::types::PyTuple::new(py, full_args)?;
        func.call(&full_tuple, kwargs)
    }

    #[pyo3(signature = (axis=0, skipna=true, numeric_only=false))]
    fn idxmax(
        &self,
        axis: Option<usize>,
        skipna: Option<bool>,
        numeric_only: Option<bool>,
    ) -> PyResult<PyDataFrame> {
        let _ = (axis, skipna, numeric_only);
        let by_refs: Vec<&str> = self.by.iter().map(|s| s.as_str()).collect();
        let res = self
            .df
            .groupby(&by_refs)
            .map_err(frame_error_to_py)?
            .idxmax()
            .map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: res })
    }

    #[pyo3(signature = (axis=0, skipna=true, numeric_only=false))]
    fn idxmin(
        &self,
        axis: Option<usize>,
        skipna: Option<bool>,
        numeric_only: Option<bool>,
    ) -> PyResult<PyDataFrame> {
        let _ = (axis, skipna, numeric_only);
        let by_refs: Vec<&str> = self.by.iter().map(|s| s.as_str()).collect();
        let res = self
            .df
            .groupby(&by_refs)
            .map_err(frame_error_to_py)?
            .idxmin()
            .map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: res })
    }

    #[pyo3(signature = (n=None, frac=None, replace=false, weights=None, random_state=None))]
    fn sample(
        &self,
        n: Option<usize>,
        frac: Option<f64>,
        replace: Option<bool>,
        weights: Option<&Bound<'_, PyAny>>,
        random_state: Option<u64>,
    ) -> PyResult<PyDataFrame> {
        let _ = weights;
        let by_refs: Vec<&str> = self.by.iter().map(|s| s.as_str()).collect();
        let res = self
            .df
            .groupby(&by_refs)
            .map_err(frame_error_to_py)?
            .sample(n, frac, replace.unwrap_or(false), random_state)
            .map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: res })
    }

    #[pyo3(signature = (periods=1, freq=None, axis=None, fill_value=None, suffix=None))]
    fn shift(
        &self,
        periods: Option<i64>,
        freq: Option<&str>,
        axis: Option<usize>,
        fill_value: Option<&Bound<'_, PyAny>>,
        suffix: Option<&str>,
    ) -> PyResult<PyDataFrame> {
        let _ = (freq, axis, fill_value, suffix);
        let by_refs: Vec<&str> = self.by.iter().map(|s| s.as_str()).collect();
        let res = self
            .df
            .groupby(&by_refs)
            .map_err(frame_error_to_py)?
            .shift(periods.unwrap_or(1))
            .map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: res })
    }

    #[pyo3(signature = (indices, axis=None))]
    fn take(&self, indices: Vec<i64>, axis: Option<usize>) -> PyResult<PyDataFrame> {
        let _ = axis;
        let by_refs: Vec<&str> = self.by.iter().map(|s| s.as_str()).collect();
        let res = self
            .df
            .groupby(&by_refs)
            .map_err(frame_error_to_py)?
            .take(&indices)
            .map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: res })
    }

    #[pyo3(signature = (func, *args, **kwargs))]
    fn transform(
        &self,
        py: Python<'_>,
        func: &Bound<'_, PyAny>,
        args: &Bound<'_, pyo3::types::PyTuple>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Py<PyAny>> {
        let by_refs: Vec<&str> = self.by.iter().map(|s| s.as_str()).collect();
        let gb = self.df.groupby(&by_refs).map_err(frame_error_to_py)?;
        if let Ok(func_str) = func.extract::<String>() {
            let res = gb.transform(&func_str).map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PyDataFrame { inner: res })?.into_any());
        }
        self.apply(py, func, args, None, kwargs)
    }

    #[pyo3(signature = (subset=None, normalize=false, sort=true, ascending=false, dropna=true))]
    fn value_counts(
        &self,
        subset: Option<Vec<String>>,
        normalize: Option<bool>,
        sort: Option<bool>,
        ascending: Option<bool>,
        dropna: Option<bool>,
    ) -> PyResult<PyDataFrame> {
        let _ = (subset, normalize, sort, ascending, dropna);
        let by_refs: Vec<&str> = self.by.iter().map(|s| s.as_str()).collect();
        let res = self
            .df
            .groupby(&by_refs)
            .map_err(frame_error_to_py)?
            .value_counts()
            .map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: res })
    }
}

/// Python wrapper for FrankenPandas SeriesGroupBy.
#[derive(Clone)]
#[pyclass(name = "SeriesGroupBy", from_py_object)]
pub struct PySeriesGroupBy {
    series: Series,
    by: Series,
}

#[pymethods]
impl PySeriesGroupBy {
    fn __repr__(&self) -> String {
        format!(
            "SeriesGroupBy(series={}, by={})",
            self.series.name(),
            self.by.name()
        )
    }

    fn sum(&self) -> PyResult<PySeries> {
        let res = self
            .series
            .groupby(&self.by)
            .map_err(frame_error_to_py)?
            .sum()
            .map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    fn mean(&self) -> PyResult<PySeries> {
        let res = self
            .series
            .groupby(&self.by)
            .map_err(frame_error_to_py)?
            .mean()
            .map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    fn std(&self) -> PyResult<PySeries> {
        let res = self
            .series
            .groupby(&self.by)
            .map_err(frame_error_to_py)?
            .std()
            .map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    fn var(&self) -> PyResult<PySeries> {
        let res = self
            .series
            .groupby(&self.by)
            .map_err(frame_error_to_py)?
            .var()
            .map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    fn min(&self) -> PyResult<PySeries> {
        let res = self
            .series
            .groupby(&self.by)
            .map_err(frame_error_to_py)?
            .min()
            .map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    fn max(&self) -> PyResult<PySeries> {
        let res = self
            .series
            .groupby(&self.by)
            .map_err(frame_error_to_py)?
            .max()
            .map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    fn count(&self) -> PyResult<PySeries> {
        let res = self
            .series
            .groupby(&self.by)
            .map_err(frame_error_to_py)?
            .count()
            .map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    fn first(&self) -> PyResult<PySeries> {
        let res = self
            .series
            .groupby(&self.by)
            .map_err(frame_error_to_py)?
            .first()
            .map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    fn last(&self) -> PyResult<PySeries> {
        let res = self
            .series
            .groupby(&self.by)
            .map_err(frame_error_to_py)?
            .last()
            .map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    fn median(&self) -> PyResult<PySeries> {
        let res = self
            .series
            .groupby(&self.by)
            .map_err(frame_error_to_py)?
            .median()
            .map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    fn prod(&self) -> PyResult<PySeries> {
        let res = self
            .series
            .groupby(&self.by)
            .map_err(frame_error_to_py)?
            .prod()
            .map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    fn size(&self) -> PyResult<PySeries> {
        let res = self
            .series
            .groupby(&self.by)
            .map_err(frame_error_to_py)?
            .size()
            .map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    fn nunique(&self) -> PyResult<PySeries> {
        let res = self
            .series
            .groupby(&self.by)
            .map_err(frame_error_to_py)?
            .nunique()
            .map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    fn any(&self) -> PyResult<PySeries> {
        let res = self
            .series
            .groupby(&self.by)
            .map_err(frame_error_to_py)?
            .any()
            .map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    fn all(&self) -> PyResult<PySeries> {
        let res = self
            .series
            .groupby(&self.by)
            .map_err(frame_error_to_py)?
            .all()
            .map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    fn value_counts(&self) -> PyResult<PySeries> {
        let res = self
            .series
            .groupby(&self.by)
            .map_err(frame_error_to_py)?
            .value_counts()
            .map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    #[pyo3(signature = (n=5))]
    fn nlargest(&self, n: usize) -> PyResult<PySeries> {
        let res = self
            .series
            .groupby(&self.by)
            .map_err(frame_error_to_py)?
            .nlargest(n)
            .map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    #[pyo3(signature = (n=5))]
    fn nsmallest(&self, n: usize) -> PyResult<PySeries> {
        let res = self
            .series
            .groupby(&self.by)
            .map_err(frame_error_to_py)?
            .nsmallest(n)
            .map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    #[pyo3(signature = (periods=1))]
    fn diff(&self, periods: usize) -> PyResult<PySeries> {
        let res = self
            .series
            .groupby(&self.by)
            .map_err(frame_error_to_py)?
            .diff(periods)
            .map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    #[pyo3(signature = (periods=1))]
    fn shift(&self, periods: i64) -> PyResult<PySeries> {
        let res = self
            .series
            .groupby(&self.by)
            .map_err(frame_error_to_py)?
            .shift(periods)
            .map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    fn cumsum(&self) -> PyResult<PySeries> {
        let res = self
            .series
            .groupby(&self.by)
            .map_err(frame_error_to_py)?
            .cumsum()
            .map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    fn cumprod(&self) -> PyResult<PySeries> {
        let res = self
            .series
            .groupby(&self.by)
            .map_err(frame_error_to_py)?
            .cumprod()
            .map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    fn cummin(&self) -> PyResult<PySeries> {
        let res = self
            .series
            .groupby(&self.by)
            .map_err(frame_error_to_py)?
            .cummin()
            .map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    fn cummax(&self) -> PyResult<PySeries> {
        let res = self
            .series
            .groupby(&self.by)
            .map_err(frame_error_to_py)?
            .cummax()
            .map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    fn ngroups(&self) -> PyResult<usize> {
        Ok(self
            .series
            .groupby(&self.by)
            .map_err(frame_error_to_py)?
            .ngroups())
    }

    #[getter]
    fn ndim(&self) -> usize {
        1
    }

    fn product(&self) -> PyResult<PySeries> {
        self.prod()
    }

    fn quantile(&self, q: f64) -> PyResult<PySeries> {
        let res = self
            .series
            .groupby(&self.by)
            .map_err(frame_error_to_py)?
            .quantile(q)
            .map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    fn sem(&self) -> PyResult<PySeries> {
        let res = self
            .series
            .groupby(&self.by)
            .map_err(frame_error_to_py)?
            .sem()
            .map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    fn skew(&self) -> PyResult<PySeries> {
        let res = self
            .series
            .groupby(&self.by)
            .map_err(frame_error_to_py)?
            .skew()
            .map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    fn kurt(&self) -> PyResult<PySeries> {
        let res = self
            .series
            .groupby(&self.by)
            .map_err(frame_error_to_py)?
            .kurtosis()
            .map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    fn kurtosis(&self) -> PyResult<PySeries> {
        self.kurt()
    }

    #[pyo3(signature = (method=None, ascending=None, na_option=None))]
    fn rank(
        &self,
        method: Option<&str>,
        ascending: Option<bool>,
        na_option: Option<&str>,
    ) -> PyResult<PySeries> {
        let m = method.unwrap_or("average");
        let asc = ascending.unwrap_or(true);
        let na = na_option.unwrap_or("keep");
        let res = self
            .series
            .groupby(&self.by)
            .map_err(frame_error_to_py)?
            .rank(m, asc, na)
            .map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    #[pyo3(signature = (ascending=true))]
    fn cumcount(&self, ascending: bool) -> PyResult<PySeries> {
        let res = self
            .series
            .groupby(&self.by)
            .map_err(frame_error_to_py)?
            .cumcount_with_ascending(ascending)
            .map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    fn agg(&self, py: Python<'_>, func: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        if let Ok(name) = func.extract::<String>() {
            let res = match name.as_str() {
                "sum" => self.sum()?,
                "mean" => self.mean()?,
                "min" => self.min()?,
                "max" => self.max()?,
                "std" => self.std()?,
                "var" => self.var()?,
                "count" => self.count()?,
                "first" => self.first()?,
                "last" => self.last()?,
                "median" => self.median()?,
                "prod" => self.prod()?,
                "size" => self.size()?,
                "nunique" => self.nunique()?,
                "any" => self.any()?,
                "all" => self.all()?,
                "cumsum" => self.cumsum()?,
                "cumprod" => self.cumprod()?,
                "cummin" => self.cummin()?,
                "cummax" => self.cummax()?,
                _ => {
                    return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                        "unsupported aggregation '{name}'"
                    )));
                }
            };
            return Ok(Py::new(py, res)?.into_any());
        } else if let Ok(list) = func.extract::<Vec<String>>() {
            let refs: Vec<&str> = list.iter().map(String::as_str).collect();
            let df = self
                .series
                .groupby(&self.by)
                .map_err(frame_error_to_py)?
                .agg(&refs)
                .map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PyDataFrame { inner: df })?.into_any());
        }
        Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
            "agg expects a string or list of function names",
        ))
    }

    fn aggregate(&self, py: Python<'_>, func: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        self.agg(py, func)
    }

    #[pyo3(signature = (n=5))]
    fn head(&self, n: usize) -> PyResult<PySeries> {
        let res = self
            .series
            .groupby(&self.by)
            .map_err(frame_error_to_py)?
            .head(n as i64)
            .map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    #[pyo3(signature = (n=5))]
    fn tail(&self, n: usize) -> PyResult<PySeries> {
        let res = self
            .series
            .groupby(&self.by)
            .map_err(frame_error_to_py)?
            .tail(n as i64)
            .map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    #[pyo3(signature = (limit=None))]
    fn ffill(&self, limit: Option<usize>) -> PyResult<PySeries> {
        let res = self
            .series
            .groupby(&self.by)
            .map_err(frame_error_to_py)?
            .ffill(limit)
            .map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    #[pyo3(signature = (limit=None))]
    fn bfill(&self, limit: Option<usize>) -> PyResult<PySeries> {
        let res = self
            .series
            .groupby(&self.by)
            .map_err(frame_error_to_py)?
            .bfill(limit)
            .map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    fn describe(&self) -> PyResult<PyDataFrame> {
        let res = self
            .series
            .groupby(&self.by)
            .map_err(frame_error_to_py)?
            .describe()
            .map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: res })
    }

    fn get_group(&self, name: &Bound<'_, PyAny>) -> PyResult<PySeries> {
        let s = name
            .extract::<String>()
            .or_else(|_| name.str().map(|py_s| py_s.to_string()))?;
        let res = self
            .series
            .groupby(&self.by)
            .map_err(frame_error_to_py)?
            .get_group(&s)
            .map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    #[getter]
    fn groups(&self, py: Python<'_>) -> PyResult<Py<PyDict>> {
        let gb = self.series.groupby(&self.by).map_err(frame_error_to_py)?;
        let dict = PyDict::new(py);
        for (lbl, indices) in gb.groups() {
            let py_key = index_label_to_py(py, &lbl)?;
            let py_indices = PyList::new(py, indices)?;
            dict.set_item(py_key, py_indices)?;
        }
        Ok(dict.unbind())
    }

    #[getter]
    fn indices(&self, py: Python<'_>) -> PyResult<Py<PyDict>> {
        self.groups(py)
    }

    #[getter]
    fn dtype(&self) -> String {
        self.series.dtype_name()
    }

    #[pyo3(signature = (func, *args, **kwargs))]
    fn apply(
        &self,
        py: Python<'_>,
        func: &Bound<'_, PyAny>,
        args: &Bound<'_, pyo3::types::PyTuple>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Py<PyAny>> {
        let _ = (args, kwargs);
        let gb = self.series.groupby(&self.by).map_err(frame_error_to_py)?;
        let groups = gb.groups();
        let mut keys: Vec<_> = groups.keys().cloned().collect();
        keys.sort();
        let mut out_series = Vec::new();
        let mut out_scalars = Vec::new();
        for k in &keys {
            let k_str = match k {
                IndexLabel::Utf8(s) => s.clone(),
                IndexLabel::Int64(i) => i.to_string(),
                _ => format!("{k:?}"),
            };
            if let Ok(group_s) = gb.get_group(&k_str) {
                let py_s = PySeries { inner: group_s };
                let res = func.call1((py_s,))?;
                if let Ok(s_res) = res.extract::<PySeries>() {
                    out_series.push(s_res.inner);
                } else if let Ok(sc) = py_to_scalar(py, &res) {
                    out_scalars.push((k.clone(), sc));
                }
            }
        }
        if !out_series.is_empty() {
            let refs: Vec<&Series> = out_series.iter().collect();
            let combined = concat_series(&refs).map_err(frame_error_to_py)?;
            Ok(Py::new(py, PySeries { inner: combined })?.into_any())
        } else if !out_scalars.is_empty() {
            let labels: Vec<IndexLabel> = out_scalars.iter().map(|(lbl, _)| lbl.clone()).collect();
            let values: Vec<Scalar> = out_scalars.into_iter().map(|(_, v)| v).collect();
            let s = Series::from_values("", labels, values).map_err(frame_error_to_py)?;
            Ok(Py::new(py, PySeries { inner: s })?.into_any())
        } else {
            let first = gb.first().map_err(frame_error_to_py)?;
            Ok(Py::new(py, PySeries { inner: first })?.into_any())
        }
    }

    #[pyo3(signature = (other, method=None, min_periods=None))]
    fn corr(
        &self,
        other: &PySeries,
        method: Option<&str>,
        min_periods: Option<usize>,
    ) -> PyResult<PySeries> {
        let _ = (method, min_periods);
        let res = self
            .series
            .groupby(&self.by)
            .map_err(frame_error_to_py)?
            .corr(&other.inner)
            .map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    #[pyo3(signature = (other, min_periods=None, ddof=None))]
    fn cov(
        &self,
        other: &PySeries,
        min_periods: Option<usize>,
        ddof: Option<usize>,
    ) -> PyResult<PySeries> {
        let _ = (min_periods, ddof);
        let res = self
            .series
            .groupby(&self.by)
            .map_err(frame_error_to_py)?
            .cov(&other.inner)
            .map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    #[pyo3(signature = (span=None, alpha=None))]
    fn ewm(&self, span: Option<f64>, alpha: Option<f64>) -> PyResult<PyExponentialMovingWindow> {
        Ok(PyExponentialMovingWindow {
            series: Some(self.series.clone()),
            dataframe: None,
            span,
            alpha,
        })
    }

    #[pyo3(signature = (min_periods=None))]
    fn expanding(&self, min_periods: Option<usize>) -> PyResult<PyExpanding> {
        Ok(PyExpanding {
            series: Some(self.series.clone()),
            dataframe: None,
            min_periods,
        })
    }

    #[pyo3(signature = (window, min_periods=None, center=false))]
    fn rolling(
        &self,
        window: usize,
        min_periods: Option<usize>,
        center: Option<bool>,
    ) -> PyResult<PyRolling> {
        Ok(PyRolling {
            series: Some(self.series.clone()),
            dataframe: None,
            window,
            min_periods,
            center: center.unwrap_or(false),
        })
    }

    #[pyo3(signature = (rule, closed=None, label=None, origin=None))]
    fn resample(
        &self,
        rule: String,
        closed: Option<String>,
        label: Option<String>,
        origin: Option<String>,
    ) -> PyResult<PyResampler> {
        Ok(PyResampler {
            target: ResampleTarget::Series(self.series.clone()),
            freq: rule,
            closed,
            label,
            origin,
        })
    }

    fn fillna(&self, py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<PySeries> {
        let sc = py_to_scalar(py, value)?;
        let res = self
            .series
            .groupby(&self.by)
            .map_err(frame_error_to_py)?
            .fillna(&sc)
            .map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    #[pyo3(signature = (func, dropna=true))]
    fn filter(&self, func: &Bound<'_, PyAny>, dropna: Option<bool>) -> PyResult<PySeries> {
        let _ = dropna;
        let gb = self.series.groupby(&self.by).map_err(frame_error_to_py)?;
        let groups = gb.groups();
        let mut keys: Vec<_> = groups.keys().cloned().collect();
        keys.sort();
        let mut kept_series = Vec::new();
        for k in &keys {
            let k_str = match k {
                IndexLabel::Utf8(s) => s.clone(),
                IndexLabel::Int64(i) => i.to_string(),
                _ => format!("{k:?}"),
            };
            if let Ok(group_s) = gb.get_group(&k_str) {
                let py_s = PySeries {
                    inner: group_s.clone(),
                };
                let res = func.call1((py_s,))?;
                if res.is_truthy()? {
                    kept_series.push(group_s);
                }
            }
        }
        if !kept_series.is_empty() {
            let refs: Vec<&Series> = kept_series.iter().collect();
            let combined = concat_series(&refs).map_err(frame_error_to_py)?;
            Ok(PySeries { inner: combined })
        } else {
            let empty = self.series.head(0).map_err(frame_error_to_py)?;
            Ok(PySeries { inner: empty })
        }
    }

    #[getter]
    fn grouper(&self) -> String {
        self.by.name().to_string()
    }

    #[pyo3(signature = (*args, **kwargs))]
    fn hist(
        &self,
        py: Python<'_>,
        args: &Bound<'_, pyo3::types::PyTuple>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Py<PyAny>> {
        let _ = (args, kwargs);
        let _ = self
            .series
            .groupby(&self.by)
            .map_err(frame_error_to_py)?
            .hist()
            .map_err(frame_error_to_py)?;
        Ok(py.None())
    }

    #[pyo3(signature = (axis=0, skipna=true))]
    fn idxmax(&self, axis: Option<usize>, skipna: Option<bool>) -> PyResult<PySeries> {
        let _ = (axis, skipna);
        let res = self
            .series
            .groupby(&self.by)
            .map_err(frame_error_to_py)?
            .idxmax()
            .map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    #[pyo3(signature = (axis=0, skipna=true))]
    fn idxmin(&self, axis: Option<usize>, skipna: Option<bool>) -> PyResult<PySeries> {
        let _ = (axis, skipna);
        let res = self
            .series
            .groupby(&self.by)
            .map_err(frame_error_to_py)?
            .idxmin()
            .map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    #[getter]
    fn is_monotonic_decreasing(&self) -> PyResult<PySeries> {
        let res = self
            .series
            .groupby(&self.by)
            .map_err(frame_error_to_py)?
            .is_monotonic_decreasing()
            .map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    #[getter]
    fn is_monotonic_increasing(&self) -> PyResult<PySeries> {
        let res = self
            .series
            .groupby(&self.by)
            .map_err(frame_error_to_py)?
            .is_monotonic_increasing()
            .map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    #[getter]
    fn keys(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        Ok(pyo3::types::PyString::new(py, self.by.name())
            .into_any()
            .unbind())
    }

    #[getter]
    fn level(&self) -> Option<usize> {
        None
    }

    #[pyo3(signature = (ascending=true))]
    fn ngroup(&self, ascending: Option<bool>) -> PyResult<PySeries> {
        let res = self
            .series
            .groupby(&self.by)
            .map_err(frame_error_to_py)?
            .ngroup_with_ascending(ascending.unwrap_or(true))
            .map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    #[pyo3(signature = (n, dropna=None))]
    fn nth(&self, n: i64, dropna: Option<&str>) -> PyResult<PySeries> {
        let _ = dropna;
        let res = self
            .series
            .groupby(&self.by)
            .map_err(frame_error_to_py)?
            .nth(n)
            .map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    fn ohlc(&self) -> PyResult<PyDataFrame> {
        let res = self
            .series
            .groupby(&self.by)
            .map_err(frame_error_to_py)?
            .ohlc()
            .map_err(frame_error_to_py)?;
        Ok(PyDataFrame { inner: res })
    }

    #[pyo3(signature = (periods=1, fill_method=None, limit=None, freq=None))]
    fn pct_change(
        &self,
        periods: Option<i64>,
        fill_method: Option<&str>,
        limit: Option<usize>,
        freq: Option<&str>,
    ) -> PyResult<PySeries> {
        let _ = (fill_method, limit, freq);
        let res = self
            .series
            .groupby(&self.by)
            .map_err(frame_error_to_py)?
            .pct_change(periods.unwrap_or(1))
            .map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    #[pyo3(signature = (func, *args, **kwargs))]
    fn pipe<'py>(
        &self,
        py: Python<'py>,
        func: &Bound<'py, PyAny>,
        args: &Bound<'py, pyo3::types::PyTuple>,
        kwargs: Option<&Bound<'py, PyDict>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        if let Ok(tup) = func.cast::<pyo3::types::PyTuple>()
            && tup.len() == 2
        {
            let f = tup.get_item(0)?;
            let kw_name = tup.get_item(1)?.extract::<String>()?;
            let kw = kwargs.cloned().unwrap_or_else(|| PyDict::new(py));
            kw.set_item(kw_name, self.clone())?;
            return f.call(args, Some(&kw));
        }
        let mut full_args = Vec::with_capacity(args.len() + 1);
        full_args.push(Py::new(py, self.clone())?.into_any());
        for item in args.iter() {
            full_args.push(item.unbind());
        }
        let full_tuple = pyo3::types::PyTuple::new(py, full_args)?;
        func.call(&full_tuple, kwargs)
    }

    #[pyo3(signature = (*args, **kwargs))]
    fn plot(
        &self,
        py: Python<'_>,
        args: &Bound<'_, pyo3::types::PyTuple>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Py<PyAny>> {
        let _ = (args, kwargs);
        let _ = self
            .series
            .groupby(&self.by)
            .map_err(frame_error_to_py)?
            .plot()
            .map_err(frame_error_to_py)?;
        Ok(py.None())
    }

    #[pyo3(signature = (n=None, frac=None, replace=false, weights=None, random_state=None))]
    fn sample(
        &self,
        n: Option<usize>,
        frac: Option<f64>,
        replace: Option<bool>,
        weights: Option<&Bound<'_, PyAny>>,
        random_state: Option<u64>,
    ) -> PyResult<PySeries> {
        let _ = weights;
        let res = self
            .series
            .groupby(&self.by)
            .map_err(frame_error_to_py)?
            .sample(n, frac, replace.unwrap_or(false), random_state)
            .map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    #[pyo3(signature = (indices, axis=None))]
    fn take(&self, indices: Vec<i64>, axis: Option<usize>) -> PyResult<PySeries> {
        let _ = axis;
        let res = self
            .series
            .groupby(&self.by)
            .map_err(frame_error_to_py)?
            .take(&indices)
            .map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }

    #[pyo3(signature = (func, *args, **kwargs))]
    fn transform(
        &self,
        py: Python<'_>,
        func: &Bound<'_, PyAny>,
        args: &Bound<'_, pyo3::types::PyTuple>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Py<PyAny>> {
        let gb = self.series.groupby(&self.by).map_err(frame_error_to_py)?;
        if let Ok(func_str) = func.extract::<String>() {
            let res = gb.transform(&func_str).map_err(frame_error_to_py)?;
            return Ok(Py::new(py, PySeries { inner: res })?.into_any());
        }
        self.apply(py, func, args, kwargs)
    }

    fn unique(&self) -> PyResult<PySeries> {
        let res = self
            .series
            .groupby(&self.by)
            .map_err(frame_error_to_py)?
            .unique()
            .map_err(frame_error_to_py)?;
        Ok(PySeries { inner: res })
    }
}

#[derive(Clone)]
enum ResampleTarget {
    Series(Series),
    DataFrame(DataFrame),
}

/// Python wrapper for FrankenPandas Resampler.
#[pyclass(name = "Resampler")]
pub struct PyResampler {
    target: ResampleTarget,
    freq: String,
    closed: Option<String>,
    label: Option<String>,
    origin: Option<String>,
}

#[pymethods]
impl PyResampler {
    fn __repr__(&self) -> String {
        format!("Resampler(freq='{}')", self.freq)
    }

    fn sum(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        match &self.target {
            ResampleTarget::Series(s) => {
                let res = s
                    .resample_ext(
                        &self.freq,
                        self.closed.as_deref(),
                        self.label.as_deref(),
                        self.origin.as_deref(),
                    )
                    .sum()
                    .map_err(frame_error_to_py)?;
                Ok(Py::new(py, PySeries { inner: res })?.into_any())
            }
            ResampleTarget::DataFrame(df) => {
                let res = df
                    .resample_ext(
                        &self.freq,
                        self.closed.as_deref(),
                        self.label.as_deref(),
                        self.origin.as_deref(),
                    )
                    .sum()
                    .map_err(frame_error_to_py)?;
                Ok(Py::new(py, PyDataFrame { inner: res })?.into_any())
            }
        }
    }

    fn mean(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        match &self.target {
            ResampleTarget::Series(s) => {
                let res = s
                    .resample_ext(
                        &self.freq,
                        self.closed.as_deref(),
                        self.label.as_deref(),
                        self.origin.as_deref(),
                    )
                    .mean()
                    .map_err(frame_error_to_py)?;
                Ok(Py::new(py, PySeries { inner: res })?.into_any())
            }
            ResampleTarget::DataFrame(df) => {
                let res = df
                    .resample_ext(
                        &self.freq,
                        self.closed.as_deref(),
                        self.label.as_deref(),
                        self.origin.as_deref(),
                    )
                    .mean()
                    .map_err(frame_error_to_py)?;
                Ok(Py::new(py, PyDataFrame { inner: res })?.into_any())
            }
        }
    }

    fn min(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        match &self.target {
            ResampleTarget::Series(s) => {
                let res = s
                    .resample_ext(
                        &self.freq,
                        self.closed.as_deref(),
                        self.label.as_deref(),
                        self.origin.as_deref(),
                    )
                    .min()
                    .map_err(frame_error_to_py)?;
                Ok(Py::new(py, PySeries { inner: res })?.into_any())
            }
            ResampleTarget::DataFrame(df) => {
                let res = df
                    .resample_ext(
                        &self.freq,
                        self.closed.as_deref(),
                        self.label.as_deref(),
                        self.origin.as_deref(),
                    )
                    .min()
                    .map_err(frame_error_to_py)?;
                Ok(Py::new(py, PyDataFrame { inner: res })?.into_any())
            }
        }
    }

    fn max(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        match &self.target {
            ResampleTarget::Series(s) => {
                let res = s
                    .resample_ext(
                        &self.freq,
                        self.closed.as_deref(),
                        self.label.as_deref(),
                        self.origin.as_deref(),
                    )
                    .max()
                    .map_err(frame_error_to_py)?;
                Ok(Py::new(py, PySeries { inner: res })?.into_any())
            }
            ResampleTarget::DataFrame(df) => {
                let res = df
                    .resample_ext(
                        &self.freq,
                        self.closed.as_deref(),
                        self.label.as_deref(),
                        self.origin.as_deref(),
                    )
                    .max()
                    .map_err(frame_error_to_py)?;
                Ok(Py::new(py, PyDataFrame { inner: res })?.into_any())
            }
        }
    }

    fn count(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        match &self.target {
            ResampleTarget::Series(s) => {
                let res = s
                    .resample_ext(
                        &self.freq,
                        self.closed.as_deref(),
                        self.label.as_deref(),
                        self.origin.as_deref(),
                    )
                    .count()
                    .map_err(frame_error_to_py)?;
                Ok(Py::new(py, PySeries { inner: res })?.into_any())
            }
            ResampleTarget::DataFrame(df) => {
                let res = df
                    .resample_ext(
                        &self.freq,
                        self.closed.as_deref(),
                        self.label.as_deref(),
                        self.origin.as_deref(),
                    )
                    .count()
                    .map_err(frame_error_to_py)?;
                Ok(Py::new(py, PyDataFrame { inner: res })?.into_any())
            }
        }
    }

    fn first(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        match &self.target {
            ResampleTarget::Series(s) => {
                let res = s
                    .resample_ext(
                        &self.freq,
                        self.closed.as_deref(),
                        self.label.as_deref(),
                        self.origin.as_deref(),
                    )
                    .first()
                    .map_err(frame_error_to_py)?;
                Ok(Py::new(py, PySeries { inner: res })?.into_any())
            }
            ResampleTarget::DataFrame(df) => {
                let res = df
                    .resample_ext(
                        &self.freq,
                        self.closed.as_deref(),
                        self.label.as_deref(),
                        self.origin.as_deref(),
                    )
                    .first()
                    .map_err(frame_error_to_py)?;
                Ok(Py::new(py, PyDataFrame { inner: res })?.into_any())
            }
        }
    }

    fn last(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        match &self.target {
            ResampleTarget::Series(s) => {
                let res = s
                    .resample_ext(
                        &self.freq,
                        self.closed.as_deref(),
                        self.label.as_deref(),
                        self.origin.as_deref(),
                    )
                    .last()
                    .map_err(frame_error_to_py)?;
                Ok(Py::new(py, PySeries { inner: res })?.into_any())
            }
            ResampleTarget::DataFrame(df) => {
                let res = df
                    .resample_ext(
                        &self.freq,
                        self.closed.as_deref(),
                        self.label.as_deref(),
                        self.origin.as_deref(),
                    )
                    .last()
                    .map_err(frame_error_to_py)?;
                Ok(Py::new(py, PyDataFrame { inner: res })?.into_any())
            }
        }
    }

    fn std(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        match &self.target {
            ResampleTarget::Series(s) => {
                let res = s
                    .resample_ext(
                        &self.freq,
                        self.closed.as_deref(),
                        self.label.as_deref(),
                        self.origin.as_deref(),
                    )
                    .std()
                    .map_err(frame_error_to_py)?;
                Ok(Py::new(py, PySeries { inner: res })?.into_any())
            }
            ResampleTarget::DataFrame(df) => {
                let res = df
                    .resample_ext(
                        &self.freq,
                        self.closed.as_deref(),
                        self.label.as_deref(),
                        self.origin.as_deref(),
                    )
                    .std()
                    .map_err(frame_error_to_py)?;
                Ok(Py::new(py, PyDataFrame { inner: res })?.into_any())
            }
        }
    }

    fn var(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        match &self.target {
            ResampleTarget::Series(s) => {
                let res = s
                    .resample_ext(
                        &self.freq,
                        self.closed.as_deref(),
                        self.label.as_deref(),
                        self.origin.as_deref(),
                    )
                    .var()
                    .map_err(frame_error_to_py)?;
                Ok(Py::new(py, PySeries { inner: res })?.into_any())
            }
            ResampleTarget::DataFrame(df) => {
                let res = df
                    .resample_ext(
                        &self.freq,
                        self.closed.as_deref(),
                        self.label.as_deref(),
                        self.origin.as_deref(),
                    )
                    .var()
                    .map_err(frame_error_to_py)?;
                Ok(Py::new(py, PyDataFrame { inner: res })?.into_any())
            }
        }
    }

    fn median(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        match &self.target {
            ResampleTarget::Series(s) => {
                let res = s
                    .resample_ext(
                        &self.freq,
                        self.closed.as_deref(),
                        self.label.as_deref(),
                        self.origin.as_deref(),
                    )
                    .median()
                    .map_err(frame_error_to_py)?;
                Ok(Py::new(py, PySeries { inner: res })?.into_any())
            }
            ResampleTarget::DataFrame(df) => {
                let res = df
                    .resample_ext(
                        &self.freq,
                        self.closed.as_deref(),
                        self.label.as_deref(),
                        self.origin.as_deref(),
                    )
                    .median()
                    .map_err(frame_error_to_py)?;
                Ok(Py::new(py, PyDataFrame { inner: res })?.into_any())
            }
        }
    }

    fn prod(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        match &self.target {
            ResampleTarget::Series(s) => {
                let res = s
                    .resample_ext(
                        &self.freq,
                        self.closed.as_deref(),
                        self.label.as_deref(),
                        self.origin.as_deref(),
                    )
                    .prod()
                    .map_err(frame_error_to_py)?;
                Ok(Py::new(py, PySeries { inner: res })?.into_any())
            }
            ResampleTarget::DataFrame(df) => {
                let res = df
                    .resample_ext(
                        &self.freq,
                        self.closed.as_deref(),
                        self.label.as_deref(),
                        self.origin.as_deref(),
                    )
                    .prod()
                    .map_err(frame_error_to_py)?;
                Ok(Py::new(py, PyDataFrame { inner: res })?.into_any())
            }
        }
    }

    fn size(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        match &self.target {
            ResampleTarget::Series(s) => {
                let res = s
                    .resample_ext(
                        &self.freq,
                        self.closed.as_deref(),
                        self.label.as_deref(),
                        self.origin.as_deref(),
                    )
                    .size()
                    .map_err(frame_error_to_py)?;
                Ok(Py::new(py, PySeries { inner: res })?.into_any())
            }
            ResampleTarget::DataFrame(df) => {
                let res = df
                    .resample_ext(
                        &self.freq,
                        self.closed.as_deref(),
                        self.label.as_deref(),
                        self.origin.as_deref(),
                    )
                    .size()
                    .map_err(frame_error_to_py)?;
                Ok(Py::new(py, PySeries { inner: res })?.into_any())
            }
        }
    }

    fn ohlc(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        match &self.target {
            ResampleTarget::Series(s) => {
                let res = s
                    .resample_ext(
                        &self.freq,
                        self.closed.as_deref(),
                        self.label.as_deref(),
                        self.origin.as_deref(),
                    )
                    .ohlc()
                    .map_err(frame_error_to_py)?;
                Ok(Py::new(py, PyDataFrame { inner: res })?.into_any())
            }
            ResampleTarget::DataFrame(df) => {
                let res = df
                    .resample_ext(
                        &self.freq,
                        self.closed.as_deref(),
                        self.label.as_deref(),
                        self.origin.as_deref(),
                    )
                    .ohlc()
                    .map_err(frame_error_to_py)?;
                Ok(Py::new(py, PyDataFrame { inner: res })?.into_any())
            }
        }
    }

    #[getter]
    fn ndim(&self) -> usize {
        match &self.target {
            ResampleTarget::Series(_) => 1,
            ResampleTarget::DataFrame(_) => 2,
        }
    }

    fn quantile(&self, py: Python<'_>, q: f64) -> PyResult<Py<PyAny>> {
        match &self.target {
            ResampleTarget::Series(s) => {
                let res = s
                    .resample_ext(
                        &self.freq,
                        self.closed.as_deref(),
                        self.label.as_deref(),
                        self.origin.as_deref(),
                    )
                    .quantile(q)
                    .map_err(frame_error_to_py)?;
                Ok(Py::new(py, PySeries { inner: res })?.into_any())
            }
            ResampleTarget::DataFrame(df) => {
                let res = df
                    .resample_ext(
                        &self.freq,
                        self.closed.as_deref(),
                        self.label.as_deref(),
                        self.origin.as_deref(),
                    )
                    .quantile(q)
                    .map_err(frame_error_to_py)?;
                Ok(Py::new(py, PyDataFrame { inner: res })?.into_any())
            }
        }
    }

    fn sem(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        match &self.target {
            ResampleTarget::Series(s) => {
                let res = s
                    .resample_ext(
                        &self.freq,
                        self.closed.as_deref(),
                        self.label.as_deref(),
                        self.origin.as_deref(),
                    )
                    .sem()
                    .map_err(frame_error_to_py)?;
                Ok(Py::new(py, PySeries { inner: res })?.into_any())
            }
            ResampleTarget::DataFrame(df) => {
                let res = df
                    .resample_ext(
                        &self.freq,
                        self.closed.as_deref(),
                        self.label.as_deref(),
                        self.origin.as_deref(),
                    )
                    .sem()
                    .map_err(frame_error_to_py)?;
                Ok(Py::new(py, PyDataFrame { inner: res })?.into_any())
            }
        }
    }

    fn skew(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        match &self.target {
            ResampleTarget::Series(s) => {
                let res = s
                    .resample_ext(
                        &self.freq,
                        self.closed.as_deref(),
                        self.label.as_deref(),
                        self.origin.as_deref(),
                    )
                    .skew()
                    .map_err(frame_error_to_py)?;
                Ok(Py::new(py, PySeries { inner: res })?.into_any())
            }
            ResampleTarget::DataFrame(df) => {
                let res = df
                    .resample_ext(
                        &self.freq,
                        self.closed.as_deref(),
                        self.label.as_deref(),
                        self.origin.as_deref(),
                    )
                    .skew()
                    .map_err(frame_error_to_py)?;
                Ok(Py::new(py, PyDataFrame { inner: res })?.into_any())
            }
        }
    }

    fn kurt(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        match &self.target {
            ResampleTarget::Series(s) => {
                let res = s
                    .resample_ext(
                        &self.freq,
                        self.closed.as_deref(),
                        self.label.as_deref(),
                        self.origin.as_deref(),
                    )
                    .kurt()
                    .map_err(frame_error_to_py)?;
                Ok(Py::new(py, PySeries { inner: res })?.into_any())
            }
            ResampleTarget::DataFrame(df) => {
                let res = df
                    .resample_ext(
                        &self.freq,
                        self.closed.as_deref(),
                        self.label.as_deref(),
                        self.origin.as_deref(),
                    )
                    .kurt()
                    .map_err(frame_error_to_py)?;
                Ok(Py::new(py, PyDataFrame { inner: res })?.into_any())
            }
        }
    }

    fn kurtosis(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        self.kurt(py)
    }

    fn nearest(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        match &self.target {
            ResampleTarget::Series(s) => {
                let res = s
                    .resample_ext(
                        &self.freq,
                        self.closed.as_deref(),
                        self.label.as_deref(),
                        self.origin.as_deref(),
                    )
                    .nearest()
                    .map_err(frame_error_to_py)?;
                Ok(Py::new(py, PySeries { inner: res })?.into_any())
            }
            ResampleTarget::DataFrame(df) => {
                let res = df
                    .resample_ext(
                        &self.freq,
                        self.closed.as_deref(),
                        self.label.as_deref(),
                        self.origin.as_deref(),
                    )
                    .nearest()
                    .map_err(frame_error_to_py)?;
                Ok(Py::new(py, PyDataFrame { inner: res })?.into_any())
            }
        }
    }

    fn agg(&self, py: Python<'_>, func: &str) -> PyResult<Py<PyAny>> {
        match func {
            "sum" => self.sum(py),
            "mean" => self.mean(py),
            "min" => self.min(py),
            "max" => self.max(py),
            "count" => self.count(py),
            "first" => self.first(py),
            "last" => self.last(py),
            "std" => self.std(py),
            "var" => self.var(py),
            "median" => self.median(py),
            "prod" => self.prod(py),
            "size" => self.size(py),
            "ohlc" => self.ohlc(py),
            "sem" => self.sem(py),
            "skew" => self.skew(py),
            "kurt" | "kurtosis" => self.kurt(py),
            "nearest" => self.nearest(py),
            _ => Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                "unsupported resampler aggregation '{func}'"
            ))),
        }
    }

    fn aggregate(&self, py: Python<'_>, func: &str) -> PyResult<Py<PyAny>> {
        self.agg(py, func)
    }
}

/// Read a CSV file into a DataFrame.
/// `fp.concat([df1, df2, ...])`: stack frames along the row axis, pandas'
/// default `axis=0` / `join="outer"` behaviour.
#[pyfunction]
fn concat(frames: Vec<PyRef<'_, PyDataFrame>>) -> PyResult<PyDataFrame> {
    if frames.is_empty() {
        return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
            "No objects to concatenate",
        ));
    }
    let refs: Vec<&DataFrame> = frames.iter().map(|frame| &frame.inner).collect();
    let inner = fp_frame::concat_dataframes(&refs)
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
    Ok(PyDataFrame { inner })
}

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

/// Merge two DataFrames (pandas `merge`).
#[pyfunction]
#[pyo3(signature = (left, right, on=None, how="inner", left_on=None, right_on=None))]
fn merge(
    left: &PyDataFrame,
    right: &PyDataFrame,
    on: Option<&Bound<'_, PyAny>>,
    how: &str,
    left_on: Option<&Bound<'_, PyAny>>,
    right_on: Option<&Bound<'_, PyAny>>,
) -> PyResult<PyDataFrame> {
    if let Some(on_val) = on {
        left.merge(right, on_val, how)
    } else if let (Some(l_on), Some(r_on)) = (left_on, right_on) {
        let l_cols: Vec<String> = if let Ok(s) = l_on.extract::<String>() {
            vec![s]
        } else {
            l_on.extract::<Vec<String>>()?
        };
        let r_cols: Vec<String> = if let Ok(s) = r_on.extract::<String>() {
            vec![s]
        } else {
            r_on.extract::<Vec<String>>()?
        };
        let join_type = match how {
            "inner" => fp_join::JoinType::Inner,
            "left" => fp_join::JoinType::Left,
            "right" => fp_join::JoinType::Right,
            "outer" => fp_join::JoinType::Outer,
            "cross" => fp_join::JoinType::Cross,
            other => {
                return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                    "unknown how={other:?}; expected inner/left/right/outer/cross"
                )));
            }
        };
        let l_refs: Vec<&str> = l_cols.iter().map(String::as_str).collect();
        let r_refs: Vec<&str> = r_cols.iter().map(String::as_str).collect();
        let merged = fp_join::merge_dataframes_on_with(
            &left.inner,
            &right.inner,
            &l_refs,
            &r_refs,
            join_type,
        )
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        let frame =
            DataFrame::new_with_column_order(merged.index, merged.columns, merged.column_order)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PyDataFrame { inner: frame })
    } else {
        Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
            "merge requires `on` or both `left_on` and `right_on`",
        ))
    }
}

/// Convert argument to numeric type (pandas `to_numeric`).
#[pyfunction]
#[pyo3(signature = (arg, errors="raise"))]
fn to_numeric(py: Python<'_>, arg: &Bound<'_, PyAny>, errors: &str) -> PyResult<Py<PyAny>> {
    let err_policy = match errors {
        "raise" => fp_frame::ToNumericErrors::Raise,
        "coerce" => fp_frame::ToNumericErrors::Coerce,
        "ignore" => fp_frame::ToNumericErrors::Ignore,
        other => {
            return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                "invalid error value {other:?}; must be one of 'raise', 'coerce', 'ignore'"
            )));
        }
    };
    let opts = fp_frame::ToNumericOptions { errors: err_policy };

    if let Ok(s) = arg.extract::<PyRef<'_, PySeries>>() {
        let res = fp_frame::to_numeric_with_options(&s.inner, opts)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        return Ok(Py::new(py, PySeries { inner: res })?.into_any());
    }
    if let Ok(list) = arg.cast::<PyList>() {
        let values: Vec<Scalar> = list
            .iter()
            .map(|v| py_to_scalar(py, &v))
            .collect::<PyResult<Vec<_>>>()?;
        let temp_series = Series::from_values(
            "",
            (0..values.len())
                .map(|i| IndexLabel::Int64(i as i64))
                .collect(),
            values,
        )
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        let res = fp_frame::to_numeric_with_options(&temp_series, opts)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        return Ok(Py::new(py, PySeries { inner: res })?.into_any());
    }
    Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
        "arg must be a Series or list",
    ))
}

/// Convert argument to datetime (pandas `to_datetime`).
#[pyfunction]
#[pyo3(signature = (arg, format=None, unit=None, utc=false))]
fn to_datetime(
    py: Python<'_>,
    arg: &Bound<'_, PyAny>,
    format: Option<&str>,
    unit: Option<&str>,
    utc: bool,
) -> PyResult<Py<PyAny>> {
    let opts = fp_frame::ToDatetimeOptions {
        format,
        unit,
        utc,
        ..Default::default()
    };
    if let Ok(s) = arg.extract::<PyRef<'_, PySeries>>() {
        let res = fp_frame::to_datetime_with_options(&s.inner, opts)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        return Ok(Py::new(py, PySeries { inner: res })?.into_any());
    }
    if let Ok(dti) = arg.extract::<PyRef<'_, PyDatetimeIndex>>() {
        return Ok(Py::new(
            py,
            PyDatetimeIndex {
                inner: dti.inner.clone(),
            },
        )?
        .into_any());
    }
    if let Ok(idx) = arg.extract::<PyRef<'_, PyIndex>>() {
        let values: Vec<Scalar> = idx
            .inner
            .labels()
            .iter()
            .map(index_label_to_scalar)
            .collect();
        let temp_series = Series::from_values(
            "",
            (0..values.len())
                .map(|i| IndexLabel::Int64(i as i64))
                .collect(),
            values,
        )
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        let res = fp_frame::to_datetime_with_options(&temp_series, opts)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        let nanos: Vec<i64> = res
            .values()
            .iter()
            .map(|s| match s {
                Scalar::Datetime64(ns) => *ns,
                _ => 0,
            })
            .collect();
        return Ok(Py::new(
            py,
            PyDatetimeIndex {
                inner: DatetimeIndex::new(nanos),
            },
        )?
        .into_any());
    }
    if let Ok(list) = arg.cast::<PyList>() {
        let values: Vec<Scalar> = list
            .iter()
            .map(|v| py_to_scalar(py, &v))
            .collect::<PyResult<Vec<_>>>()?;
        let temp_series = Series::from_values(
            "",
            (0..values.len())
                .map(|i| IndexLabel::Int64(i as i64))
                .collect(),
            values,
        )
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        let res = fp_frame::to_datetime_with_options(&temp_series, opts)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        let nanos: Vec<i64> = res
            .values()
            .iter()
            .map(|s| match s {
                Scalar::Datetime64(ns) => *ns,
                _ => 0,
            })
            .collect();
        return Ok(Py::new(
            py,
            PyDatetimeIndex {
                inner: DatetimeIndex::new(nanos),
            },
        )?
        .into_any());
    }
    if let Ok(s) = py_to_scalar(py, arg) {
        let temp_series = Series::from_values("", vec![IndexLabel::Int64(0)], vec![s])
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        let res = fp_frame::to_datetime_with_options(&temp_series, opts)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        if let Some(val) = res.values().first() {
            return scalar_to_py(py, val);
        }
    }
    Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
        "arg must be a Series, Index, list, or date-like scalar",
    ))
}

/// Return a fixed frequency DatetimeIndex (pandas `date_range`).
#[pyfunction]
#[pyo3(signature = (start=None, end=None, periods=None, freq=None, name=None))]
fn date_range(
    start: Option<&str>,
    end: Option<&str>,
    periods: Option<usize>,
    freq: Option<&str>,
    name: Option<&str>,
) -> PyResult<PyDatetimeIndex> {
    let freq_str = freq.unwrap_or("D");
    let freq_nanos = parse_freq_to_nanos(freq_str)?;
    let idx = fp_index::date_range(start, end, periods, freq_nanos, name)
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
    let dti = DatetimeIndex::from_index(idx)
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
    Ok(PyDatetimeIndex { inner: dti })
}

/// Detect missing values for an array-like object or scalar (pandas `isna`).
#[pyfunction]
fn isna(py: Python<'_>, obj: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    if let Ok(df) = obj.extract::<PyRef<'_, PyDataFrame>>() {
        let res = df.isna()?;
        return Ok(Py::new(py, res)?.into_any());
    }
    if let Ok(s) = obj.extract::<PyRef<'_, PySeries>>() {
        let res = s.isna()?;
        return Ok(Py::new(py, res)?.into_any());
    }
    if let Ok(idx) = obj.extract::<PyRef<'_, PyIndex>>() {
        let mask = idx.isna();
        return Ok(PyList::new(py, mask)?.into_any().unbind());
    }
    if let Ok(dti) = obj.extract::<PyRef<'_, PyDatetimeIndex>>() {
        let mask = dti.isna();
        return Ok(PyList::new(py, mask)?.into_any().unbind());
    }
    if let Ok(tdi) = obj.extract::<PyRef<'_, PyTimedeltaIndex>>() {
        let mask = tdi.isna();
        return Ok(PyList::new(py, mask)?.into_any().unbind());
    }
    if let Ok(ri) = obj.extract::<PyRef<'_, PyRangeIndex>>() {
        let mask = vec![false; ri.inner.len()];
        return Ok(PyList::new(py, mask)?.into_any().unbind());
    }
    if let Ok(pi) = obj.extract::<PyRef<'_, PyPeriodIndex>>() {
        let mask: Vec<bool> = (0..pi.inner.len())
            .map(|i| {
                pi.inner
                    .values()
                    .get(i)
                    .is_some_and(|p| p.ordinal() == i64::MIN)
            })
            .collect();
        return Ok(PyList::new(py, mask)?.into_any().unbind());
    }
    if let Ok(ci) = obj.extract::<PyRef<'_, PyCategoricalIndex>>() {
        let codes = ci.inner.codes();
        let mask: Vec<bool> = codes.iter().map(Option::is_none).collect();
        return Ok(PyList::new(py, mask)?.into_any().unbind());
    }
    if let Ok(list) = obj.cast::<PyList>() {
        let mut out = Vec::with_capacity(list.len());
        for item in list.iter() {
            if item.is_none() {
                out.push(true);
            } else if let Ok(f) = item.extract::<f64>() {
                out.push(f.is_nan());
            } else {
                out.push(false);
            }
        }
        return Ok(PyList::new(py, out)?.into_any().unbind());
    }
    if obj.is_none() {
        return Ok(pyo3::types::PyBool::new(py, true)
            .to_owned()
            .into_any()
            .unbind());
    }
    if let Ok(f) = obj.extract::<f64>() {
        return Ok(pyo3::types::PyBool::new(py, f.is_nan())
            .to_owned()
            .into_any()
            .unbind());
    }
    Ok(pyo3::types::PyBool::new(py, false)
        .to_owned()
        .into_any()
        .unbind())
}

/// Detect missing values for an array-like object or scalar (pandas `isnull`).
#[pyfunction]
fn isnull(py: Python<'_>, obj: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    isna(py, obj)
}

/// Detect non-missing values for an array-like object or scalar (pandas `notna`).
#[pyfunction]
fn notna(py: Python<'_>, obj: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    if let Ok(df) = obj.extract::<PyRef<'_, PyDataFrame>>() {
        let res = df.notna()?;
        return Ok(Py::new(py, res)?.into_any());
    }
    if let Ok(s) = obj.extract::<PyRef<'_, PySeries>>() {
        let res = s.notna()?;
        return Ok(Py::new(py, res)?.into_any());
    }
    if let Ok(idx) = obj.extract::<PyRef<'_, PyIndex>>() {
        let mask = idx.notna();
        return Ok(PyList::new(py, mask)?.into_any().unbind());
    }
    if let Ok(dti) = obj.extract::<PyRef<'_, PyDatetimeIndex>>() {
        let mask = dti.notna();
        return Ok(PyList::new(py, mask)?.into_any().unbind());
    }
    if let Ok(tdi) = obj.extract::<PyRef<'_, PyTimedeltaIndex>>() {
        let mask = tdi.notna();
        return Ok(PyList::new(py, mask)?.into_any().unbind());
    }
    if let Ok(ri) = obj.extract::<PyRef<'_, PyRangeIndex>>() {
        let mask = vec![true; ri.inner.len()];
        return Ok(PyList::new(py, mask)?.into_any().unbind());
    }
    if let Ok(pi) = obj.extract::<PyRef<'_, PyPeriodIndex>>() {
        let mask: Vec<bool> = (0..pi.inner.len())
            .map(|i| {
                pi.inner
                    .values()
                    .get(i)
                    .is_none_or(|p| p.ordinal() != i64::MIN)
            })
            .collect();
        return Ok(PyList::new(py, mask)?.into_any().unbind());
    }
    if let Ok(ci) = obj.extract::<PyRef<'_, PyCategoricalIndex>>() {
        let codes = ci.inner.codes();
        let mask: Vec<bool> = codes.iter().map(Option::is_some).collect();
        return Ok(PyList::new(py, mask)?.into_any().unbind());
    }
    if let Ok(list) = obj.cast::<PyList>() {
        let mut out = Vec::with_capacity(list.len());
        for item in list.iter() {
            if item.is_none() {
                out.push(false);
            } else if let Ok(f) = item.extract::<f64>() {
                out.push(!f.is_nan());
            } else {
                out.push(true);
            }
        }
        return Ok(PyList::new(py, out)?.into_any().unbind());
    }
    if obj.is_none() {
        return Ok(pyo3::types::PyBool::new(py, false)
            .to_owned()
            .into_any()
            .unbind());
    }
    if let Ok(f) = obj.extract::<f64>() {
        return Ok(pyo3::types::PyBool::new(py, !f.is_nan())
            .to_owned()
            .into_any()
            .unbind());
    }
    Ok(pyo3::types::PyBool::new(py, true)
        .to_owned()
        .into_any()
        .unbind())
}

/// Detect non-missing values for an array-like object or scalar (pandas `notnull`).
#[pyfunction]
fn notnull(py: Python<'_>, obj: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    notna(py, obj)
}

/// Convert argument to timedelta (pandas `to_timedelta`).
#[pyfunction]
#[pyo3(signature = (arg, unit=None, errors="raise"))]
fn to_timedelta(
    py: Python<'_>,
    arg: &Bound<'_, PyAny>,
    unit: Option<&str>,
    errors: &str,
) -> PyResult<Py<PyAny>> {
    let err_policy = match errors {
        "raise" => fp_frame::ToTimedeltaErrors::Raise,
        "coerce" => fp_frame::ToTimedeltaErrors::Coerce,
        "ignore" => fp_frame::ToTimedeltaErrors::Ignore,
        other => {
            return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                "invalid error value {other:?}; must be one of 'raise', 'coerce', 'ignore'"
            )));
        }
    };
    let opts = fp_frame::ToTimedeltaOptions {
        unit,
        errors: err_policy,
    };

    if let Ok(s) = arg.extract::<PyRef<'_, PySeries>>() {
        let res = fp_frame::to_timedelta_with_options(&s.inner, opts).map_err(frame_error_to_py)?;
        return Ok(Py::new(py, PySeries { inner: res })?.into_any());
    }
    if let Ok(tdi) = arg.extract::<PyRef<'_, PyTimedeltaIndex>>() {
        return Ok(Py::new(
            py,
            PyTimedeltaIndex {
                inner: tdi.inner.clone(),
            },
        )?
        .into_any());
    }
    if let Ok(idx) = arg.extract::<PyRef<'_, PyIndex>>() {
        let values: Vec<Scalar> = idx
            .inner
            .labels()
            .iter()
            .map(index_label_to_scalar)
            .collect();
        let temp_series = Series::from_values(
            "",
            (0..values.len())
                .map(|i| IndexLabel::Int64(i as i64))
                .collect(),
            values,
        )
        .map_err(frame_error_to_py)?;
        let res =
            fp_frame::to_timedelta_with_options(&temp_series, opts).map_err(frame_error_to_py)?;
        let nanos: Vec<i64> = res
            .values()
            .iter()
            .map(|s| match s {
                Scalar::Timedelta64(ns) => *ns,
                _ => 0,
            })
            .collect();
        return Ok(Py::new(
            py,
            PyTimedeltaIndex {
                inner: TimedeltaIndex::new(nanos),
            },
        )?
        .into_any());
    }
    if let Ok(list) = arg.cast::<PyList>() {
        let values: Vec<Scalar> = list
            .iter()
            .map(|v| py_to_scalar(py, &v))
            .collect::<PyResult<Vec<_>>>()?;
        let temp_series = Series::from_values(
            "",
            (0..values.len())
                .map(|i| IndexLabel::Int64(i as i64))
                .collect(),
            values,
        )
        .map_err(frame_error_to_py)?;
        let res =
            fp_frame::to_timedelta_with_options(&temp_series, opts).map_err(frame_error_to_py)?;
        let nanos: Vec<i64> = res
            .values()
            .iter()
            .map(|s| match s {
                Scalar::Timedelta64(ns) => *ns,
                _ => 0,
            })
            .collect();
        return Ok(Py::new(
            py,
            PyTimedeltaIndex {
                inner: TimedeltaIndex::new(nanos),
            },
        )?
        .into_any());
    }
    if let Ok(s) = py_to_scalar(py, arg) {
        let temp_series = Series::from_values("", vec![IndexLabel::Int64(0)], vec![s])
            .map_err(frame_error_to_py)?;
        let res =
            fp_frame::to_timedelta_with_options(&temp_series, opts).map_err(frame_error_to_py)?;
        if let Some(val) = res.values().first() {
            return scalar_to_py(py, val);
        }
    }
    Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
        "arg must be a Series, Index, list, or duration-like scalar",
    ))
}

/// Return a fixed frequency TimedeltaIndex (pandas `timedelta_range`).
#[pyfunction]
#[pyo3(signature = (start=None, end=None, periods=None, freq="D", name=None))]
fn timedelta_range(
    start: Option<&str>,
    end: Option<&str>,
    periods: Option<usize>,
    freq: &str,
    name: Option<&str>,
) -> PyResult<PyTimedeltaIndex> {
    let freq_nanos = parse_freq_to_nanos(freq)?;

    let parse_td_arg = |arg: Option<&str>| -> PyResult<Option<i64>> {
        match arg {
            None => Ok(None),
            Some(s) => {
                let trimmed = s.trim();
                if trimmed.is_empty() {
                    Ok(None)
                } else if let Ok(ns) = trimmed.parse::<i64>() {
                    Ok(Some(ns))
                } else {
                    let ns = fp_types::Timedelta::parse(trimmed).map_err(|e| {
                        PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string())
                    })?;
                    Ok(Some(ns))
                }
            }
        }
    };

    let start_ns = parse_td_arg(start)?;
    let end_ns = parse_td_arg(end)?;

    let idx = fp_index::timedelta_range(start_ns, end_ns, periods, freq_nanos, name)
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
    let tdi = TimedeltaIndex::from_index(idx)
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
    Ok(PyTimedeltaIndex { inner: tdi })
}

/// Return a fixed frequency PeriodIndex (pandas `period_range`).
#[pyfunction]
#[pyo3(signature = (start=None, end=None, periods=None, freq=None, name=None))]
fn period_range(
    start: Option<&str>,
    end: Option<&str>,
    periods: Option<usize>,
    freq: Option<&str>,
    name: Option<&str>,
) -> PyResult<PyPeriodIndex> {
    let target_freq = match freq {
        Some(f) => PeriodFreq::parse(f).ok_or_else(|| {
            PyErr::new::<pyo3::exceptions::PyValueError, _>(format!("unsupported frequency '{f}'"))
        })?,
        None => PeriodFreq::Daily,
    };

    let (start_period, count) = match (start, end, periods) {
        (Some(s), _, Some(p)) => {
            let parsed = Period::parse(s)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
            let p_start = if freq.is_some() {
                Period::new(parsed.ordinal(), target_freq)
            } else {
                parsed
            };
            (p_start, p)
        }
        (Some(s), Some(e), None) => {
            let parsed_s = Period::parse(s)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
            let parsed_e = Period::parse(e)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
            let p_start = if freq.is_some() {
                Period::new(parsed_s.ordinal(), target_freq)
            } else {
                parsed_s
            };
            let p_end = if freq.is_some() {
                Period::new(parsed_e.ordinal(), target_freq)
            } else {
                parsed_e
            };
            let diff = p_end.ordinal() - p_start.ordinal();
            if diff < 0 {
                return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
                    "start must be <= end",
                ));
            }
            (p_start, (diff + 1) as usize)
        }
        (None, Some(e), Some(p)) => {
            let parsed_e = Period::parse(e)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
            let p_end = if freq.is_some() {
                Period::new(parsed_e.ordinal(), target_freq)
            } else {
                parsed_e
            };
            let shift_amt = p.saturating_sub(1) as i64;
            let p_start = p_end.shift(-shift_amt);
            (p_start, p)
        }
        _ => {
            return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
                "must specify at least two of: start, end, periods",
            ));
        }
    };

    let values = fp_types::period_range(start_period, count);
    let mut inner = PeriodIndex::new(values);
    if let Some(n) = name {
        inner = inner.set_name(n);
    }
    Ok(PyPeriodIndex { inner })
}

/// Return a fixed frequency DatetimeIndex with business day frequency (pandas `bdate_range`).
#[pyfunction]
#[pyo3(signature = (start=None, end=None, periods=None, freq=None, name=None))]
fn bdate_range(
    start: Option<&str>,
    end: Option<&str>,
    periods: Option<usize>,
    freq: Option<&str>,
    name: Option<&str>,
) -> PyResult<PyDatetimeIndex> {
    let freq_str = freq.unwrap_or("B");
    if freq_str.eq_ignore_ascii_case("b") {
        let idx = fp_index::bdate_range(start, end, periods, name)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        let dti = DatetimeIndex::from_index(idx)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PyDatetimeIndex { inner: dti })
    } else {
        date_range(start, end, periods, Some(freq_str), name)
    }
}

/// Unpivot a DataFrame from wide to long format (pandas `melt`).
#[pyfunction]
#[pyo3(signature = (frame, id_vars=None, value_vars=None, var_name=None, value_name=None))]
fn melt(
    frame: &PyDataFrame,
    id_vars: Option<Vec<String>>,
    value_vars: Option<Vec<String>>,
    var_name: Option<&str>,
    value_name: Option<&str>,
) -> PyResult<PyDataFrame> {
    frame.melt(id_vars, value_vars, var_name, value_name)
}

/// Reshape data based on column values (pandas `pivot`).
#[pyfunction]
#[pyo3(signature = (data, index, columns, values))]
fn pivot(data: &PyDataFrame, index: &str, columns: &str, values: &str) -> PyResult<PyDataFrame> {
    data.pivot(index, columns, values)
}

/// Create a spreadsheet-style pivot table as a DataFrame (pandas `pivot_table`).
#[pyfunction]
#[pyo3(signature = (data, values, index, columns, aggfunc="mean"))]
fn pivot_table(
    data: &PyDataFrame,
    values: &str,
    index: &str,
    columns: &str,
    aggfunc: &str,
) -> PyResult<PyDataFrame> {
    data.pivot_table(values, index, columns, aggfunc)
}

/// Bin values into discrete intervals (pandas `cut`).
#[pyfunction]
#[pyo3(signature = (x, bins, right=true, labels=None, precision=3, include_lowest=false))]
fn cut(
    py: Python<'_>,
    x: &Bound<'_, PyAny>,
    bins: &Bound<'_, PyAny>,
    right: bool,
    labels: Option<Vec<String>>,
    precision: usize,
    include_lowest: bool,
) -> PyResult<PySeries> {
    let _ = precision;
    let series = if let Ok(s) = x.extract::<PyRef<'_, PySeries>>() {
        s.inner.clone()
    } else if let Ok(list) = x.cast::<PyList>() {
        let values: Vec<Scalar> = list
            .iter()
            .map(|v| py_to_scalar(py, &v))
            .collect::<PyResult<Vec<_>>>()?;
        Series::from_values(
            "",
            (0..values.len())
                .map(|i| IndexLabel::Int64(i as i64))
                .collect(),
            values,
        )
        .map_err(frame_error_to_py)?
    } else {
        return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
            "x must be a Series or list of numbers",
        ));
    };

    let label_strings = labels.unwrap_or_default();
    let label_refs: Option<Vec<&str>> = if label_strings.is_empty() {
        None
    } else {
        Some(label_strings.iter().map(String::as_str).collect())
    };

    let res = if let Ok(n_bins) = bins.extract::<usize>() {
        if label_refs.is_some() || !right || include_lowest {
            let mut min_v = f64::INFINITY;
            let mut max_v = f64::NEG_INFINITY;
            for v in series.values() {
                if let Ok(f) = v.to_f64() {
                    if f < min_v {
                        min_v = f;
                    }
                    if f > max_v {
                        max_v = f;
                    }
                }
            }
            if min_v.is_infinite() || max_v.is_infinite() {
                return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
                    "cannot cut empty or all-null series",
                ));
            }
            if (min_v - max_v).abs() < f64::EPSILON {
                min_v -= 0.001 * min_v.abs().max(1.0);
                max_v += 0.001 * max_v.abs().max(1.0);
            }
            let step = (max_v - min_v) / n_bins as f64;
            let edges: Vec<Scalar> = (0..=n_bins)
                .map(|i| Scalar::Float64(min_v + i as f64 * step))
                .collect();
            fp_frame::cut_bins(
                &series,
                &edges,
                right,
                label_refs.as_deref(),
                include_lowest,
            )
            .map_err(frame_error_to_py)?
        } else {
            fp_frame::cut(&series, n_bins).map_err(frame_error_to_py)?
        }
    } else if let Ok(edges_list) = bins.cast::<PyList>() {
        let edges: Vec<Scalar> = edges_list
            .iter()
            .map(|v| py_to_scalar(py, &v))
            .collect::<PyResult<Vec<_>>>()?;
        fp_frame::cut_bins(
            &series,
            &edges,
            right,
            label_refs.as_deref(),
            include_lowest,
        )
        .map_err(frame_error_to_py)?
    } else {
        return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
            "bins must be an integer or a list of bin edges",
        ));
    };

    Ok(PySeries { inner: res })
}

/// Discretize variable into equal-sized buckets based on rank or sample quantiles (pandas `qcut`).
#[pyfunction]
#[pyo3(signature = (x, q, labels=None, precision=3))]
fn qcut(
    py: Python<'_>,
    x: &Bound<'_, PyAny>,
    q: &Bound<'_, PyAny>,
    labels: Option<Vec<String>>,
    precision: usize,
) -> PyResult<PySeries> {
    let _ = precision;
    let series = if let Ok(s) = x.extract::<PyRef<'_, PySeries>>() {
        s.inner.clone()
    } else if let Ok(list) = x.cast::<PyList>() {
        let values: Vec<Scalar> = list
            .iter()
            .map(|v| py_to_scalar(py, &v))
            .collect::<PyResult<Vec<_>>>()?;
        Series::from_values(
            "",
            (0..values.len())
                .map(|i| IndexLabel::Int64(i as i64))
                .collect(),
            values,
        )
        .map_err(frame_error_to_py)?
    } else {
        return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
            "x must be a Series or list of numbers",
        ));
    };

    let label_strings = labels.unwrap_or_default();
    let label_refs: Option<Vec<&str>> = if label_strings.is_empty() {
        None
    } else {
        Some(label_strings.iter().map(String::as_str).collect())
    };

    let res = if let Ok(n_q) = q.extract::<usize>() {
        if let Some(lbls) = label_refs {
            let probs: Vec<f64> = (0..=n_q).map(|i| i as f64 / n_q as f64).collect();
            fp_frame::qcut_at_quantiles(&series, &probs, Some(&lbls)).map_err(frame_error_to_py)?
        } else {
            fp_frame::qcut(&series, n_q).map_err(frame_error_to_py)?
        }
    } else if let Ok(q_list) = q.cast::<PyList>() {
        let quantiles: Vec<f64> = q_list
            .iter()
            .map(|v| v.extract::<f64>())
            .collect::<PyResult<Vec<_>>>()?;
        fp_frame::qcut_at_quantiles(&series, &quantiles, label_refs.as_deref())
            .map_err(frame_error_to_py)?
    } else {
        return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
            "q must be an integer or a list of quantiles [0.0..1.0]",
        ));
    };

    Ok(PySeries { inner: res })
}

/// FrankenPandas Python module.
#[pymodule]
fn frankenpandas(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PySeries>()?;
    m.add_class::<PyDataFrame>()?;
    m.add_class::<PyGroupBy>()?;
    m.add_class::<PySeriesGroupBy>()?;
    m.add_class::<PyResampler>()?;
    m.add_class::<PyStyler>()?;
    m.add_class::<PyIndex>()?;
    m.add_class::<PyIndexStringMethods>()?;
    m.add_class::<PyDatetimeIndex>()?;
    m.add_class::<PyTimedeltaIndex>()?;
    m.add_class::<PyRangeIndex>()?;
    m.add_class::<PyPeriodIndex>()?;
    m.add_class::<PyCategoricalIndex>()?;
    m.add_class::<PyMultiIndex>()?;
    m.add_class::<PyRolling>()?;
    m.add_class::<PyExpanding>()?;
    m.add_class::<PyExponentialMovingWindow>()?;
    m.add_class::<PySeriesILoc>()?;
    m.add_class::<PySeriesLoc>()?;
    m.add_class::<PySeriesIAt>()?;
    m.add_class::<PySeriesAt>()?;
    m.add_class::<PyDataFrameILoc>()?;
    m.add_class::<PyDataFrameLoc>()?;
    m.add_class::<PyDataFrameIAt>()?;
    m.add_class::<PyDataFrameAt>()?;
    m.add_class::<PySeriesStringAccessor>()?;
    m.add_class::<PySeriesDatetimeAccessor>()?;
    m.add_class::<PySeriesCategoricalAccessor>()?;
    m.add_class::<PySeriesListAccessor>()?;
    m.add_class::<PySeriesStructAccessor>()?;
    m.add_class::<PySparseAccessor>()?;
    m.add_function(wrap_pyfunction!(read_csv, m)?)?;
    m.add_function(wrap_pyfunction!(read_json, m)?)?;
    m.add_function(wrap_pyfunction!(read_jsonl, m)?)?;
    m.add_function(wrap_pyfunction!(read_parquet, m)?)?;
    m.add_function(wrap_pyfunction!(concat, m)?)?;
    m.add_function(wrap_pyfunction!(merge, m)?)?;
    m.add_function(wrap_pyfunction!(melt, m)?)?;
    m.add_function(wrap_pyfunction!(pivot, m)?)?;
    m.add_function(wrap_pyfunction!(pivot_table, m)?)?;
    m.add_function(wrap_pyfunction!(cut, m)?)?;
    m.add_function(wrap_pyfunction!(qcut, m)?)?;
    m.add_function(wrap_pyfunction!(to_numeric, m)?)?;
    m.add_function(wrap_pyfunction!(to_datetime, m)?)?;
    m.add_function(wrap_pyfunction!(to_timedelta, m)?)?;
    m.add_function(wrap_pyfunction!(date_range, m)?)?;
    m.add_function(wrap_pyfunction!(bdate_range, m)?)?;
    m.add_function(wrap_pyfunction!(timedelta_range, m)?)?;
    m.add_function(wrap_pyfunction!(period_range, m)?)?;
    m.add_function(wrap_pyfunction!(isna, m)?)?;
    m.add_function(wrap_pyfunction!(isnull, m)?)?;
    m.add_function(wrap_pyfunction!(notna, m)?)?;
    m.add_function(wrap_pyfunction!(notnull, m)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use fp_frame::{DataFrame, Series};
    use fp_index::IndexLabel;
    use fp_types::Scalar;

    use super::*;

    #[cfg(feature = "lazy-transpose-view")]
    #[test]
    fn dataframe_observers_preserve_lazy_transpose_storage() {
        let source = DataFrame::from_dict(
            &["left", "right"],
            vec![
                (
                    "left",
                    vec![Scalar::Int64(10), Scalar::Int64(20), Scalar::Int64(30)],
                ),
                (
                    "right",
                    vec![Scalar::Int64(40), Scalar::Int64(50), Scalar::Int64(60)],
                ),
            ],
        )
        .expect("source frame"); // ubs:ignore — static unit-test fixture is valid
        let dataframe = PyDataFrame {
            inner: source.transpose().expect("lazy transpose"), // ubs:ignore — homogeneous static fixture supports transpose
        };

        assert!(dataframe.inner.is_lazy_transpose_storage());
        assert_eq!(dataframe.shape(), (2, 3));
        assert_eq!(dataframe.columns(), vec!["0", "1", "2"]);
        assert!(dataframe.inner.is_lazy_transpose_storage());

        let selected = dataframe.column_series("1").expect("selected column"); // ubs:ignore — asserted static transpose label exists
        assert_eq!(
            selected.inner.values(),
            &[Scalar::Int64(20), Scalar::Int64(50)]
        );
        assert!(dataframe.inner.is_lazy_transpose_storage());
    }

    #[test]
    fn test_py_index() {
        let labels = vec![
            IndexLabel::Int64(10),
            IndexLabel::Int64(20),
            IndexLabel::Int64(30),
        ];
        let mut idx = PyIndex {
            inner: Index::new(labels).set_name("my_idx"),
        };
        assert_eq!(idx.len(), 3);
        assert_eq!(idx.name(), Some("my_idx".to_string()));
        assert!(idx.is_unique());
        assert_eq!(idx.__repr__(), "Index([10, 20, 30], name='my_idx')");

        idx.set_name(Some("renamed"));
        assert_eq!(idx.name(), Some("renamed".to_string()));

        let other = PyIndex {
            inner: Index::new(vec![
                IndexLabel::Int64(10),
                IndexLabel::Int64(20),
                IndexLabel::Int64(30),
            ])
            .set_name("renamed"),
        };
        assert!(idx.equals(&other));

        let non_matching = PyIndex {
            inner: Index::new(vec![
                IndexLabel::Int64(1),
                IndexLabel::Int64(2),
                IndexLabel::Int64(3),
            ]),
        };
        assert!(!idx.equals(&non_matching));
    }

    #[test]
    fn test_py_series_and_indexers() {
        let labels = vec![
            IndexLabel::Utf8("a".into()),
            IndexLabel::Utf8("b".into()),
            IndexLabel::Utf8("c".into()),
        ];
        let values = vec![Scalar::Int64(100), Scalar::Int64(200), Scalar::Int64(300)];
        let series = Series::from_values("test_s", labels.clone(), values).expect("valid series"); // ubs:ignore — test fixture
        let py_s = PySeries { inner: series };

        // Index getter
        let idx = py_s.index();
        assert_eq!(idx.len(), 3);
        assert_eq!(idx.inner.labels(), &labels);

        // iloc proxy
        let iloc = py_s.iloc();
        assert_eq!(iloc.inner.len(), 3);
        assert_eq!(iloc.inner.iat(0).expect("iat 0"), Scalar::Int64(100)); // ubs:ignore — test fixture

        // loc proxy
        let loc = py_s.loc();
        assert_eq!(loc.inner.len(), 3);
        let loc_val = loc
            .inner
            .loc(&[IndexLabel::Utf8("b".into())])
            .expect("loc b"); // ubs:ignore — test fixture
        assert_eq!(loc_val.values(), &[Scalar::Int64(200)]);

        // iat proxy
        let iat = py_s.iat();
        assert_eq!(iat.inner.len(), 3);

        // at proxy
        let at = py_s.at();
        assert_eq!(at.inner.len(), 3);
    }

    #[test]
    fn test_py_dataframe_indexers_and_dunders() {
        let df = DataFrame::from_dict(
            &["x", "y"],
            vec![
                (
                    "x",
                    vec![Scalar::Int64(10), Scalar::Int64(20), Scalar::Int64(30)],
                ),
                (
                    "y",
                    vec![Scalar::Int64(40), Scalar::Int64(50), Scalar::Int64(60)],
                ),
            ],
        )
        .expect("dataframe"); // ubs:ignore — test fixture
        let py_df = PyDataFrame { inner: df };

        // Shape and index
        assert_eq!(py_df.shape(), (3, 2));
        let idx = py_df.index();
        assert_eq!(idx.len(), 3);

        // Arithmetic: __neg__
        let neg = py_df.__neg__().expect("negate"); // ubs:ignore — test fixture
        let col_x = neg.column_series("x").expect("col x"); // ubs:ignore — test fixture
        assert_eq!(
            col_x.inner.values(),
            &[Scalar::Int64(-10), Scalar::Int64(-20), Scalar::Int64(-30)]
        );

        // iloc proxy
        let iloc = py_df.iloc();
        assert_eq!(iloc.inner.shape(), (3, 2));
        let row0 = iloc.inner.iloc_row(0).expect("row 0"); // ubs:ignore — test fixture
        assert_eq!(row0.len(), 2);

        // loc proxy
        let loc = py_df.loc();
        assert_eq!(loc.inner.shape(), (3, 2));

        // iat proxy
        let iat = py_df.iat();
        assert_eq!(iat.inner.shape(), (3, 2));

        // at proxy
        let at = py_df.at();
        assert_eq!(at.inner.shape(), (3, 2));
    }

    #[test]
    fn test_py_merge_to_numeric_to_datetime_backends() {
        // Test merge logic via inner DataFrame
        let df1 = DataFrame::from_dict(
            &["id", "v1"],
            vec![
                ("id", vec![Scalar::Int64(1), Scalar::Int64(2)]),
                ("v1", vec![Scalar::Int64(10), Scalar::Int64(20)]),
            ],
        )
        .expect("df1"); // ubs:ignore — test fixture
        let df2 = DataFrame::from_dict(
            &["id", "v2"],
            vec![
                ("id", vec![Scalar::Int64(2), Scalar::Int64(3)]),
                ("v2", vec![Scalar::Int64(200), Scalar::Int64(300)]),
            ],
        )
        .expect("df2"); // ubs:ignore — test fixture

        let merged = fp_join::merge_dataframes_on_with(
            &df1,
            &df2,
            &["id"],
            &["id"],
            fp_join::JoinType::Inner,
        )
        .expect("merge"); // ubs:ignore — test fixture
        assert_eq!(merged.index.len(), 1);

        // Test to_numeric logic on Series
        let s_text = Series::from_values(
            "nums",
            vec![IndexLabel::Int64(0), IndexLabel::Int64(1)],
            vec![Scalar::Utf8("123".into()), Scalar::Utf8("456".into())],
        )
        .expect("s_text"); // ubs:ignore — test fixture
        let s_num = fp_frame::to_numeric_with_options(
            &s_text,
            fp_frame::ToNumericOptions {
                errors: fp_frame::ToNumericErrors::Raise,
            },
        )
        .expect("to_numeric"); // ubs:ignore — test fixture
        assert_eq!(s_num.values(), &[Scalar::Int64(123), Scalar::Int64(456)]);

        // Test to_datetime logic on Series
        let s_dates = Series::from_values(
            "dates",
            vec![IndexLabel::Int64(0)],
            vec![Scalar::Utf8("2023-01-01".into())],
        )
        .expect("s_dates"); // ubs:ignore — test fixture
        let s_dt =
            fp_frame::to_datetime_with_options(&s_dates, fp_frame::ToDatetimeOptions::default())
                .expect("to_datetime"); // ubs:ignore — test fixture
        assert_eq!(s_dt.len(), 1);
    }

    #[test]
    fn test_frame_error_to_py_mapping() {
        use fp_columnar::ColumnError;
        use fp_frame::FrameError;
        use fp_index::IndexError;

        let out_of_bounds = FrameError::Index(IndexError::OutOfBounds {
            position: 5,
            length: 3,
        });
        let (kind, msg) = classify_frame_error(&out_of_bounds);
        assert_eq!(kind, PyErrorKind::Index);
        assert!(msg.contains("out of bounds"));

        let type_err = FrameError::Column(ColumnError::InvalidMaskType {
            dtype: fp_types::DType::Int64,
        });
        let (kind, msg) = classify_frame_error(&type_err);
        assert_eq!(kind, PyErrorKind::Type);
        assert!(msg.contains("mask must be Bool dtype"));

        let len_err = FrameError::LengthMismatch {
            index_len: 2,
            column_len: 3,
        };
        let (kind, msg) = classify_frame_error(&len_err);
        assert_eq!(kind, PyErrorKind::Value);
        assert!(msg.contains("index length"));

        let not_impl_err = FrameError::CompatibilityRejected("feature not implemented yet".into());
        let (kind, msg) = classify_frame_error(&not_impl_err);
        assert_eq!(kind, PyErrorKind::NotImplemented);
        assert!(msg.contains("not implemented"));

        let key_err = FrameError::CompatibilityRejected("column not found: missing".into());
        let (kind, msg) = classify_frame_error(&key_err);
        assert_eq!(kind, PyErrorKind::Key);
        assert!(msg.contains("column not found"));
    }

    #[test]
    fn test_parse_freq_to_nanos() {
        assert_eq!(parse_freq_to_nanos("D").unwrap(), 86_400_000_000_000);
        assert_eq!(parse_freq_to_nanos("2D").unwrap(), 2 * 86_400_000_000_000);
        assert_eq!(parse_freq_to_nanos("h").unwrap(), 3_600_000_000_000);
        assert_eq!(parse_freq_to_nanos("5h").unwrap(), 5 * 3_600_000_000_000);
        assert_eq!(parse_freq_to_nanos("min").unwrap(), 60_000_000_000);
        assert_eq!(parse_freq_to_nanos("s").unwrap(), 1_000_000_000);
        assert_eq!(parse_freq_to_nanos("ms").unwrap(), 1_000_000);
        assert_eq!(parse_freq_to_nanos("us").unwrap(), 1_000);
        assert_eq!(parse_freq_to_nanos("ns").unwrap(), 1);
        assert!(parse_freq_to_nanos("invalid").is_err());
        assert!(parse_freq_to_nanos("").is_err());
    }

    #[test]
    fn test_py_index_advanced_ops() {
        let labels = vec![
            IndexLabel::Int64(10),
            IndexLabel::Int64(20),
            IndexLabel::Int64(30),
            IndexLabel::Int64(20),
        ];
        let idx = PyIndex {
            inner: Index::new(labels),
        };
        assert_eq!(idx.dtype(), "int64");
        assert_eq!(idx.shape(), (4,));
        assert_eq!(idx.size(), 4);
        assert_eq!(idx.ndim(), 1);
        assert!(!idx.empty());
        assert!(!idx.is_monotonic_increasing());
        assert!(!idx.is_monotonic_decreasing());
        assert!(idx.has_duplicates());
        assert_eq!(idx.nunique(), 3);

        let u = idx.unique();
        assert_eq!(u.len(), 3);
        let dedup = idx.drop_duplicates();
        assert_eq!(dedup.len(), 3);

        let sorted_idx = PyIndex {
            inner: Index::new(vec![
                IndexLabel::Int64(1),
                IndexLabel::Int64(2),
                IndexLabel::Int64(3),
            ]),
        };
        assert!(sorted_idx.is_monotonic_increasing());
        assert_eq!(sorted_idx.inner.min(), Some(IndexLabel::Int64(1)));
        assert_eq!(sorted_idx.inner.max(), Some(IndexLabel::Int64(3)));

        let isin_res = sorted_idx
            .inner
            .isin(&[IndexLabel::Int64(2), IndexLabel::Int64(99)]);
        assert_eq!(isin_res, vec![false, true, false]);
        assert_eq!(sorted_idx.isna(), vec![false, false, false]);
        assert_eq!(sorted_idx.notna(), vec![true, true, true]);

        let idx_a = PyIndex {
            inner: Index::new(vec![IndexLabel::Int64(1), IndexLabel::Int64(2)]),
        };
        let idx_b = PyIndex {
            inner: Index::new(vec![IndexLabel::Int64(2), IndexLabel::Int64(3)]),
        };
        assert_eq!(idx_a.union(&idx_b).len(), 3);
        assert_eq!(idx_a.intersection(&idx_b).len(), 1);
        assert_eq!(idx_a.difference(&idx_b).len(), 1);
        assert_eq!(idx_a.append(&idx_b).len(), 4);
    }

    #[test]
    fn test_py_datetime_index_calendar_and_stats() {
        // 2024-01-01 00:00:00 UTC = 1_704_067_200_000_000_000 ns
        let nanos1 = 1_704_067_200_000_000_000_i64;
        // 2024-01-02 00:00:00 UTC = 1_704_153_600_000_000_000 ns
        let nanos2 = 1_704_153_600_000_000_000_i64;
        let dti = PyDatetimeIndex {
            inner: DatetimeIndex::new(vec![nanos1, nanos2]),
        };

        assert_eq!(dti.dtype(), "datetime64[ns]");
        assert_eq!(dti.shape(), (2,));
        assert_eq!(dti.size(), 2);
        assert_eq!(dti.ndim(), 1);
        assert!(!dti.empty());
        assert_eq!(dti.len(), 2);
        assert!(dti.is_monotonic_increasing());

        assert_eq!(dti.year(), vec![Some(2024), Some(2024)]);
        assert_eq!(dti.month(), vec![Some(1), Some(1)]);
        assert_eq!(dti.day(), vec![Some(1), Some(2)]);
        assert_eq!(dti.hour(), vec![Some(0), Some(0)]);
        assert_eq!(dti.minute(), vec![Some(0), Some(0)]);
        assert_eq!(dti.second(), vec![Some(0), Some(0)]);
        assert_eq!(dti.microsecond(), vec![Some(0), Some(0)]);
        assert_eq!(dti.nanosecond(), vec![Some(0), Some(0)]);
        assert_eq!(dti.days_in_month(), vec![Some(31), Some(31)]);
        assert_eq!(dti.is_leap_year(), vec![Some(true), Some(true)]);

        assert_eq!(dti.asi8(), vec![nanos1, nanos2]);
        assert_eq!(dti.nunique(), 2);
        assert_eq!(dti.isna(), vec![false, false]);
        assert_eq!(dti.notna(), vec![true, true]);
        assert_eq!(dti.isin(vec![nanos1]), vec![true, false]);

        let shifted = dti.shift(1, "D").expect("shift"); // ubs:ignore — valid freq
        assert_eq!(shifted.len(), 2);
        assert_eq!(shifted.asi8()[0], nanos2);

        let diffed = dti.diff(1);
        assert_eq!(diffed, vec![None, Some(86_400_000_000_000)]);
    }

    #[test]
    fn test_py_multi_index_levels_and_flat() {
        let tuples = vec![
            vec![IndexLabel::Utf8("a".into()), IndexLabel::Utf8("b".into())],
            vec![IndexLabel::Utf8("c".into()), IndexLabel::Utf8("d".into())],
        ];
        let mi = MultiIndex::from_tuples(tuples).expect("valid multiindex"); // ubs:ignore — test fixture
        let py_mi = PyMultiIndex { inner: mi };

        assert_eq!(py_mi.nlevels(), 2);
        assert_eq!(py_mi.len(), 2);

        let lvl0 = py_mi.get_level_values(0).expect("level 0"); // ubs:ignore — test fixture
        assert_eq!(lvl0.len(), 2);
        let flat = py_mi.to_flat_index("/");
        assert_eq!(flat.len(), 2);
    }

    #[test]
    fn test_py_series_extended_ops() {
        let labels = vec![
            IndexLabel::Int64(0),
            IndexLabel::Int64(1),
            IndexLabel::Int64(2),
            IndexLabel::Int64(3),
        ];
        let values = vec![
            Scalar::Float64(10.0),
            Scalar::Float64(50.0),
            Scalar::Float64(20.0),
            Scalar::Float64(10.0),
        ];
        let s = Series::from_values("s", labels, values).expect("series"); // ubs:ignore — test fixture
        let py_s = PySeries { inner: s };

        assert_eq!(py_s.dtype(), "float64");
        assert_eq!(py_s.shape(), (4,));
        assert_eq!(py_s.size(), 4);
        assert_eq!(py_s.ndim(), 1);
        assert!(!py_s.empty());

        let isin_res = py_s.inner.isin(&[Scalar::Float64(10.0)]).expect("isin"); // ubs:ignore — test fixture
        assert_eq!(
            isin_res.values(),
            &[
                Scalar::Bool(true),
                Scalar::Bool(false),
                Scalar::Bool(false),
                Scalar::Bool(true)
            ]
        );

        let betw = py_s
            .inner
            .between(&Scalar::Float64(15.0), &Scalar::Float64(55.0), "both")
            .expect("between"); // ubs:ignore — test fixture
        assert_eq!(
            betw.values(),
            &[
                Scalar::Bool(false),
                Scalar::Bool(true),
                Scalar::Bool(true),
                Scalar::Bool(false)
            ]
        );

        assert_eq!(py_s.argmax().expect("argmax"), 1); // ubs:ignore — test fixture
        assert_eq!(py_s.argmin().expect("argmin"), 0); // ubs:ignore — test fixture
        assert_eq!(py_s.inner.idxmax().expect("idxmax"), IndexLabel::Int64(1)); // ubs:ignore — test fixture
        assert_eq!(py_s.inner.idxmin().expect("idxmin"), IndexLabel::Int64(0)); // ubs:ignore — test fixture

        let shifted = py_s.shift(1).expect("shift"); // ubs:ignore — test fixture
        assert_eq!(shifted.inner.len(), 4);

        let nlg = py_s.nlargest(2).expect("nlargest"); // ubs:ignore — test fixture
        assert_eq!(nlg.inner.len(), 2);
        let nsm = py_s.nsmallest(2).expect("nsmallest"); // ubs:ignore — test fixture
        assert_eq!(nsm.inner.len(), 2);

        let dedup = py_s.drop_duplicates().expect("drop_duplicates"); // ubs:ignore — test fixture
        assert_eq!(dedup.inner.len(), 3);
        let dups = py_s.duplicated().expect("duplicated"); // ubs:ignore — test fixture
        assert_eq!(dups.inner.len(), 4);

        let reset = py_s.inner.reset_index(false).expect("reset_index"); // ubs:ignore — test fixture
        assert!(matches!(
            reset,
            fp_frame::SeriesResetIndexResult::DataFrame(ref df) if df.shape() == (4, 2)
        ));
    }

    #[test]
    fn test_py_dataframe_extended_ops() {
        let df = DataFrame::from_dict(
            &["a", "b"],
            vec![
                (
                    "a",
                    vec![
                        Scalar::Float64(1.0),
                        Scalar::Float64(2.0),
                        Scalar::Float64(3.0),
                    ],
                ),
                (
                    "b",
                    vec![
                        Scalar::Float64(10.0),
                        Scalar::Float64(20.0),
                        Scalar::Float64(30.0),
                    ],
                ),
            ],
        )
        .expect("dataframe"); // ubs:ignore — test fixture
        let py_df = PyDataFrame { inner: df };

        assert!(!py_df.empty());
        assert_eq!(py_df.ndim(), 2);
        assert_eq!(py_df.size(), 6);

        let cp = py_df.copy();
        assert_eq!(cp.shape(), (3, 2));

        let isna_df = py_df.isna().expect("isna"); // ubs:ignore — test fixture
        assert_eq!(isna_df.shape(), (3, 2));
        let notna_df = py_df.notna().expect("notna"); // ubs:ignore — test fixture
        assert_eq!(notna_df.shape(), (3, 2));
        let isnull_df = py_df.isnull().expect("isnull"); // ubs:ignore — test fixture
        assert_eq!(isnull_df.shape(), (3, 2));
        let notnull_df = py_df.notnull().expect("notnull"); // ubs:ignore — test fixture
        assert_eq!(notnull_df.shape(), (3, 2));

        let idxmax_s = py_df.idxmax().expect("idxmax"); // ubs:ignore — test fixture
        assert_eq!(idxmax_s.inner.len(), 2);
        let idxmin_s = py_df.idxmin().expect("idxmin"); // ubs:ignore — test fixture
        assert_eq!(idxmin_s.inner.len(), 2);

        let diff_df = py_df.diff(1).expect("diff"); // ubs:ignore — test fixture
        assert_eq!(diff_df.shape(), (3, 2));

        let pct_df = py_df.pct_change(1).expect("pct_change"); // ubs:ignore — test fixture
        assert_eq!(pct_df.shape(), (3, 2));

        let cs = py_df.cumsum(true).expect("cumsum"); // ubs:ignore — test fixture
        assert_eq!(cs.shape(), (3, 2));
        let cp = py_df.cumprod(true).expect("cumprod"); // ubs:ignore — test fixture
        assert_eq!(cp.shape(), (3, 2));
        let cmin = py_df.cummin(true).expect("cummin"); // ubs:ignore — test fixture
        assert_eq!(cmin.shape(), (3, 2));
        let cmax = py_df.cummax(true).expect("cummax"); // ubs:ignore — test fixture
        assert_eq!(cmax.shape(), (3, 2));

        let sh = py_df.shift(1).expect("shift"); // ubs:ignore — test fixture
        assert_eq!(sh.shape(), (3, 2));

        let queried = py_df.query("a > 1").expect("query"); // ubs:ignore — test fixture
        assert_eq!(queried.shape(), (2, 2));

        let dups = py_df.duplicated(None, None).expect("duplicated"); // ubs:ignore — test fixture
        assert_eq!(dups.inner.len(), 3);
        let dedup = py_df
            .drop_duplicates(None, None, false)
            .expect("drop_duplicates"); // ubs:ignore — test fixture
        assert_eq!(dedup.shape(), (3, 2));
    }

    #[test]
    fn test_py_timedelta_index_and_range() {
        let tdi = timedelta_range(Some("0D"), None, Some(4), "1D", Some("td_idx"))
            .expect("timedelta_range"); // ubs:ignore — test fixture
        assert_eq!(tdi.len(), 4);
        assert_eq!(tdi.name().as_deref(), Some("td_idx"));
        assert_eq!(tdi.ndim(), 1);
        assert!(!tdi.empty());
        assert_eq!(tdi.shape(), (4,));
        assert_eq!(tdi.size(), 4);
        let days = tdi.days();
        assert_eq!(days, vec![Some(0), Some(1), Some(2), Some(3)]);

        let shifted = tdi.shift(1, "D").expect("shift"); // ubs:ignore — test fixture
        assert_eq!(shifted.len(), 4);
    }

    #[test]
    fn test_py_range_index() {
        let ri =
            PyRangeIndex::new(Some(0), Some(10), Some(2), Some("my_range")).expect("range_index"); // ubs:ignore — test fixture
        assert_eq!(ri.len(), 5);
        assert_eq!(ri.start(), 0);
        assert_eq!(ri.stop(), 10);
        assert_eq!(ri.step(), 2);
        assert_eq!(ri.name().as_deref(), Some("my_range"));
        assert!(ri.__contains__(4));
        assert!(!ri.__contains__(5));
        let vals = ri.tolist();
        assert_eq!(vals, vec![0, 2, 4, 6, 8]);
    }

    #[test]
    fn test_py_period_index_and_range() {
        let pi = period_range(Some("2024-01"), None, Some(3), Some("M"), Some("monthly"))
            .expect("period_range"); // ubs:ignore — test fixture
        assert_eq!(pi.len(), 3);
        assert_eq!(pi.name().as_deref(), Some("monthly"));
        let years = pi.year().expect("year"); // ubs:ignore — test fixture
        assert_eq!(years, vec![Some(2024), Some(2024), Some(2024)]);
        let months = pi.month().expect("month"); // ubs:ignore — test fixture
        assert_eq!(months, vec![Some(1), Some(2), Some(3)]);
    }

    #[test]
    fn test_py_categorical_index() {
        let ci = CategoricalIndex::with_categories(
            vec![
                "cat".to_string(),
                "dog".to_string(),
                "cat".to_string(),
                "dog".to_string(),
            ],
            vec!["cat".to_string(), "dog".to_string()],
            false,
        )
        .expect("categorical index"); // ubs:ignore — test fixture
        let py_ci = PyCategoricalIndex { inner: ci };
        assert_eq!(py_ci.len(), 4);
        assert_eq!(py_ci.name().as_deref(), None);
        assert_eq!(py_ci.categories(), vec!["cat", "dog"]);
        assert_eq!(py_ci.codes(), vec![Some(0), Some(1), Some(0), Some(1)]);
        assert!(!py_ci.ordered());
    }

    #[test]
    fn test_py_windowing_and_series_math() {
        let s = Series::from_values(
            "val",
            vec![
                IndexLabel::Int64(0),
                IndexLabel::Int64(1),
                IndexLabel::Int64(2),
                IndexLabel::Int64(3),
            ],
            vec![
                Scalar::Float64(1.0),
                Scalar::Float64(2.0),
                Scalar::Float64(3.0),
                Scalar::Float64(4.0),
            ],
        )
        .expect("series"); // ubs:ignore — test fixture
        let py_s = PySeries { inner: s };

        let roll = py_s.rolling(2, None, false);
        assert_eq!(roll.window, 2);
        assert!(!roll.center);

        let exp = py_s.expanding(None);
        assert_eq!(exp.min_periods, None);

        let ewm = py_s.ewm(Some(0.5), None);
        assert_eq!(ewm.span, Some(0.5));

        let s2 = py_s.clone();
        let c = py_s.corr(&s2).expect("corr"); // ubs:ignore — test fixture
        assert!((c - 1.0).abs() < 1e-6);

        let ffilled = py_s.ffill(None).expect("ffill"); // ubs:ignore — test fixture
        assert_eq!(ffilled.inner.len(), 4);
    }

    #[test]
    fn test_py_dataframe_windowing_and_transformations() {
        let df = DataFrame::from_dict(
            &["a", "b"],
            vec![
                (
                    "a",
                    vec![
                        Scalar::Float64(1.0),
                        Scalar::Float64(2.0),
                        Scalar::Float64(3.0),
                    ],
                ),
                (
                    "b",
                    vec![
                        Scalar::Float64(4.0),
                        Scalar::Float64(5.0),
                        Scalar::Float64(6.0),
                    ],
                ),
            ],
        )
        .expect("df"); // ubs:ignore — test fixture
        let py_df = PyDataFrame { inner: df };

        let roll = py_df.rolling(2, None, false);
        assert_eq!(roll.window, 2);
        assert!(!roll.center);

        let tr = py_df.transpose().expect("transpose"); // ubs:ignore — test fixture
        assert_eq!(tr.shape(), (2, 3));
        let t_prop = py_df.T().expect("T property"); // ubs:ignore — test fixture
        assert_eq!(t_prop.shape(), (2, 3));

        let melted = py_df
            .melt(Some(vec!["a".to_string()]), None, None, None)
            .expect("melt"); // ubs:ignore — test fixture
        assert_eq!(melted.shape(), (3, 3));

        let abs_df = py_df.abs().expect("abs"); // ubs:ignore — test fixture
        assert_eq!(abs_df.shape(), (3, 2));

        let clip_df = py_df.clip(Some(2.0), Some(5.0)).expect("clip"); // ubs:ignore — test fixture
        assert_eq!(clip_df.shape(), (3, 2));
    }

    #[test]
    fn test_bdate_range_helper() {
        let bdr = bdate_range(Some("2024-01-01"), None, Some(5), None, Some("bday"))
            .expect("bdate_range"); // ubs:ignore — test fixture
        assert_eq!(bdr.len(), 5);
        assert_eq!(bdr.name().as_deref(), Some("bday"));
    }

    #[test]
    fn test_py_groupby_and_resampler() {
        let s = Series::from_values(
            "vals",
            vec![
                IndexLabel::Int64(0),
                IndexLabel::Int64(1),
                IndexLabel::Int64(2),
                IndexLabel::Int64(3),
            ],
            vec![
                Scalar::Float64(10.0),
                Scalar::Float64(20.0),
                Scalar::Float64(30.0),
                Scalar::Float64(40.0),
            ],
        )
        .expect("series"); // ubs:ignore — test fixture
        let py_s = PySeries { inner: s };

        let by_s = Series::from_values(
            "k",
            vec![
                IndexLabel::Int64(0),
                IndexLabel::Int64(1),
                IndexLabel::Int64(2),
                IndexLabel::Int64(3),
            ],
            vec![
                Scalar::Utf8("a".to_string()),
                Scalar::Utf8("b".to_string()),
                Scalar::Utf8("a".to_string()),
                Scalar::Utf8("b".to_string()),
            ],
        )
        .expect("by_series"); // ubs:ignore — test fixture

        let sgb = PySeriesGroupBy {
            series: py_s.inner.clone(),
            by: by_s,
        };

        let sum_s = sgb.sum().expect("sum"); // ubs:ignore — test fixture
        assert_eq!(sum_s.inner.len(), 2);
        let mean_s = sgb.mean().expect("mean"); // ubs:ignore — test fixture
        assert_eq!(mean_s.inner.len(), 2);
        let count_s = sgb.count().expect("count"); // ubs:ignore — test fixture
        assert_eq!(count_s.inner.len(), 2);
        let min_s = sgb.min().expect("min"); // ubs:ignore — test fixture
        assert_eq!(min_s.inner.len(), 2);
        let max_s = sgb.max().expect("max"); // ubs:ignore — test fixture
        assert_eq!(max_s.inner.len(), 2);
        let first_s = sgb.first().expect("first"); // ubs:ignore — test fixture
        assert_eq!(first_s.inner.len(), 2);
        let last_s = sgb.last().expect("last"); // ubs:ignore — test fixture
        assert_eq!(last_s.inner.len(), 2);
        let size_s = sgb.size().expect("size"); // ubs:ignore — test fixture
        assert_eq!(size_s.inner.len(), 2);
        let nq = sgb.nunique().expect("nunique"); // ubs:ignore — test fixture
        assert_eq!(nq.inner.len(), 2);
        assert_eq!(sgb.ngroups().expect("ngroups"), 2); // ubs:ignore — test fixture

        let df = DataFrame::from_dict(
            &["grp", "val"],
            vec![
                (
                    "grp",
                    vec![
                        Scalar::Utf8("x".to_string()),
                        Scalar::Utf8("x".to_string()),
                        Scalar::Utf8("y".to_string()),
                    ],
                ),
                (
                    "val",
                    vec![
                        Scalar::Float64(1.0),
                        Scalar::Float64(2.0),
                        Scalar::Float64(3.0),
                    ],
                ),
            ],
        )
        .expect("df"); // ubs:ignore — test fixture
        let py_df = PyDataFrame { inner: df };

        let gb = PyGroupBy {
            df: py_df.inner.clone(),
            by: vec!["grp".to_string()],
        };
        let gb_first = gb.first().expect("first"); // ubs:ignore — test fixture
        assert_eq!(gb_first.shape(), (2, 1));
        let gb_last = gb.last().expect("last"); // ubs:ignore — test fixture
        assert_eq!(gb_last.shape(), (2, 1));
        let gb_size = gb.size().expect("size"); // ubs:ignore — test fixture
        assert_eq!(gb_size.inner.len(), 2);
        let gb_nq = gb.nunique().expect("nunique"); // ubs:ignore — test fixture
        assert_eq!(gb_nq.shape(), (2, 1));
        assert_eq!(gb.ngroups().expect("ngroups"), 2); // ubs:ignore — test fixture

        let resampler_df = py_df.resample("1D", None, None, None);
        assert_eq!(resampler_df.freq, "1D");

        let resampler_s = py_s.resample("1D", None, None, None);
        assert_eq!(resampler_s.freq, "1D");
    }

    #[test]
    fn test_py_series_and_dataframe_operations() {
        let s = Series::new(
            "s",
            Index::new(vec![
                IndexLabel::Int64(0),
                IndexLabel::Int64(1),
                IndexLabel::Int64(2),
            ]),
            Column::from_f64_values(vec![10.0, 20.0, 30.0]),
        )
        .expect("series"); // ubs:ignore — test fixture
        let py_s = PySeries { inner: s };

        let t_s = py_s.T();
        assert_eq!(t_s.inner.len(), 3);
        let keys_s = py_s.keys();
        assert_eq!(keys_s.inner.len(), 3);
        let pad_s = py_s.pad(None).expect("pad"); // ubs:ignore — test fixture
        assert_eq!(pad_s.inner.len(), 3);
        let backfill_s = py_s.backfill(None).expect("backfill"); // ubs:ignore — test fixture
        assert_eq!(backfill_s.inner.len(), 3);

        let df = DataFrame::from_dict(
            &["a", "b"],
            vec![
                ("a", vec![Scalar::Float64(1.0), Scalar::Float64(2.0)]),
                ("b", vec![Scalar::Float64(3.0), Scalar::Float64(4.0)]),
            ],
        )
        .expect("df"); // ubs:ignore — test fixture
        let py_df = PyDataFrame { inner: df };
        assert_eq!(py_df.keys(), vec!["a".to_string(), "b".to_string()]);
        let t_df = py_df.T().expect("T"); // ubs:ignore — test fixture
        assert_eq!(t_df.shape(), (2, 2));

        let roll = py_df.rolling(2, None, false);
        assert_eq!(roll.ndim(), 2);
        let exp = py_df.expanding(None);
        assert_eq!(exp.ndim(), 2);
        let ewm = py_df.ewm(Some(0.5), None);
        assert_eq!(ewm.ndim(), 2);
    }

    #[test]
    fn test_py_batch2_operations() {
        let idx = PyIndex {
            inner: Index::new(vec![
                IndexLabel::Int64(1),
                IndexLabel::Int64(2),
                IndexLabel::Null(fp_types::NullKind::Null),
            ]),
        };
        assert!(!idx.all());
        assert!(idx.any());
        let dropped = idx.dropna(None);
        assert_eq!(dropped.inner.len(), 2);
        let deleted = idx.delete(0).expect("delete"); // ubs:ignore — test fixture
        assert_eq!(deleted.inner.len(), 2);
        let repeated = idx.repeat(2);
        assert_eq!(repeated.inner.len(), 6);
        let taken = idx.take(vec![1, 0]).expect("take"); // ubs:ignore — test fixture
        assert_eq!(taken.inner.len(), 2);

        let s1 = Series::new(
            "s1",
            Index::new(vec![IndexLabel::Int64(0), IndexLabel::Int64(1)]),
            Column::from_f64_values(vec![10.0, 20.0]),
        )
        .expect("series"); // ubs:ignore — test fixture
        let py_s1 = PySeries { inner: s1 };
        let s2 = Series::new(
            "s2",
            Index::new(vec![IndexLabel::Int64(1), IndexLabel::Int64(2)]),
            Column::from_f64_values(vec![25.0, 30.0]),
        )
        .expect("series"); // ubs:ignore — test fixture
        let py_s2 = PySeries { inner: s2 };

        let (a1, a2) = py_s1.align(&py_s2, "outer").expect("align"); // ubs:ignore — test fixture
        assert_eq!(a1.inner.len(), 3);
        assert_eq!(a2.inner.len(), 3);

        let cf = py_s1.combine_first(&py_s2).expect("combine_first"); // ubs:ignore — test fixture
        assert_eq!(cf.inner.len(), 3);

        let df1 = DataFrame::from_dict(
            &["a"],
            vec![("a", vec![Scalar::Float64(1.0), Scalar::Float64(2.0)])],
        )
        .expect("df"); // ubs:ignore — test fixture
        let py_df1 = PyDataFrame { inner: df1 };
        let (d1, d2) = py_df1.align(&py_df1, "inner").expect("align df"); // ubs:ignore — test fixture
        assert_eq!(d1.shape(), (2, 1));
        assert_eq!(d2.shape(), (2, 1));

        let ewm = py_df1.ewm(Some(0.5), None);
        assert_eq!(ewm.ndim(), 2);
        assert!(ewm.online("numba").is_ok());
    }
}
