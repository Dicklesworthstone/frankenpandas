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
use fp_frame::{DataFrame, Series};
use fp_index::{DatetimeIndex, DuplicateKeep, Index, IndexLabel, MultiIndex, format_datetime_ns};
use fp_types::Scalar;
use mimalloc::MiMalloc;
use pyo3::{
    IntoPyObjectExt,
    prelude::*,
    types::{PyDict, PyList},
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

    fn is_unique(&self) -> bool {
        self.inner.is_unique()
    }

    fn equals(&self, other: &PyIndex) -> bool {
        self.inner == other.inner
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
        FrameError::CompatibilityRejected(msg) => (
            PyErrorKind::Value,
            format!("compatibility gate rejected operation: {msg}"),
        ),
        other => (PyErrorKind::Value, other.to_string()),
    }
}

fn frame_error_to_py(err: fp_frame::FrameError) -> PyErr {
    let (kind, msg) = classify_frame_error(&err);
    match kind {
        PyErrorKind::Index => PyErr::new::<pyo3::exceptions::PyIndexError, _>(msg),
        PyErrorKind::Type => PyErr::new::<pyo3::exceptions::PyTypeError, _>(msg),
        PyErrorKind::Value => PyErr::new::<pyo3::exceptions::PyValueError, _>(msg),
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

    /// Map values via an `{old: new}` dict (pandas `Series.map`); unmapped
    /// values follow the Rust core's semantics.
    fn map(&self, py: Python<'_>, mapping: &Bound<'_, PyDict>) -> PyResult<PySeries> {
        let pairs = py_dict_to_scalar_pairs(py, mapping)?;
        let r = self
            .inner
            .map(&pairs)
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

    /// Drop the named columns, returning a new DataFrame (pandas
    /// `DataFrame.drop(columns=...)`).
    fn drop(&self, columns: Vec<String>) -> PyResult<PyDataFrame> {
        let refs: Vec<&str> = columns.iter().map(String::as_str).collect();
        let result = self
            .inner
            .drop_columns(&refs)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PyDataFrame { inner: result })
    }

    /// Rename columns via an `{old: new}` mapping, returning a new DataFrame.
    fn rename(&self, mapping: &Bound<'_, PyDict>) -> PyResult<PyDataFrame> {
        let mut pairs: Vec<(String, String)> = Vec::with_capacity(mapping.len());
        for (k, v) in mapping.iter() {
            pairs.push((k.extract::<String>()?, v.extract::<String>()?));
        }
        let refs: Vec<(&str, &str)> = pairs
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let result = self
            .inner
            .rename(&refs)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
        Ok(PyDataFrame { inner: result })
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

/// FrankenPandas Python module.
#[pymodule]
fn frankenpandas(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PySeries>()?;
    m.add_class::<PyDataFrame>()?;
    m.add_class::<PyGroupBy>()?;
    m.add_class::<PyStyler>()?;
    m.add_class::<PyIndex>()?;
    m.add_class::<PyDatetimeIndex>()?;
    m.add_class::<PyMultiIndex>()?;
    m.add_class::<PySeriesILoc>()?;
    m.add_class::<PySeriesLoc>()?;
    m.add_class::<PySeriesIAt>()?;
    m.add_class::<PySeriesAt>()?;
    m.add_class::<PyDataFrameILoc>()?;
    m.add_class::<PyDataFrameLoc>()?;
    m.add_class::<PyDataFrameIAt>()?;
    m.add_class::<PyDataFrameAt>()?;
    m.add_function(wrap_pyfunction!(read_csv, m)?)?;
    m.add_function(wrap_pyfunction!(read_json, m)?)?;
    m.add_function(wrap_pyfunction!(read_jsonl, m)?)?;
    m.add_function(wrap_pyfunction!(read_parquet, m)?)?;
    m.add_function(wrap_pyfunction!(concat, m)?)?;
    m.add_function(wrap_pyfunction!(merge, m)?)?;
    m.add_function(wrap_pyfunction!(to_numeric, m)?)?;
    m.add_function(wrap_pyfunction!(to_datetime, m)?)?;
    m.add_function(wrap_pyfunction!(date_range, m)?)?;
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
}
