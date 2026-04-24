use std::collections::{BTreeMap, BTreeSet};

use fp_frame::DataFrame;
use fp_index::{IndexLabel, align_union, validate_alignment_plan};
use fp_io::{
    IoError as FpIoError, read_feather_bytes, read_ipc_stream_bytes, read_parquet_bytes,
    write_feather_bytes, write_ipc_stream_bytes, write_parquet_bytes,
};

use super::{fuzz_feather_frame_from_bytes, fuzz_index_from_bytes};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FuzzArrowFormat {
    Parquet,
    Feather,
    IpcStream,
}

impl FuzzArrowFormat {
    fn from_byte(byte: u8) -> Self {
        match byte % 3 {
            0 => Self::Parquet,
            1 => Self::Feather,
            _ => Self::IpcStream,
        }
    }

    fn distinct_from_byte(primary: Self, byte: u8) -> Self {
        match primary {
            Self::Parquet => {
                if byte.is_multiple_of(2) {
                    Self::Feather
                } else {
                    Self::IpcStream
                }
            }
            Self::Feather => {
                if byte.is_multiple_of(2) {
                    Self::Parquet
                } else {
                    Self::IpcStream
                }
            }
            Self::IpcStream => {
                if byte.is_multiple_of(2) {
                    Self::Parquet
                } else {
                    Self::Feather
                }
            }
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Parquet => "parquet",
            Self::Feather => "feather",
            Self::IpcStream => "ipc_stream",
        }
    }
}

fn fuzz_roundtrip_frame_via_format(
    format: FuzzArrowFormat,
    frame: &DataFrame,
) -> Result<DataFrame, FpIoError> {
    match format {
        FuzzArrowFormat::Parquet => {
            let encoded = write_parquet_bytes(frame)?;
            read_parquet_bytes(&encoded)
        }
        FuzzArrowFormat::Feather => {
            let encoded = write_feather_bytes(frame)?;
            read_feather_bytes(&encoded)
        }
        FuzzArrowFormat::IpcStream => {
            let encoded = write_ipc_stream_bytes(frame)?;
            read_ipc_stream_bytes(&encoded)
        }
    }
}

/// Structure-aware fuzz entrypoint for cross-format Arrow-backed IO parity.
///
/// The input always runs in synth mode: bytes are projected into a small typed
/// `DataFrame`, round-tripped through a primary Arrow-backed format, then
/// round-tripped again through a distinct secondary format. The logical frame
/// recovered from both stages must converge to the same content.
pub fn fuzz_format_cross_round_trip_bytes(input: &[u8]) -> Result<(), FpIoError> {
    let primary = FuzzArrowFormat::from_byte(input.first().copied().unwrap_or_default());
    let secondary =
        FuzzArrowFormat::distinct_from_byte(primary, input.get(1).copied().unwrap_or(1));
    let payload = input.get(2..).unwrap_or(&[]);
    let source = fuzz_feather_frame_from_bytes(payload)?;
    let primary_frame = fuzz_roundtrip_frame_via_format(primary, &source)?;
    let secondary_frame = fuzz_roundtrip_frame_via_format(secondary, &primary_frame)?;

    if !primary_frame.equals(&secondary_frame) {
        return Err(FpIoError::Io(std::io::Error::other(format!(
            "cross-format round-trip drifted between {} and {}",
            primary.name(),
            secondary.name()
        ))));
    }

    Ok(())
}

