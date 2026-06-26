//! No-mock conformance guard for Series.str.slice routed through apply_str_utf8
//! (contiguous byte-buffer output). A CONTIGUOUS Utf8 input hits the zero-copy
//! chained-read branch; a SCALAR-BACKED Utf8 input with identical strings hits
//! the Vec<Scalar> write branch / apply_str fallback. Both must produce the
//! bit-identical sliced result across forward-ASCII, negative, step, reverse,
//! and non-ASCII (multi-byte char) cases. A null-bearing scalar input must keep
//! its nulls unchanged (apply_str fallback path).

use fp_columnar::Column;
use fp_frame::Series;
use fp_index::{Index, IndexLabel};
use fp_types::{NullKind, Scalar};

fn contig(words: &[&str]) -> Series {
    let mut bytes = Vec::new();
    let mut offsets = vec![0usize];
    for w in words {
        bytes.extend_from_slice(w.as_bytes());
        offsets.push(bytes.len());
    }
    let labels: Vec<IndexLabel> = (0..words.len() as i64).map(IndexLabel::Int64).collect();
    Series::new(
        "v",
        Index::new(labels),
        Column::from_utf8_contiguous(bytes, offsets),
    )
    .unwrap()
}

fn scalar(words: &[&str]) -> Series {
    let labels: Vec<IndexLabel> = (0..words.len() as i64).map(IndexLabel::Int64).collect();
    Series::from_values(
        "v",
        labels,
        words.iter().map(|w| Scalar::Utf8((*w).into())).collect(),
    )
    .unwrap()
}

#[test]
fn slice_contiguous_equals_scalar_backed() {
    // Mix ASCII and multi-byte (é, ï, 世界) to exercise both the ASCII byte-slice
    // fast arm and the general python_slice_chars arm in write AND fallback.
    let words = [
        "hello world",
        "abcdef",
        "héllo",
        "naïve café",
        "世界abc",
        "x",
        "",
        "ASCII_only_123",
    ];
    let c = contig(&words);
    let s = scalar(&words);
    let cases: &[(Option<i64>, Option<i64>, Option<i64>)] = &[
        (Some(0), Some(5), None),
        (Some(0), Some(5), Some(1)),
        (Some(-3), None, None),
        (Some(0), Some(-1), None),
        (None, None, Some(2)),
        (None, None, Some(-1)),
        (Some(1), Some(5), Some(2)),
        (Some(-1), None, Some(-1)),
        (Some(2), Some(100), None),
        (Some(-100), Some(-1), None),
    ];
    for &(start, stop, step) in cases {
        let rc = c.str().slice(start, stop, step).unwrap();
        let rs = s.str().slice(start, stop, step).unwrap();
        assert_eq!(
            rc.values(),
            rs.values(),
            "contiguous vs scalar slice differ at {start:?}:{stop:?}:{step:?}"
        );
        // And the contiguous result must round-trip as a valid Utf8 column.
        assert_eq!(rc.values().len(), words.len());
    }
}

#[test]
fn slice_scalar_backed_preserves_nulls() {
    let labels: Vec<IndexLabel> = (0..3_i64).map(IndexLabel::Int64).collect();
    let s = Series::from_values(
        "v",
        labels,
        vec![
            Scalar::Utf8("abcdef".into()),
            Scalar::Null(NullKind::NaN),
            Scalar::Utf8("xy".into()),
        ],
    )
    .unwrap();
    let r = s.str().slice(Some(0), Some(3), None).unwrap();
    assert_eq!(r.values()[0], Scalar::Utf8("abc".into()));
    assert!(r.values()[1].is_missing(), "null must be preserved");
    assert_eq!(r.values()[2], Scalar::Utf8("xy".into()));
}
