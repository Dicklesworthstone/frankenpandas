# fuzz_rolling_window artifacts

New crashes from local or CI fuzz runs land here first.
Minimize them with cargo fuzz tmin fuzz_rolling_window <artifact> and then promote the minimized input into fuzz/corpus/fuzz_rolling_window/.
