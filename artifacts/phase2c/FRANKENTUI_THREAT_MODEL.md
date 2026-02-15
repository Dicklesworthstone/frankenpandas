# FRANKENTUI Security + Compatibility Threat Model

Bead: `bd-2gi.28.3` [FRANKENTUI-C]
Subsystem: FRANKENTUI -- terminal user interface operator cockpit (conformance, performance, forensics, decision dashboards)
Source anchors: `FRANKENTUI_ANCHOR_MAP.md` (bd-2gi.28.1), `FRANKENTUI_CONTRACT_TABLE.md` (bd-2gi.28.2)

Doctrine: **fail-closed on unknown/unsafe paths**. Every threat is evaluated against
the operator-facing TUI surface. Where behavior is undefined or terminal capability
is uncertain, the fail-closed doctrine requires FTUI to degrade gracefully or refuse
to render, never to emit garbled output, leak data, or hang the event loop.

---

## 1. Summary

This threat model covers the security, integrity, and compatibility risks of
FRANKENTUI, the planned TUI operator cockpit for FrankenPandas. FTUI is a
read-only consumer of conformance artifacts (`fp-conformance`), decision engine
state (`fp-runtime`), and forensic logs. It renders dashboards for conformance
status, performance baselines, failure forensics, and Bayesian decision audit
trails.

**Scope:** Terminal rendering, data ingestion, file system interaction, user input
handling, cross-process communication, and dependency supply chain. The model
enumerates 6 threat surfaces and 25 specific threats, defines a compatibility
envelope across terminal emulators and operating systems, applies fail-closed
doctrine to 5 failure categories, specifies input validation rules for all data
ingestion points, analyzes dependency chain risks, and provides 15 prioritized
recommendations.

**Exclusions:** Network access (FTUI is local-only), authentication/authorization
(single-user local tool), artifact mutation (FTUI is read-only), and async
runtime concerns (FTUI uses synchronous event loop).

---

## 2. Threat Surface Enumeration

Surfaces are numbered FTS-N (FRANKENTUI Threat Surface) to distinguish from the
ATS-N surfaces in `ASUPERSYNC_THREAT_MODEL.md` and the general TS-N surfaces in
`SECURITY_COMPATIBILITY_THREAT_MATRIX.md`.

### FTS-1: Terminal Rendering Attack Surface

| Property | Value |
|---|---|
| Entry point | Terminal emulator (iTerm2, Alacritty, WezTerm, Windows Terminal, xterm, etc.) |
| Input class | ANSI escape sequences, Unicode codepoints, color codes emitted by FTUI |
| Fail-closed behavior | On unknown terminal capabilities, degrade to 16-color or monochrome |

**Threats:**
- **FTS-1.1 Escape sequence injection via data fields:** If artifact data
  (e.g., `mismatch_summary`, `FailureDigest::replay_command`, `DriftRecord::message`,
  `CiGateResult::errors`) contains embedded ANSI escape sequences, rendering these
  strings raw to the terminal could alter cursor position, change colors, overwrite
  screen regions, or trigger terminal emulator vulnerabilities. The `replay_command`
  field is particularly dangerous because it contains shell command strings that may
  include backticks, `$()` substitutions, or escape characters.
- **FTS-1.2 Terminal emulator CVE exposure:** Certain terminal emulators have
  historical vulnerabilities triggered by specific escape sequences (e.g., OSC
  sequences, DCS strings, title-setting sequences). FTUI's rendering backend
  (crossterm or termion) may emit sequences that trigger these bugs.
- **FTS-1.3 Unicode rendering exploits:** Malformed Unicode in artifact data
  (overlong UTF-8 encodings, bidirectional override characters U+202E, zero-width
  joiners) could cause display corruption, right-to-left text override attacks, or
  terminal rendering hangs on poorly-implemented grapheme cluster handling.
- **FTS-1.4 Wide character width miscalculation:** CJK characters, emoji, and
  other wide characters occupy 2 terminal columns but 1 string character. If
  FTUI's layout engine uses byte length or char count instead of display width,
  table alignment and panel borders will be broken.

### FTS-2: Data Ingestion Surface

| Property | Value |
|---|---|
| Entry point | Artifact JSON files, `drift_history.jsonl`, in-memory types from `fp-runtime`/`fp-conformance` |
| Input class | JSON, JSONL, Rust struct deserialization |
| Fail-closed behavior | Malformed data degrades individual panels; never crashes FTUI |

**Threats:**
- **FTS-2.1 Malformed JSON causing parse panics:** While `serde_json::from_str`
  returns `Result`, deeply nested JSON (stack overflow in parser), extremely long
  strings (OOM), or NaN/Infinity float values (serde_json rejects by default) could
  cause unexpected failures during artifact parsing.
- **FTS-2.2 JSONL partial line from concurrent write:** `drift_history.jsonl` is
  append-only and may be written by a concurrent conformance run. FTUI reading
  the file while a write is in progress may encounter a truncated last line that
  is valid UTF-8 but invalid JSON. This is a race condition between reader and
  writer with no file locking.
- **FTS-2.3 Schema drift in artifact files:** If `fp-conformance` or `fp-runtime`
  adds fields to serialized types without `#[serde(default)]`, FTUI deserialization
  will fail on artifacts produced by a newer version. Conversely, if fields are
  removed, FTUI may attempt to display absent data.
- **FTS-2.4 Unbounded data structures in artifacts:** Several ingested types have
  unbounded `Vec` fields: `ForensicLog::events`, `DecisionRecord::evidence`,
  `PacketParityReport::results`, `CiGateResult::errors`, `RaptorQMetadata::symbol_hashes`.
  A maliciously crafted or pathologically large artifact file could exhaust memory.
- **FTS-2.5 Adversarial floating-point values:** `DecisionMetrics` fields
  (`posterior_compatible`, `expected_loss_allow`, etc.) are `f64` values. While
  serde_json rejects NaN/Infinity by default, values extremely close to 0 or 1
  (e.g., 1e-300) could produce rendering artifacts such as extremely long decimal
  strings, scientific notation in unexpected places, or "0.0000" display for
  nonzero values.

### FTS-3: File System Interaction

| Property | Value |
|---|---|
| Entry point | `artifacts/phase2c/` directory, config files, cache paths |
| Input class | File paths, directory traversal, file metadata |
| Fail-closed behavior | Missing files degrade panels; inaccessible directory degrades dashboard |

**Threats:**
- **FTS-3.1 Symlink following into sensitive directories:** Artifact paths follow
  the deterministic scheme `artifacts/phase2c/{packet_id}/{artifact_file}`. If a
  `packet_id` directory is replaced with a symlink pointing to `/etc/`, `/home/`,
  or other sensitive locations, FTUI would read and display the contents of those
  files in its panels, potentially exposing sensitive data.
