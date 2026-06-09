# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.1](https://github.com/Dicklesworthstone/frankenpandas/compare/fp-conformance-v0.1.0...fp-conformance-v0.1.1) - 2026-06-09

### <!-- 2 -->Performance

- *(fp-frame)* byte-span FxHash tally for string value_counts — 2.64x, bit-identical
- *(fp-frame)* per-group radix sort for f64 groupby rank — 2.72x, bit-identical
- *(fp-frame)* per-group counting-sort histogram for groupby rank — 3.26x, bit-identical
- *(fp-columnar)* LazyNullableUtf8 typed backing for null-introducing Utf8 gather — 1.7x, bit-identical
- *(fp-columnar)* contiguous-gather Scalar-backed all-valid Utf8 in take_positions — 1.8x, bit-identical
- *(fp-io)* extend typed to_csv fast path to Utf8/mixed frames — 2.0x, byte-identical
- *(fp-io)* all-numeric to_csv fast path — 1.87x, byte-identical
