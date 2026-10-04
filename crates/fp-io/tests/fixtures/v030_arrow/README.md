# Published v0.3.0 writer fixtures

These nine files were produced by the authentic published FrankenPandas 0.3.0
packages, using the public `fp_io` Parquet, Feather and IPC stream writers.
`PROVENANCE.json` records the source commit, producer custody hashes, exact file
sizes and SHA256 digests. No machine-specific paths or credentials are needed to
use this corpus.

Each file contains two rows, two declared row-index levels and two payload
columns. The three cases have named, unnamed and duplicate-named logical index
levels. `__index_level_2__` is deliberately ordinary payload data; only the
declared first two fields belong to the row index.

The old format records logical level names without an independent flat-index
name. Its historical reader convention derives the flat name from the two
physical fields, `__index_level_0__|__index_level_1__`. This is a format control,
not a claim that an independent flat name was persisted.

Keep these exact bytes. Regenerating them with the current writer would remove
the previous-version input from the reader regression. New-format or malformed
metadata constructions belong in separate tests.

During recovery, stock PyArrow 25.0.1 independently decoded all nine files and
verified their schema, values, declared logical names, sizes and SHA256 digests.
That check establishes the corpus contents; it does not establish that the
current FrankenPandas reader passes its compiler or runtime gates. The recovered
reader test proposal remains subject to current-main qualification.