- **FTS-3.2 Path traversal via crafted packet_id:** If `packet_id` values in
  artifact filenames contain `../` sequences (e.g., `../../etc/passwd` as a
  packet ID), FTUI could construct paths that escape the artifact directory.
  While `FP-P2C-NNN` is the expected convention, FTUI must validate this.
- **FTS-3.3 Race condition on artifact refresh:** When FTUI re-reads artifact
  files on refresh (per contract rule 3c), an attacker with file system write
  access could swap a valid artifact file for a malicious one between FTUI's
  stat check and read. This is a classic TOCTOU vulnerability.
- **FTS-3.4 Disk exhaustion from cache or log accumulation:** If FTUI maintains
  a parsed artifact cache or writes its own logs, unbounded growth could exhaust
  disk space, affecting the conformance harness and other system components.

### FTS-4: User Input Handling

| Property | Value |
|---|---|
| Entry point | Keyboard events, terminal resize events, mouse events (if enabled) |
| Input class | Raw terminal input via crossterm or termion |
| Fail-closed behavior | Unrecognized input is ignored; no command injection path |

**Threats:**
- **FTS-4.1 Key sequence injection via terminal paste:** If a user pastes text
  containing escape sequences followed by key codes, the terminal input parser
  may interpret embedded sequences as FTUI commands, performing unintended
  navigation, panel switching, or (if implemented) clipboard operations.
- **FTS-4.2 Rapid resize event storm:** A rapid sequence of `SIGWINCH` resize
  events (e.g., programmatic terminal resizing) could cause FTUI to recompute
  layout and re-render on every event, consuming 100% CPU and making the UI
  unresponsive. This is a denial-of-service via event flooding.
- **FTS-4.3 Mouse event flood (if mouse capture enabled):** If FTUI enables
  mouse event capture, rapid mouse movement or programmatic mouse event
  generation could flood the event queue and starve keyboard event processing.
- **FTS-4.4 Clipboard injection via replay command copy:** When FTUI copies a
  `FailureDigest::replay_command` to the clipboard, if the command string
  contains malicious shell commands, the user may paste and execute them. FTUI
  must not modify the command (per INV-FTUI-REPLAY-VERBATIM), but should
  display a warning that clipboard content is a shell command.

### FTS-5: Cross-Process Communication

| Property | Value |
|---|---|
| Entry point | Planned: reading from running FP conformance processes |
| Input class | File-based IPC (artifact files written by conformance harness) |
| Fail-closed behavior | If conformance harness is unavailable, display stale/cached data |

**Threats:**
- **FTS-5.1 Stale data display without staleness indicator:** When the conformance
  harness is not running, FTUI displays artifact files from the last completed run.
  If significant time has passed, these results may be misleading. Without a visible
  staleness indicator (e.g., "data from 3 hours ago"), the operator may act on
  outdated information.
- **FTS-5.2 Partial write visibility:** During an E2E orchestration run, artifacts
  are written sequentially per packet. FTUI may read a completed artifact for
  FP-P2C-001 while FP-P2C-003 is still being generated. The dashboard would show
  a mix of current-run and previous-run data without clear demarcation.
- **FTS-5.3 Process impersonation:** If the artifact directory is world-writable,
  any process could write artifact files that FTUI would display as legitimate
  conformance results. There is no authentication of artifact provenance.
- **FTS-5.4 Forensic log interleaving:** If multiple E2E runs execute concurrently
  (each writing their own `ForensicLog`), FTUI may read a forensic log from one
  run while displaying artifacts from another, producing inconsistent views.

### FTS-6: Dependency Supply Chain

| Property | Value |
|---|---|
| Entry point | `Cargo.toml` dependencies for the FTUI crate |
| Input class | Rust crate ecosystem (crates.io) |
| Fail-closed behavior | Pinned versions; `cargo audit` gating; `#![forbid(unsafe_code)]` |

**Threats:**
- **FTS-6.1 TUI framework vulnerability:** The primary TUI dependency (ratatui
  or equivalent) processes terminal input and generates escape sequences. A
  vulnerability in the framework could allow terminal escape injection, denial of
  service via malformed input sequences, or memory corruption in unsafe blocks.
- **FTS-6.2 Terminal backend vulnerability:** crossterm (or termion/termwiz) handles
  raw terminal I/O including reading from stdin and writing escape sequences. A
  vulnerability could allow code execution via crafted terminal responses (e.g.,
  device status report responses).
- **FTS-6.3 Transitive dependency compromise:** TUI frameworks pull transitive
  dependencies for Unicode width calculation, color handling, and event parsing.
  Any of these could be compromised via supply chain attack.
- **FTS-6.4 Build script injection:** Dependencies with `build.rs` scripts
  execute arbitrary code at compile time. A compromised dependency could inject
  malicious code during the build process.

---

## 3. Threat Matrix

| ID | Surface | Description | Likelihood | Impact | Risk | Mitigation |
|---|---|---|---|---|---|---|
| FTS-1.1 | Rendering | Escape sequence injection via artifact data fields | M | H | HIGH | Sanitize all display strings; strip ANSI escape sequences before rendering |
| FTS-1.2 | Rendering | Terminal emulator CVE triggered by FTUI escape sequences | L | H | MEDIUM | Use well-maintained backend (crossterm); track CVE advisories |
| FTS-1.3 | Rendering | Unicode rendering exploits (BiDi override, overlong UTF-8) | L | M | LOW | Strip BiDi control characters; validate UTF-8 at ingestion |
| FTS-1.4 | Rendering | Wide character width miscalculation breaking layout | M | M | MEDIUM | Use `unicode-width` crate for display width calculation |
| FTS-2.1 | Ingestion | Malformed JSON causing parse failures or stack overflow | M | M | MEDIUM | Limit JSON nesting depth; catch all parse errors; set file size limits |
| FTS-2.2 | Ingestion | JSONL partial line from concurrent write | H | L | MEDIUM | Tolerate trailing incomplete lines; skip and count malformed lines |
| FTS-2.3 | Ingestion | Schema drift between FTUI and artifact producer versions | M | M | MEDIUM | Use `#[serde(default)]` on all optional fields; version-aware deserialization |
| FTS-2.4 | Ingestion | Unbounded Vec fields causing OOM | M | H | HIGH | Enforce size limits: cap events at 50K, evidence at 1K, results at 10K |
| FTS-2.5 | Ingestion | Adversarial floating-point values in DecisionMetrics | L | L | LOW | Clamp display precision to 4 decimal places; handle extremes explicitly |
| FTS-3.1 | Filesystem | Symlink following into sensitive directories | L | H | MEDIUM | Resolve symlinks and validate paths stay within artifact root |
| FTS-3.2 | Filesystem | Path traversal via crafted packet_id (`../` injection) | L | H | MEDIUM | Validate packet_id against `^FP-P2C-\d{3}$` regex; reject others |
| FTS-3.3 | Filesystem | TOCTOU race on artifact file refresh | L | M | LOW | Accept as inherent; display file modification timestamp |
| FTS-3.4 | Filesystem | Disk exhaustion from FTUI cache growth | L | M | LOW | Bound cache size; no persistent logs by default |
| FTS-4.1 | Input | Key sequence injection via terminal paste | M | M | MEDIUM | Use bracketed paste mode if supported; ignore invalid sequences |
| FTS-4.2 | Input | Rapid resize event storm causing CPU exhaustion | M | M | MEDIUM | Debounce resize events (50ms minimum interval between relayouts) |
| FTS-4.3 | Input | Mouse event flood starving keyboard processing | L | L | LOW | Rate-limit mouse events; process keyboard events with priority |
| FTS-4.4 | Input | Clipboard injection via replay command copy | M | H | HIGH | Display warning "copied shell command to clipboard" on copy action |
| FTS-5.1 | Cross-process | Stale data display without staleness indicator | H | M | HIGH | Display artifact file modification timestamp; warn if >1 hour old |
| FTS-5.2 | Cross-process | Partial write visibility during active E2E run | M | M | MEDIUM | Display "run in progress" indicator; show per-packet write timestamps |
| FTS-5.3 | Cross-process | Process impersonation via writable artifact directory | L | H | MEDIUM | Warn if artifact directory has world-write permissions |
| FTS-5.4 | Cross-process | Forensic log interleaving from concurrent runs | L | M | LOW | Validate `run_ts_unix_ms` consistency within a single forensic log |
| FTS-6.1 | Supply chain | TUI framework vulnerability (ratatui) | L | H | MEDIUM | Pin version; run `cargo audit`; review changelogs on update |
| FTS-6.2 | Supply chain | Terminal backend vulnerability (crossterm) | L | H | MEDIUM | Pin version; run `cargo audit`; prefer crossterm (largest community) |
| FTS-6.3 | Supply chain | Transitive dependency compromise | L | H | MEDIUM | Use `cargo-deny`; audit transitive tree; minimize dependency count |
| FTS-6.4 | Supply chain | Build script injection in dependency | L | H | MEDIUM | Audit `build.rs` files in new dependencies; use `cargo-deny` |

