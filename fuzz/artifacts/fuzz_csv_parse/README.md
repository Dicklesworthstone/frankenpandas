# fuzz_csv_parse artifacts

New crashes from local or CI fuzz runs land here first.
Minimize them with cargo fuzz tmin fuzz_csv_parse <artifact> and then promote the minimized input into fuzz/corpus/fuzz_csv_parse/.
