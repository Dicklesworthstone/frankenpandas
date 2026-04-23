# fuzz_query_str_with_locals artifacts

New crashes from local or CI fuzz runs land here first.
Minimize them with cargo fuzz tmin fuzz_query_str_with_locals <artifact> and then promote the minimized input into fuzz/corpus/fuzz_query_str_with_locals/.