---

## 4. Compatibility Envelope

### 4.1 Terminal Emulator Compatibility

| Terminal | Platform | True Color | 256 Color | Unicode | Mouse | Status |
|---|---|---|---|---|---|---|
| iTerm2 | macOS | Yes | Yes | Full | Yes | Tier 1 (primary target) |
| Alacritty | Linux/macOS/Windows | Yes | Yes | Full | Yes | Tier 1 |
| WezTerm | Linux/macOS/Windows | Yes | Yes | Full | Yes | Tier 1 |
| Windows Terminal | Windows | Yes | Yes | Full | Yes | Tier 1 |
| kitty | Linux/macOS | Yes | Yes | Full | Yes | Tier 2 |
| GNOME Terminal | Linux | Yes | Yes | Full | Yes | Tier 2 |
| xterm | Linux/macOS | Partial | Yes | Partial | Yes | Tier 2 (degraded rendering) |
| macOS Terminal.app | macOS | No | Yes | Partial | Yes | Tier 2 (256-color fallback) |
| tmux | Any (multiplexer) | Yes (passthrough) | Yes | Full | Partial | Tier 2 (test inside tmux) |
| screen | Any (multiplexer) | No | Yes | Partial | No | Tier 3 (256-color, no mouse) |
| Linux console (VT) | Linux | No | No | Minimal | No | Tier 3 (16-color, ASCII-only) |
| PuTTY | Windows (SSH) | No | Yes | Partial | Partial | Tier 3 (degraded) |

**Tier definitions:**
- **Tier 1:** Full feature support guaranteed. CI testing targets these terminals.
- **Tier 2:** Functional with possible visual degradation. Manual testing.
- **Tier 3:** Basic functionality only. Graceful degradation to minimal rendering.

### 4.2 Unicode and Emoji Rendering

| Feature | Requirement | Fallback |
|---|---|---|
| Basic Latin + digits | Required (all terminals) | None (hard requirement) |
| Box-drawing characters (U+2500-U+257F) | Required for panel borders | ASCII fallback: `+-|` |
| Block elements (U+2580-U+259F) | Required for sparkline charts | ASCII fallback: `#=-` |
| Braille patterns (U+2800-U+28FF) | Desired for high-resolution charts | Block element fallback |
| Emoji (status indicators) | Not used by default | N/A (use text badges: `[PASS]`, `[FAIL]`) |
| CJK characters in data fields | Must render correctly if present | Display width via `unicode-width` crate |
| BiDi control characters | Stripped at ingestion | N/A (security requirement FTS-1.3) |

### 4.3 Color Depth Requirements

| Mode | Colors | Detection | Usage |
|---|---|---|---|
| True color (24-bit) | 16.7M | `$COLORTERM=truecolor` or terminal query | Full palette: smooth gradients in trend charts, precise severity colors |
| 256 color | 256 | `$TERM` contains `256color` | Reduced palette: mapped severity colors, dithered charts |
| 16 color | 16 | Default ANSI | Minimal: bold/dim for emphasis, standard ANSI colors for status |
| Monochrome | 2 | `$NO_COLOR` set or `$TERM=dumb` | Text-only: `[PASS]`/`[FAIL]` badges, ASCII art borders |

**Fail-closed rule:** If color depth cannot be determined, default to 16-color mode.
Never assume true color without detection.

### 4.4 Minimum Terminal Size

| View | Min Width | Min Height | Below-Minimum Behavior |
|---|---|---|---|
| Any dashboard | 80 columns | 24 rows | Display "Terminal too small (min: 80x24)" and block rendering |
| Conformance dashboard | 100 columns | 30 rows | Functional at 80x24 with truncated packet cards |
| Drift trend chart | 60 columns | 10 rows | Chart panel requires minimum; omitted if insufficient |
| CI pipeline bar | 70 columns | 8 rows | Stacked vertical layout at narrow widths |
| Help overlay | 60 columns | 20 rows | Scrollable if terminal is too short |

**Fail-closed rule:** Below 80x24, FTUI refuses to render dashboards and displays
a minimum-size message. It automatically resumes on resize above threshold.

### 4.5 Operating System Compatibility

| OS | Terminal Backend | Raw Mode | Signal Handling | Status |
|---|---|---|---|---|
| Linux (x86_64, aarch64) | crossterm (termios) | Full | SIGWINCH, SIGINT, SIGTERM | Tier 1 |
| macOS (x86_64, aarch64) | crossterm (termios) | Full | SIGWINCH, SIGINT, SIGTERM | Tier 1 |
| Windows 10/11 | crossterm (Win32 Console API) | Full | CTRL_C_EVENT, window resize | Tier 1 |
| FreeBSD | crossterm (termios) | Expected | Standard POSIX signals | Tier 3 (untested) |
| WSL2 | crossterm (termios via Linux kernel) | Full | Standard POSIX signals | Tier 2 |

### 4.6 Rust Edition and MSRV

