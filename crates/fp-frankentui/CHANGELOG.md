# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0](https://github.com/Dicklesworthstone/frankenpandas/releases/tag/fp-frankentui-v0.1.0) - 2026-04-23

### <!-- 0 -->Added

- *(fp-frame)* land row_multiindex DataFrame field (br-frankenpandas-1zzp.1)
- *(fp-frame)* add to_markdown tablefmt options (frankenpandas-ot0k)
- *(conformance)* add --python-bin CLI flag and join benchmark binary
- *(phase2c)* expand compat-closure evidence packs FP-P2C-006 through FP-P2C-011
- *(fp-frankentui)* update CLI binary with improved integration logic
- *(fp-frankentui)* expand TUI integration library with 387 lines of new functionality
- *(fp-frankentui)* add E2E scenario replay and differential validation
- *(fp-frankentui)* expand TUI crate library with Rust integration documentation
- *(fp-frankentui)* add CLI binary entry point and refine TUI specification artifacts
- *(fp-frankentui)* expand TUI crate library with initial module structure
- *(workspace)* add fp-frankentui crate scaffold and update workspace dependencies

### <!-- 1 -->Fixed

- *(conformance)* require live oracle in CI (br-frankenpandas-d6xa)
- *(docs)* fix misaligned right-side borders in all 3 ASCII art diagrams

### <!-- 4 -->Documentation

- surface Arrow IPC stream as 8th IO format (br-frankenpandas-zbz5)
- per-crate README.md for each fp-* crate (br-frankenpandas-kw5q)
- close row MultiIndex epic (br-frankenpandas-1zzp)
- align conformance claim to actual 430+ packets / 1249 fixtures (br-frankenpandas-zgqj)
- scope SQL parity claim to SQLite-only (br-frankenpandas-m3e8)
- link row MultiIndex roadmap to epic (br-frankenpandas-0yz7)
- split MultiIndex capability into Row vs Column (br-frankenpandas-0yz7)
- qualify drop-in positioning until PyO3 ships (br-frankenpandas-diic)
- expand README to 1574 lines, de-slopify all AI writing artifacts
- expand README to 1440+ lines with missing data, coercion, element-wise ops, selection, introspection
- expand README to 1200+ lines with optimization catalog, error architecture, constructors, merge options
- expand README to 1000+ lines with recipes, deep dives, roadmap
- expand README with deep technical content (+300 lines)
- complete README.md rewrite reflecting current project state
- update README with project status and architecture overview

### Licensing

- adopt MIT + OpenAI/Anthropic rider across workspace and crates
