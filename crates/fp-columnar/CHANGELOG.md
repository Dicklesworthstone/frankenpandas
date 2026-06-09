# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.1](https://github.com/Dicklesworthstone/frankenpandas/compare/fp-columnar-v0.1.0...fp-columnar-v0.1.1) - 2026-06-09

### <!-- 2 -->Performance

- *(fp-join)* certify lower-hex UTF8 overlap ranges
- *(fp-frame)* per-group radix sort for f64 groupby rank — 2.72x, bit-identical
- *(fp-columnar)* LazyNullableUtf8 typed backing for null-introducing Utf8 gather — 1.7x, bit-identical
- *(fp-frame)* MSD byte-radix for str-groupby output-label ordering — 1.6x, bit-identical
- *(fp-columnar)* contiguous-gather Scalar-backed all-valid Utf8 in take_positions — 1.8x, bit-identical
- *(fp-join)* carry ordered UTF8 range positions (br-frankenpandas-jbyuc.1.1.1.1.1)
- *(fp-columnar)* zero-copy Float64 range take (br-frankenpandas-jbyuc.1.1.1.1)