| Property | Value | Rationale |
|---|---|---|
| Rust edition | 2024 | Workspace-level setting in `Cargo.toml` |
| MSRV (minimum supported Rust version) | 1.85.0 | Edition 2024 requires Rust 1.85+ |
| `#![forbid(unsafe_code)]` | Required | Matches `fp-runtime` and `fp-conformance` policy |
| serde version | 1.0.219+ | Workspace dependency; must match existing crates |
| serde_json version | 1.0.140+ | Workspace dependency; must match existing crates |

---

## 5. Fail-Closed Doctrine Application

For each failure scenario, the fail-closed doctrine prescribes the safest
behavior. FTUI never guesses, never renders garbled output, and never silently
ignores data integrity issues.

### 5.1 Malformed Data

| Scenario | Fail-Closed Response |
|---|---|
| Artifact JSON file fails to parse | Display "parse error: {path}: {error}" in the affected panel. Other panels render normally. Do not retry automatically. |
| `drift_history.jsonl` line is not valid JSON | Skip the malformed line. Increment a "skipped lines" counter displayed in the trend panel footer. Continue processing remaining lines. |
| `drift_history.jsonl` trailing incomplete line | Skip the incomplete line silently (likely concurrent write). Do not count as malformed. |
| Artifact JSON exceeds 10MB size limit | Refuse to parse. Display "artifact too large: {path} ({size}MB > 10MB limit)". |
| JSON nesting depth exceeds 64 levels | Refuse to parse. Display "artifact rejected: excessive nesting depth". |
| Deserialization produces unexpected `None` for required field | Display "incomplete data: {field} missing in {path}". Render available fields; mark missing fields as "N/A". |
| `f64` value is `NaN` or `Infinity` after deserialization | Display as "NaN" or "Inf" with warning color. Do not use in calculations or chart rendering. |

### 5.2 Terminal Too Small

| Scenario | Fail-Closed Response |
|---|---|
| Terminal width < 80 or height < 24 | Clear screen. Display centered message: "FRANKENTUI requires minimum 80x24 terminal. Current: {W}x{H}". Block all dashboard rendering. |
| Terminal resized below minimum during operation | Abort current render. Switch to minimum-size message. Automatically resume when resized above threshold. |
| Terminal resized during mid-render | Drop the current frame. Recompute layout for new dimensions. Render fresh frame. Accept one dropped frame as the cost of correctness. |

### 5.3 Required Files Missing

| Scenario | Fail-Closed Response |
|---|---|
| `artifacts/phase2c/` directory does not exist | Display "Artifact directory not found: {path}" at dashboard level. All conformance panels show "no data". Decision and performance dashboards may still function from in-memory data. |
| `artifacts/phase2c/` exists but is not readable | Display "Permission denied: {path}". Same degradation as missing directory. |
| Specific packet artifact file missing | Display "artifact not found: {path}" in that packet's panel only. Other packets render normally. |
| `drift_history.jsonl` missing | Display "No drift history available" in the trend panel. Other conformance panels function normally. |
| Parity gate config YAML missing | Display "gate config not found: {path}" in gate panel. Gate evaluation skipped for that packet. |

### 5.4 Conformance Harness Unavailable

| Scenario | Fail-Closed Response |
|---|---|
| No artifact files exist (fresh install) | Display "No conformance data available. Run the E2E harness to generate artifacts." |
| Artifacts exist but are from a previous run | Display artifacts with modification timestamp. Show warning if artifacts are >1 hour old: "Data may be stale (last modified: {timestamp})". |
| E2E run in progress (partial artifacts) | Display available artifacts. Show "Run in progress" indicator if a forensic log file is being written (detected by file modification time within last 60 seconds). |

### 5.5 Graceful Degradation Strategy

FTUI implements a 4-level degradation ladder:

| Level | Trigger | Behavior |
|---|---|---|
| **L0: Full** | All data available, terminal capable | Full dashboard rendering with all panels, charts, and interactivity |
| **L1: Data-Degraded** | Some artifact files missing or malformed | Affected panels show error messages; unaffected panels render normally; navigation remains functional |
| **L2: Display-Degraded** | Terminal lacks capabilities (no true color, small size, no Unicode) | Reduced visual fidelity: 16-color, ASCII borders, simplified charts; all data still accessible |
| **L3: Minimal** | Terminal below minimum size or in `$TERM=dumb` mode | Text-only output; no interactive dashboards; dump summary to stdout and exit |

---

## 6. Input Validation Model

### 6.1 Artifact File Ingestion

| Ingestion Point | Validation Rule | Rejection Behavior |
|---|---|---|
| `parity_report.json` | Must parse as `PacketParityReport`; `fixture_count` >= 0; `passed + failed <= fixture_count` | Panel shows "invalid parity report" |
| `parity_gate_result.json` | Must parse as `PacketGateResult`; `packet_id` non-empty | Panel shows "invalid gate result" |
| `parity_mismatch_corpus.json` | Must parse as `Vec<CaseResult>`; cap at 10,000 entries | Truncate at cap; show "showing 10000 of N" |
| `parity_report.raptorq.json` | Must parse as `RaptorQSidecarArtifact` | Panel shows "invalid RaptorQ sidecar" |
| `parity_report.decode_proof.json` | Must parse as `DecodeProofArtifact` | Panel shows "invalid decode proof" |
| `drift_history.jsonl` | Per-line parse as `PacketDriftHistoryEntry`; skip failures | Show "N lines skipped" in footer |
| Any artifact file | Size limit: 10MB per file | Refuse to load; show size error |
| Any artifact file | UTF-8 validation | Refuse non-UTF-8 files |

### 6.2 Display String Sanitization

All strings extracted from artifact data must be sanitized before rendering to
the terminal. The sanitization pipeline:

1. **UTF-8 validation:** Already guaranteed by serde_json (JSON is UTF-8).
2. **Control character stripping:** Remove all C0 control characters (U+0000-U+001F)
   except TAB (U+0009) and NEWLINE (U+000A). Remove all C1 control characters
   (U+0080-U+009F).
3. **ANSI escape sequence stripping:** Remove all sequences matching the pattern
   `\x1B\[[\x30-\x3F]*[\x20-\x2F]*[\x40-\x7E]` (CSI sequences) and
   `\x1B[\x40-\x5F]` (two-byte sequences). This prevents injected escape codes
   from altering terminal state.
4. **BiDi control character removal:** Strip U+200E (LRM), U+200F (RLM),
   U+202A-U+202E (embedding/override), U+2066-U+2069 (isolate) to prevent
   right-to-left text reordering attacks.
5. **Display width calculation:** Use `unicode-width` crate for correct column
   width computation. Truncate strings at display width, not byte or char count.
6. **Length capping:** `mismatch_summary` capped at 200 display characters with
   ellipsis. `replay_command` displayed in full (may be scrollable). `DriftRecord::message`
   capped at 500 display characters. `CiGateResult::errors` entries capped at 200
   each.

