# fuzz_dataframe_op_chain artifacts

New crashes from local or CI fuzz runs land here first.
Minimize them with cargo fuzz tmin fuzz_dataframe_op_chain <artifact> and then promote the minimized input into fuzz/corpus/fuzz_dataframe_op_chain/.