/// Structure-aware fuzz entrypoint for `fp-index` outer alignment semantics.
///
/// Input is split at the first `|` byte into left/right index payloads. Each
/// payload byte is projected onto a small `IndexLabel` domain so fuzzing
/// naturally exercises duplicates, overlap, and mixed Int64/Utf8 labels.
pub fn fuzz_index_align_bytes(input: &[u8]) -> Result<(), String> {
    let Some(split_at) = input.iter().position(|byte| *byte == b'|') else {
        return Ok(());
    };

    let left = fuzz_index_from_bytes(&input[..split_at]);
    let right = fuzz_index_from_bytes(&input[split_at + 1..]);
    let plan = align_union(&left, &right);
    validate_alignment_plan(&plan).map_err(|err| err.to_string())?;

    let mut output_counts = BTreeMap::<IndexLabel, usize>::new();
    let mut left_position_counts = BTreeMap::<IndexLabel, BTreeMap<usize, usize>>::new();
    let mut right_position_counts = BTreeMap::<IndexLabel, BTreeMap<usize, usize>>::new();

    for row in 0..plan.union_index.len() {
        let label = &plan.union_index.labels()[row];
        *output_counts.entry(label.clone()).or_default() += 1;

        match plan.left_positions[row] {
            Some(left_pos) => {
                let actual = left.labels().get(left_pos).ok_or_else(|| {
                    format!("left position out of bounds: row={row} pos={left_pos}")
                })?;
                if actual != label {
                    return Err(format!(
                        "left alignment label mismatch: row={row} pos={left_pos} \
                         output={label:?} actual={actual:?}"
                    ));
                }
                *left_position_counts
                    .entry(label.clone())
                    .or_default()
                    .entry(left_pos)
                    .or_default() += 1;
            }
            None if left.labels().iter().any(|candidate| candidate == label) => {
                return Err(format!(
                    "left position missing despite label presence: row={row} label={label:?}"
                ));
            }
            None => {}
        }

        match plan.right_positions[row] {
            Some(right_pos) => {
                let actual = right.labels().get(right_pos).ok_or_else(|| {
                    format!("right position out of bounds: row={row} pos={right_pos}")
                })?;
                if actual != label {
                    return Err(format!(
                        "right alignment label mismatch: row={row} pos={right_pos} \
                         output={label:?} actual={actual:?}"
                    ));
                }
                *right_position_counts
                    .entry(label.clone())
                    .or_default()
                    .entry(right_pos)
                    .or_default() += 1;
            }
            None if right.labels().iter().any(|candidate| candidate == label) => {
                return Err(format!(
                    "right position missing despite label presence: row={row} label={label:?}"
                ));
            }
            None => {}
        }
    }

    let mut left_counts = BTreeMap::<IndexLabel, usize>::new();
    let mut right_counts = BTreeMap::<IndexLabel, usize>::new();
    let mut left_expected_position_counts = BTreeMap::<IndexLabel, BTreeMap<usize, usize>>::new();
    let mut right_expected_position_counts = BTreeMap::<IndexLabel, BTreeMap<usize, usize>>::new();

    for (position, label) in left.labels().iter().enumerate() {
        *left_counts.entry(label.clone()).or_default() += 1;
        left_expected_position_counts
            .entry(label.clone())
            .or_default()
            .insert(position, 0);
    }

    for (position, label) in right.labels().iter().enumerate() {
        *right_counts.entry(label.clone()).or_default() += 1;
        right_expected_position_counts
            .entry(label.clone())
            .or_default()
            .insert(position, 0);
    }

    let labels = left_counts
        .keys()
        .chain(right_counts.keys())
        .cloned()
        .collect::<BTreeSet<_>>();

    for label in labels {
        let left_count = *left_counts.get(&label).unwrap_or(&0);
        let right_count = *right_counts.get(&label).unwrap_or(&0);
        let expected_output_count = match (left_count, right_count) {
            (0, count) => count,
            (count, 0) => count,
            (left_count, right_count) => left_count * right_count,
        };
        let actual_output_count = *output_counts.get(&label).unwrap_or(&0);
        if actual_output_count != expected_output_count {
            return Err(format!(
                "alignment multiplicity mismatch for {label:?}: expected={expected_output_count} \
                 actual={actual_output_count} left_count={left_count} right_count={right_count}"
            ));
        }

        if left_count > 0 && right_count > 0 {
            if let Some(expected_counts) = left_expected_position_counts.get_mut(&label) {
                for count in expected_counts.values_mut() {
                    *count = right_count;
                }
            }
            if let Some(expected_counts) = right_expected_position_counts.get_mut(&label) {
                for count in expected_counts.values_mut() {
                    *count = left_count;
                }
            }
        } else if right_count == 0 {
            if let Some(expected_counts) = left_expected_position_counts.get_mut(&label) {
                for count in expected_counts.values_mut() {
                    *count = 1;
                }
            }
            right_expected_position_counts.remove(&label);
        } else {
            left_expected_position_counts.remove(&label);
            if let Some(expected_counts) = right_expected_position_counts.get_mut(&label) {
                for count in expected_counts.values_mut() {
                    *count = 1;
                }
            }
        }

        let actual_left = left_position_counts
            .get(&label)
            .cloned()
            .unwrap_or_default();
        let expected_left = left_expected_position_counts
            .get(&label)
            .cloned()
            .unwrap_or_default();
        if actual_left != expected_left {
            return Err(format!(
                "left position multiplicity mismatch for {label:?}: \
                 expected={expected_left:?} actual={actual_left:?}"
            ));
        }

        let actual_right = right_position_counts
            .get(&label)
            .cloned()
            .unwrap_or_default();
        let expected_right = right_expected_position_counts
            .get(&label)
            .cloned()
            .unwrap_or_default();
        if actual_right != expected_right {
            return Err(format!(
                "right position multiplicity mismatch for {label:?}: \
                 expected={expected_right:?} actual={actual_right:?}"
            ));
        }
    }

    Ok(())
}
