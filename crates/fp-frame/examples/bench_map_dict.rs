//! NOTE: Series::map_dict takes `&BTreeMap<Scalar, Scalar>`, but `Scalar` does
//! not implement `Ord`, so such a map cannot be populated from Rust and the
//! method has no in-tree callers — it is effectively dead. Left as a no-op so
//! `cargo build --examples` stays green. See bead notes for details.
fn main() {}
