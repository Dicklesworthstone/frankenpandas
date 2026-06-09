# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.1](https://github.com/Dicklesworthstone/frankenpandas/compare/fp-frame-v0.1.0...fp-frame-v0.1.1) - 2026-06-09

### <!-- 2 -->Performance

- *(fp-index)* lazy contiguous-Utf8 Index backing — str-groupby output index, up to 1.6x, bit-identical
- *(fp-frame)* radix sort for standalone Series.rank f64 path — 2.04x, bit-identical
- *(fp-frame)* per-group radix sort for f64 groupby rank — 2.72x, bit-identical
- *(fp-frame)* per-group counting-sort histogram for groupby rank — 3.26x, bit-identical
- *(fp-frame)* MSD byte-radix for str-groupby output-label ordering — 1.6x, bit-identical
- *(fp-frame)* route Scalar-backed all-valid Utf8 Series.sort_values to MSD byte-radix — 2.07x, bit-identical
- *(fp-frame)* typed fast path for DataFrame/Series to_numpy — 3.16x, bit-identical

### <!-- 5 -->Testing

- *(fp-frame)* FP-vs-pandas quantile interpolation differential (5 modes, verified parity)