### 6.3 Size Limits on Data Loading

| Data Source | Soft Limit | Hard Limit | Enforcement |
|---|---|---|---|
| `drift_history.jsonl` | 10,000 lines | 100,000 lines | Hard: read only last 100K lines; show "truncated" |
| `ForensicLog::events` | 1,000 events | 50,000 events | Hard: show last 50K; virtual scrolling at soft limit |
| `parity_mismatch_corpus.json` entries | 100 entries | 10,000 entries | Hard: truncate; paginate at soft limit |
| `EvidenceLedger::records` | 500 records | 10,000 records | Hard: show last 10K; paginate at soft limit |
| `DecisionRecord::evidence` terms | 50 terms | 1,000 terms | Hard: cap at 50; show "and N more" |
| Individual artifact file size | 1 MB | 10 MB | Warn at soft; refuse at hard |
| Total loaded data in memory | 100 MB | 500 MB | Warn at soft; refuse new loads at hard |

### 6.4 Schema Validation for JSON/JSONL Inputs

FTUI uses serde deserialization as the primary schema validation mechanism.
Additional structural checks:

| Check | Rule | On Failure |
|---|---|---|
| `packet_id` format | Must match `^FP-P2C-\d{3}$` or be treated as opaque string | Render raw string; skip sort optimization |
| `ts_unix_ms` range | Must be `> 0` and `< 2^53` (JavaScript safe integer) | Display as "unknown" if 0; reject if negative or overflow |
| `fixture_count` consistency | `passed + failed <= fixture_count` | Display warning "inconsistent counts" |
| `gate_pass` consistency | `pass == true` implies `strict_failed == 0` (in strict config) | Display both fields; let operator interpret |
| `report_hash` format | Expected to be hex string; no validation enforced | Display raw string |
| `source_hash` sentinel | `"blake3:placeholder"` is a known sentinel | Render as "placeholder" status |

---

## 7. Dependency Chain Risks

### 7.1 Primary TUI Dependencies (Planned)

| Dependency | Version (Expected) | Contains `unsafe` | CVE History | Platform Support |
|---|---|---|---|---|
| `ratatui` | ~0.29+ | No (pure Rust) | No known CVEs | All (terminal-agnostic) |
| `crossterm` | ~0.28+ | Yes (platform I/O) | No known CVEs | Linux, macOS, Windows |
| `unicode-width` | ~0.2+ | No | No known CVEs | All |
| `unicode-segmentation` | ~1.12+ | No | No known CVEs | All |

**Note:** The `frankentui` crate referenced in the FrankenSQLite spec does not
exist. FTUI will likely be built directly on `ratatui` + `crossterm`, the
dominant Rust TUI stack. If `frankentui` is implemented as a wrapper, its
dependency tree would include these crates transitively.

### 7.2 Transitive Dependencies (crossterm)

| Dependency | Purpose | Risk |
|---|---|---|
| `mio` | Event-driven I/O | LOW -- well-maintained; contains unsafe for platform I/O |
| `signal-hook` | POSIX signal handling | LOW -- minimal unsafe; Unix-only |
| `signal-hook-mio` | Signal integration with mio | LOW -- thin adapter |
| `parking_lot` | Efficient synchronization | LOW -- well-audited unsafe |
| `libc` | FFI bindings (Unix) | MEDIUM -- unsafe FFI; platform-dependent |
| `winapi` / `windows-sys` | Win32 API bindings (Windows) | MEDIUM -- unsafe FFI; Windows-only |
| `bitflags` | Type-safe bitflags | LOW -- no unsafe |

### 7.3 Transitive Dependencies (ratatui)

| Dependency | Purpose | Risk |
|---|---|---|
| `unicode-width` | Display width calculation | LOW -- pure Rust; widely used |
| `unicode-segmentation` | Grapheme cluster handling | LOW -- pure Rust |
| `itertools` | Iterator utilities | LOW -- no unsafe |
| `strum` / `strum_macros` | Enum iteration/display | LOW -- proc macro |
| `compact_str` | Small string optimization | LOW -- minimal unsafe |
| `instability` | Stability attribute macros | LOW -- proc macro only |

### 7.4 Supply Chain Risk Mitigation

| Vector | Description | Mitigation |
|---|---|---|
| Typosquatting | Misspelled crate name in `Cargo.toml` | Pin exact crate names; code review on dependency changes |
| Malicious patch release | Compromised maintainer pushes backdoor | Pin exact versions with `=` in `Cargo.toml`; run `cargo audit` in CI |
| Transitive vulnerability | CVE in deep dependency | Regular `cargo audit`; Dependabot/RenovateBot alerts |
| Build script injection | Dependency `build.rs` runs arbitrary code | Audit build scripts; use `cargo-deny` with `[bans]` table |
| Dependency confusion | Private registry shadowed | FrankenPandas uses only crates.io; no private registries |

### 7.5 `unsafe` Code Audit Surface

FTUI itself must use `#![forbid(unsafe_code)]` (per INV-FTUI-NO-UNSAFE).
Unsafe code exists only in dependencies:

| Crate | Unsafe Usage | Justification | Audit Status |
|---|---|---|---|
| `crossterm` | Platform terminal I/O | Required for raw mode | Actively maintained; community-reviewed |
| `mio` | Event-driven I/O primitives | Required for event polling | Tokio ecosystem; well-audited |
| `libc` | POSIX FFI bindings | Required on Unix | Rust project-adjacent; foundational |
| `parking_lot` | Lock-free synchronization | Performance optimization | Well-audited; RustSec clean |
| `signal-hook` | Signal handler registration | Required for SIGWINCH | Minimal unsafe surface |

---

## 8. Information Disclosure Risks

### 8.1 Sensitive Data in Display

| Data Category | Sensitivity | Exposure Vector | Mitigation |
|---|---|---|---|
| File system paths (artifact paths, replay commands) | LOW-MEDIUM | Visible on screen; reveals directory structure | Accept as inherent; paths are local-only |
| Conformance failure details (mismatch summaries) | LOW | Reveals internal data representation differences | Accept as inherent; this is the tool's purpose |
| Bayesian decision metrics (posteriors, loss values) | LOW | Reveals decision engine parameters | Accept as inherent; operator needs this information |
| `replay_command` strings | MEDIUM | Contains shell commands; may include file paths | Display as-is (per contract); warn on clipboard copy |
| Environment variables (if displayed) | HIGH | Could contain secrets (API keys, tokens) | FTUI must never read or display environment variables |
| `CiGateResult::errors` strings | MEDIUM | Could contain stack traces with memory addresses | Sanitize; strip ANSI escapes; truncate at 200 chars |
| `ForensicEventKind::Error` messages | MEDIUM | Could contain internal error details | Display as-is (operator-facing tool); sanitize escapes |

### 8.2 Screen Capture and Recording Concerns

