# fuzz_parse_expr artifacts

New crashes from local or CI fuzz runs land here first.
Minimize them with cargo fuzz tmin fuzz_parse_expr <artifact> and then promote the minimized input into fuzz/corpus/fuzz_parse_expr/.
