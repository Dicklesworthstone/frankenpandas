# fuzz_column_arith artifacts

New crashes from local or CI fuzz runs land here first.
Minimize them with cargo fuzz tmin fuzz_column_arith <artifact> and then promote the minimized input into fuzz/corpus/fuzz_column_arith/.