| Concern | Risk | Mitigation |
|---|---|---|
| Terminal session recording (asciinema, script) | Data visible in recording | Document in FTUI help: "terminal recordings capture all displayed data" |
| Screen sharing / remote desktop | Data visible to observers | Not FTUI's responsibility; operational security concern |
| Terminal scrollback buffer | Data persists in buffer after FTUI exits | Use alternate screen buffer; restore on exit (standard TUI practice) |
| Core dumps on crash | In-memory data dumped to disk | `#![forbid(unsafe_code)]` prevents most crash scenarios; no secrets in memory |

### 8.3 Log File Content

| Log Type | Content | Sanitization |
|---|---|---|
| FTUI operational logs (if any) | Artifact parse errors, render timing, user actions | Strip file contents; log only file paths and error types |
| stderr output | Panic messages, error propagation | Ensure no artifact data in panic messages |
| No persistent logs by default | FTUI should not write logs unless explicitly enabled | Default: no log files; opt-in via `--log` flag |

### 8.4 Environment Variable Exposure

FTUI reads the following environment variables for configuration:

| Variable | Purpose | Sensitive | Handling |
|---|---|---|---|
| `$TERM` | Terminal type detection | No | Read-only |
| `$COLORTERM` | Color depth detection | No | Read-only |
| `$NO_COLOR` | Disable color output | No | Read-only |
| `$FTUI_ARTIFACT_DIR` (planned) | Override artifact directory path | No | Validate path; reject non-directory |
| `$HOME` | Config file location | LOW | Used for path construction only; never displayed |

**Invariant:** FTUI must never display or log the values of environment variables
other than those listed above. In particular, it must never access `$AWS_SECRET_ACCESS_KEY`,
`$DATABASE_URL`, `$API_KEY`, or similar credential variables.

---

## 9. Availability Threats

### 9.1 Render Loop Hang Scenarios

| Scenario | Cause | Impact | Mitigation |
|---|---|---|---|
| Infinite loop in chart rendering | Edge case in sparkline algorithm (zero-range data) | UI freeze; terminal left in raw mode | Set render timeout (100ms per frame); watchdog timer restores terminal on timeout |
| Blocking file I/O in render path | Reading large artifact file synchronously during render | Frame drops; unresponsive UI | Perform all I/O in background; render only from cached data |
| Layout computation explosion | Deeply nested widget tree with constraint conflicts | CPU spin; unresponsive UI | Cap layout iterations at 10; accept imperfect layout over hang |
| Event loop deadlock | Lock contention between input handler and renderer | Complete freeze | Single-threaded event loop (no locks); render and input in same thread |

### 9.2 Large Data Set Rendering

| Data Set | Size Threshold | Without Mitigation | With Mitigation |
|---|---|---|---|
| Forensic event log | >10,000 events | Render takes >1s; UI appears frozen | Virtual scrolling: render 50-event window only |
| Mismatch corpus | >1,000 entries | Layout computation >500ms | Pagination: 25 entries per page |
| Drift history | >10,000 entries | Chart rendering >1s | Downsample: reduce to 1,000 data points for chart |
| Evidence terms | >100 per record | Detail view takes >200ms | Cap at 50 terms; "and N more" indicator |
| Packet count | >50 packets | Packet list layout >200ms | Scrollable list with virtual rendering |

### 9.3 Event Loop Blocking

| Blocker | Cause | Impact | Mitigation |
|---|---|---|---|
| Artifact file read | Synchronous `fs::read_to_string()` on slow disk | Dropped frames during refresh | Read files in chunks; yield to event loop between reads |
| JSON parsing | Large mismatch corpus parsing | Dropped frames during load | Parse incrementally or in background; show loading indicator |
| JSONL streaming | Reading 100K-line drift history | UI blocked for full parse duration | Read in batches of 1,000 lines; render progressively |
| Clipboard operation | Platform clipboard API may block | Brief UI pause | Perform clipboard operations asynchronously; show "copied" after completion |

### 9.4 Crash Recovery (Terminal State Restoration)

FTUI operates in raw terminal mode (no line buffering, no echo, no signal
processing by the terminal driver). If FTUI crashes or is killed, the terminal
must be restored to a usable state.

| Scenario | Terminal State After | Recovery |
|---|---|---|
| Normal exit (quit command) | Restored: alternate screen exited, cursor visible, raw mode off | Automatic via drop handler |
| Panic (Rust panic) | **Corrupted:** raw mode active, alternate screen, cursor hidden | Panic hook must call terminal restore before unwinding |
| SIGINT (Ctrl+C) | **Corrupted** without handler | Signal handler must restore terminal then exit |
| SIGTERM (kill) | **Corrupted** without handler | Signal handler must restore terminal then exit |
| SIGKILL (kill -9) | **Corrupted:** no handler possible | User runs `reset` or `stty sane` manually |
| OOM killer | **Corrupted:** no handler possible | User runs `reset` manually |

**Fail-closed rule:** FTUI must install a panic hook and signal handlers that
restore terminal state before process exit. The `scopeguard` or custom `Drop`
pattern on the terminal handle must execute even during panic unwinding.

**Implementation pattern:**
```
// Pseudocode for terminal restoration guarantee
let _guard = scopeguard::guard(terminal, |mut t| {
    let _ = t.leave_alternate_screen();
    let _ = t.show_cursor();
    let _ = crossterm::terminal::disable_raw_mode();
});
```

---

## 10. Prioritized Recommendations

### P0: Critical (Must Implement Before First FTUI Release)

**R-01: Implement display string sanitization for all rendered artifact data.**
- Strip ANSI escape sequences, C0/C1 control characters, and BiDi overrides
  from all strings before rendering.
- Use a dedicated `sanitize_display_string(input: &str) -> String` function
  called at the data ingestion boundary, not at render time.
- **Addresses:** FTS-1.1, FTS-1.3
- **Blocked by:** Nothing.
- **Blocks:** Safe rendering of untrusted artifact data.

**R-02: Install panic hook and signal handlers for terminal state restoration.**
- Register a custom panic hook (`std::panic::set_hook`) that restores terminal
  state (disable raw mode, leave alternate screen, show cursor) before printing
  the panic message.
- Register signal handlers for SIGINT and SIGTERM that restore terminal state
  and exit cleanly.
- **Addresses:** Section 9.4 crash recovery.
- **Blocked by:** Terminal backend choice (crossterm provides helpers).
- **Blocks:** Usable crash recovery.

**R-03: Enforce `#![forbid(unsafe_code)]` in the FTUI crate.**
- Add `#![forbid(unsafe_code)]` to the FTUI crate root, consistent with
  `fp-runtime` and `fp-conformance`.
- All unsafe operations are delegated to dependencies (crossterm, etc.).
- **Addresses:** INV-FTUI-NO-UNSAFE.
- **Blocked by:** Nothing.
- **Blocks:** Memory safety assurance.

