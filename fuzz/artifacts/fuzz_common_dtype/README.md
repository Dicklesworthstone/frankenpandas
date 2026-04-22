# fuzz_common_dtype artifacts

New crashes from local or CI fuzz runs land here first.
Minimize them with cargo fuzz tmin fuzz_common_dtype <artifact> and then promote the minimized input into fuzz/corpus/fuzz_common_dtype/.
