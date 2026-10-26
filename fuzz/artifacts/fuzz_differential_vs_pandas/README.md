# fuzz_differential_vs_pandas artifacts

New crashes or differential divergences from local or CI fuzz runs land here first.
Minimize them with cargo fuzz tmin fuzz_differential_vs_pandas <artifact> and then promote the minimized input into fuzz/corpus/fuzz_differential_vs_pandas/.