**R-04: Implement size limits on all data ingestion points.**
- Enforce per-file size limit (10MB).
- Enforce per-collection caps (50K events, 10K records, 1K evidence terms).
- Reject oversized inputs at the ingestion boundary with clear error messages.
- **Addresses:** FTS-2.4, Section 6.3.
- **Blocked by:** Nothing.
- **Blocks:** OOM prevention.

### P1: High (Must Implement Before Operator Use)

**R-05: Validate and sanitize `packet_id` in file path construction.**
- Validate `packet_id` against expected pattern (`FP-P2C-\d{3}`) when
  constructing file paths.
- Reject `packet_id` values containing path separator characters (`/`, `\`, `..`).
- Canonicalize constructed paths and verify they remain within the artifact root.
- **Addresses:** FTS-3.1, FTS-3.2.
- **Blocked by:** Nothing.
- **Blocks:** Path traversal prevention.

**R-06: Display artifact staleness indicators.**
- Show file modification timestamp alongside each artifact panel.
- Display warning when artifacts are older than 1 hour.
- Show "Run in progress" when forensic log file is actively being written.
- **Addresses:** FTS-5.1, FTS-5.2.
- **Blocked by:** Nothing.
- **Blocks:** Operator trust in displayed data.

**R-07: Implement resize event debouncing.**
- Debounce `SIGWINCH` / terminal resize events with a 50ms minimum interval.
- Coalesce rapid resize events into a single relayout.
- **Addresses:** FTS-4.2.
- **Blocked by:** Event loop implementation.
- **Blocks:** UI responsiveness under rapid resize.

**R-08: Implement virtual scrolling for all unbounded lists.**
- Forensic event timeline: virtual scroll at >1,000 events.
- Mismatch corpus: paginate at >100 entries.
- Evidence ledger: paginate at >500 records.
- Drift history: downsample at >1,000 entries for charts.
- **Addresses:** Section 9.2.
- **Blocked by:** Widget framework selection.
- **Blocks:** Performance with large data sets.

### P2: Medium (Should Implement Before Production Use)

**R-09: Implement clipboard copy warning for shell commands.**
- When user copies a `replay_command` to clipboard, display a transient
  notification: "Copied shell command to clipboard. Review before executing."
- **Addresses:** FTS-4.4.
- **Blocked by:** Clipboard integration.
- **Blocks:** Safe operator workflow.

**R-10: Use `unicode-width` for all layout width calculations.**
- Replace any byte-length or char-count width calculations with
  `UnicodeWidthStr::width()` from the `unicode-width` crate.
- Handle wide characters (CJK), zero-width joiners, and combining characters.
- **Addresses:** FTS-1.4.
- **Blocked by:** Nothing.
- **Blocks:** Correct table alignment.

**R-11: Implement color depth detection and graceful fallback.**
- Detect true color via `$COLORTERM`.
- Detect 256-color via `$TERM`.
- Respect `$NO_COLOR` for monochrome mode.
- Implement fallback palettes for each depth level.
- **Addresses:** Section 4.3.
- **Blocked by:** Theme system implementation.
- **Blocks:** Cross-terminal compatibility.

**R-12: Add `cargo-deny` configuration for FTUI dependency tree.**
- Deny known-vulnerable dependency versions.
- Deny duplicate dependency versions.
- Audit build scripts of new direct dependencies.
- Run in CI as a gating check.
- **Addresses:** FTS-6.3, FTS-6.4.
- **Blocked by:** Nothing.
- **Blocks:** Supply chain risk reduction.

### P3: Low (Deferred to Maturity Phase)

**R-13: Implement bracketed paste mode for input handling.**
- Enable bracketed paste mode to distinguish typed input from pasted text.
- Ignore any escape sequences embedded in pasted content.
- **Addresses:** FTS-4.1.
- **Blocked by:** Terminal backend support.
- **Blocks:** Paste-based injection prevention.

**R-14: Warn on world-writable artifact directory.**
- Check permissions of `artifacts/phase2c/` directory on startup.
- Display warning if directory is world-writable: "Artifact directory has
  open permissions. Data may be untrusted."
- **Addresses:** FTS-5.3.
- **Blocked by:** Nothing.
- **Blocks:** Artifact provenance awareness.

**R-15: Implement render timeout watchdog.**
- Set a 100ms timeout on each render frame.
- If rendering exceeds timeout, abort the frame, display "render timeout"
  in status bar, and retry with simplified rendering.
- If 3 consecutive timeouts occur, switch to degradation level L2.
- **Addresses:** Section 9.1.
- **Blocked by:** Event loop architecture.
- **Blocks:** Hang prevention.

---

## 11. Appendix A: Machine-Checkable Invariants for FTUI Security Properties

| Invariant ID | Statement | Check Method | Priority |
|---|---|---|---|
| INV-FTUI-NO-UNSAFE | FTUI crate uses `#![forbid(unsafe_code)]` | Compiler enforcement | P0 |
| INV-FTUI-SANITIZE-DISPLAY | All artifact strings pass through `sanitize_display_string()` before rendering | Code review; grep for direct string rendering bypassing sanitizer | P0 |
| INV-FTUI-NO-ESCAPE-PASSTHROUGH | No ANSI escape sequences from artifact data reach the terminal | Unit test: inject `\x1B[31m` in `mismatch_summary`; verify stripped | P0 |
| INV-FTUI-TERMINAL-RESTORE | Panic hook and signal handlers restore terminal state | Integration test: trigger panic; verify terminal is usable after | P0 |
| INV-FTUI-FILE-SIZE-LIMIT | Artifact files >10MB are rejected at ingestion | Unit test: create 11MB file; verify rejection | P0 |
| INV-FTUI-COLLECTION-CAP | All unbounded collections capped at defined limits | Unit test: inject 100K forensic events; verify cap at 50K | P0 |
| INV-FTUI-PATH-VALIDATION | File paths constructed from `packet_id` cannot escape artifact root | Unit test: `packet_id = "../../etc/passwd"`; verify rejection | P1 |
| INV-FTUI-NO-SYMLINK-FOLLOW | Symlinks in artifact directory are either rejected or resolved+validated | Unit test: create symlink to `/tmp`; verify behavior | P1 |
| INV-FTUI-STALENESS-DISPLAY | Artifact file modification timestamps are displayed | Integration test: verify timestamp visible in panel | P1 |
| INV-FTUI-RESIZE-DEBOUNCE | Resize events are coalesced with >=50ms interval | Performance test: send 100 resize events in 100ms; verify <=3 relayouts | P1 |
| INV-FTUI-VIRTUAL-SCROLL | Lists exceeding threshold use virtual scrolling | Performance test: inject >10K events; verify render time <16ms | P1 |
| INV-FTUI-MIN-TERMINAL-GATE | Below 80x24, FTUI shows size message, not garbled output | Integration test: set terminal to 40x12; verify message | P1 |
| INV-FTUI-COLOR-FALLBACK | Unknown color depth defaults to 16-color, not true color | Unit test: unset `$COLORTERM`; verify 16-color palette | P2 |
| INV-FTUI-UNICODE-WIDTH | Layout uses `unicode-width` for column width calculation | Unit test: CJK string in table cell; verify alignment | P2 |
| INV-FTUI-NO-ENV-LEAK | FTUI never reads environment variables outside the allowed list | Code review; grep for `std::env::var` outside config module | P2 |
| INV-FTUI-CLIPBOARD-WARN | Clipboard copy of shell commands shows warning | Unit test: copy replay_command; verify warning rendered | P2 |
| INV-FTUI-BIDI-STRIP | BiDi control characters are stripped from display strings | Unit test: inject U+202E in artifact string; verify stripped | P2 |
| INV-FTUI-JSONL-SKIP | Malformed JSONL lines are skipped, not fatal | Unit test: inject invalid JSON line; verify skip count | P2 |
| INV-FTUI-RENDER-TIMEOUT | Render frames exceeding 100ms are aborted | Performance test: inject pathological layout; verify timeout | P3 |
| INV-FTUI-NO-PERSISTENT-LOGS | FTUI writes no log files by default | Integration test: run FTUI; verify no new files created | P3 |
| INV-FTUI-PASTE-BRACKET | Bracketed paste mode is enabled when supported | Integration test: send bracketed paste sequence; verify handling | P3 |
| INV-FTUI-CARGO-DENY | `cargo-deny` passes with no advisories or bans | CI gate: `cargo deny check` | P2 |
| INV-FTUI-FEATURE-ISOLATED | Asupersync panels compile-gated with `#[cfg(feature = "asupersync")]` | Compiler: build without asupersync feature | P1 |

---

## Drift Gates

These conditions must hold for this threat model to remain valid. If any gate
is violated, this document must be re-evaluated.

1. `#![forbid(unsafe_code)]` remains on the FTUI crate (memory safety boundary).
2. FTUI remains a read-only consumer of artifact files (no write operations).
3. FTUI remains a local-only tool (no network access, no remote data sources).
4. FTUI remains single-user (no authentication, no multi-tenant access).
5. The TUI framework is ratatui + crossterm (or a compatible alternative).
6. `drift_history.jsonl` remains the only append-only JSONL data source.
7. Artifact files remain JSON/JSONL with UTF-8 encoding produced by serde.
8. The conformance harness remains the sole artifact producer.
9. `ForensicEventKind` has exactly 10 variants (exhaustive match guards).
10. `CiGate` has exactly 9 variants (exhaustive match guards).

If FTUI gains network access (e.g., remote artifact sync), authentication,
or write capabilities, this threat model must be extended to cover the
expanded attack surface.

---

## Appendix B: Threat-to-Recommendation Traceability

| Threat ID | Recommendation | Priority |
|---|---|---|
| FTS-1.1 | **R-01 (display string sanitization)** | **P0** |
| FTS-1.2 | R-12 (cargo-deny for dependency audit) | P2 |
| FTS-1.3 | R-01 (display string sanitization, BiDi stripping) | P0 |
| FTS-1.4 | R-10 (unicode-width for layout) | P2 |
| FTS-2.1 | R-04 (size limits on ingestion) | P0 |
| FTS-2.2 | (Handled by design: skip trailing incomplete lines) | -- |
| FTS-2.3 | (Mitigated by `#[serde(default)]` policy; ongoing vigilance) | -- |
| FTS-2.4 | **R-04 (collection caps)** | **P0** |
| FTS-2.5 | (Handled by 4-decimal fixed precision display) | -- |
| FTS-3.1 | **R-05 (path validation)** | **P1** |
| FTS-3.2 | **R-05 (packet_id sanitization)** | **P1** |
| FTS-3.3 | (Accepted as inherent; TOCTOU risk is minimal for read-only tool) | -- |
| FTS-3.4 | R-04 (bounded cache; no persistent logs by default) | P0 |
| FTS-4.1 | R-13 (bracketed paste mode) | P3 |
| FTS-4.2 | **R-07 (resize event debouncing)** | **P1** |
| FTS-4.3 | (Mitigated by keyboard event priority in event loop) | -- |
| FTS-4.4 | R-09 (clipboard copy warning) | P2 |
| FTS-5.1 | **R-06 (staleness indicators)** | **P1** |
| FTS-5.2 | R-06 (partial write indicators) | P1 |
| FTS-5.3 | R-14 (world-writable directory warning) | P3 |
| FTS-5.4 | (Mitigated by `run_ts_unix_ms` consistency check) | -- |
| FTS-6.1 | R-12 (cargo-deny + cargo audit) | P2 |
| FTS-6.2 | R-12 (cargo-deny + cargo audit) | P2 |
| FTS-6.3 | R-12 (cargo-deny + cargo audit) | P2 |
| FTS-6.4 | R-12 (build script audit) | P2 |

---

## Appendix C: Fail-Closed Decision Tree

```
FTUI Startup
  |
  +-- Can open artifact directory?
  |     NO --> Display "artifact directory not found"; degrade to L1
  |     YES --> Continue
  |
  +-- Terminal size >= 80x24?
  |     NO --> Display minimum-size message; block rendering (L3)
  |     YES --> Continue
  |
  +-- Color depth detected?
  |     NO --> Default to 16-color (fail-closed)
  |     YES --> Use detected depth
  |
  +-- Artifact files parseable?
  |     ALL YES --> Full rendering (L0)
  |     SOME NO --> Degrade affected panels (L1)
  |     ALL NO  --> Display "no valid data"; degrade to L1
  |
  +-- drift_history.jsonl exists?
  |     NO --> Trend panel shows "No drift history available"
  |     YES --> Parse line by line; skip malformed; count skips
  |
  +-- Data within size limits?
  |     YES --> Full load
  |     OVER SOFT --> Warn in footer; enable virtual scrolling
  |     OVER HARD --> Truncate; show "truncated" indicator
  |
  +-- Render within frame budget?
        YES --> Display at target FPS
        NO  --> Drop frame; simplify rendering; degrade to L2 after 3 timeouts
```

---

## Changelog

- **bd-2gi.28.3** (2026-02-14): Initial FRANKENTUI security + compatibility threat model. Enumerates 6 threat surfaces (terminal rendering, data ingestion, file system, user input, cross-process communication, supply chain) with 25 specific threats in a detailed threat matrix. Defines compatibility envelope across 12 terminal emulators, 4 color depth levels, 5 operating systems, and minimum terminal size requirements. Applies fail-closed doctrine across 5 failure categories (malformed data, terminal too small, missing files, unavailable harness, graceful degradation). Specifies input validation model with sanitization pipeline, size limits, and schema checks. Analyzes dependency chain risks for ratatui, crossterm, and transitive dependencies. Covers information disclosure (8 data categories), availability threats (render hangs, large data, event blocking, crash recovery), and provides 15 prioritized recommendations (4 P0, 4 P1, 4 P2, 3 P3). Appendices include 23 machine-checkable invariants, threat-to-recommendation traceability matrix, and fail-closed decision tree.
