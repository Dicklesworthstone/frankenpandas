#!/usr/bin/env python3
"""Perf-ledger and audit-compile preflight.

Campaign `perf-campaign-20260725`, Meta-Lever #1 institutionalization. Modelled on
frankensqlite's `sql_pipeline_candidate_preflight` (exit 2 = BLOCKED), which is why that
repo sits at a 1.7% void rate after four months while repos that audited once and stopped
sit at 25-91%. Ledger integrity DECAYS; this is the ratchet.

Modes:

1. `--candidate "<lever>" --surface "<target>"` — grep the ledger BEFORE source mutation.
   A prior row on the surface blocks with its retry predicate printed.

2. `--check-new-rows` (default; pre-commit mode) — compare complete entries in
   `docs/NEGATIVE_EVIDENCE.md` and `docs/LEDGER_RESURRECTION.md`, including modified
   entries and the legacy `## Results` table. REJECT requires an exact numeric A/A
   marker or counted mechanism. Every kept entry requires exact result-class,
   executing-ELF, A/A, median-CI, and CV-role markers. `incumbent-win` additionally
   requires a pinned live pandas arm in the same invocation; fp-before/fp-after is
   `maintenance-self-speedup`, never campaign output.

3. `--check-disk` — print `df -h /data` and block below 120 GiB free.

4. `--run-compile ...` — run one compile command only after the disk guard passes. It
   requires one explicit/shared `CARGO_TARGET_DIR`, preventing per-snapshot target growth.

NOTE: "bit-identical" / "byte-identical" / "0 diffs" are parity proofs, not counted
mechanism refutations, and are deliberately not accepted.

Exit codes: 0 = pass, 2 = BLOCKED, 1 = usage/IO error.
"""

from __future__ import annotations

import argparse
import os
import re
import shutil
import subprocess
import sys
from collections import Counter
from dataclasses import dataclass
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
DEFAULT_DISK_PATH = Path("/data")
DEFAULT_MIN_FREE_GIB = 120.0
DEFAULT_GIT_TIMEOUT_SECONDS = 30.0
DEFAULT_DF_TIMEOUT_SECONDS = 10.0
DEFAULT_COMPILE_TIMEOUT_SECONDS = 3600.0


@dataclass(frozen=True)
class LedgerSpec:
    relative_path: str
    legacy_results_table: bool = False


@dataclass(frozen=True)
class LedgerEntry:
    ledger: str
    schema: str
    line_no: int
    title: str
    body: str
    verdict_text: str

    def fingerprint(self) -> tuple[str, str, str, str]:
        return (self.schema, self.title, self.body, self.verdict_text)


@dataclass(frozen=True)
class PolicyViolation:
    code: str
    entry: LedgerEntry
    detail: str


LEDGER_SPECS = (
    LedgerSpec("docs/NEGATIVE_EVIDENCE.md", legacy_results_table=True),
    LedgerSpec("docs/LEDGER_RESURRECTION.md"),
)
NEGATIVE_LEDGER = REPO / LEDGER_SPECS[0].relative_path

EXPLICIT_REJECT_MARK = re.compile(
    r"\bREJECT(?:ED|S)?\b|\bREVERT(?:ED)?\b|\bNOSHIP\b|\bNO-SHIP\b|"
    r"zero-gain|~0-gain",
    re.IGNORECASE,
)
DIRECTION_MARK = re.compile(
    r"\b(?P<negative>SLOWER|LOSS(?:ES)?|REGRESSION(?:S)?)\b|"
    r"\b(?P<positive>WIN|FASTER|FIXED|KEEP|SHIPPED)\b",
    re.IGNORECASE,
)
NEGATED_DIRECTION_MARK = re.compile(
    r"\b(?:not\s+(?:a\s+)?|no(?:\s+[A-Za-z-]+){0,3}\s+|phantom\s+)"
    r"(?:slower|loss(?:es)?|regression(?:s)?)\b",
    re.IGNORECASE,
)
KEEP_MARK = re.compile(r"\bKEEP\b|\bSHIPPED\b|\bWIN\b", re.IGNORECASE)
ZERO_VERDICT_COUNT = re.compile(
    r"\b(?:0|zero|no)\s+(?:new\s+)?"
    r"(?:REJECT(?:ED|S)?|NOSHIP|NO-SHIP|SLOWER|LOSS(?:ES)?|"
    r"REGRESSION(?:S)?|KEEP(?:S)?|SHIPPED|WIN(?:S)?)\b",
    re.IGNORECASE,
)
NEGATED_NULL_CTRL = re.compile(
    r"(?:\bno\b|\bwithout\b|\bmissing\b|\babsent\b|\bnever (?:ran|recorded)\b|"
    r"\bunavailable\b|\bnot (?:run|recorded|measured)\b).{0,40}"
    r"(?:A/A|null control|null-control|null arm|null floor)|"
    r"(?:A/A|null control|null-control|null arm|null floor).{0,40}"
    r"(?:\bmissing\b|\babsent\b|\bunavailable\b|\bN/?A\b|"
    r"\bnot (?:run|recorded|measured)\b)",
    re.IGNORECASE,
)
COUNTED_METRIC = re.compile(
    r"\b(?:instructions?|cycles|syscalls?|allocations?|mallocs?|"
    r"(?:page|branch|cache)[- ](?:faults?|miss(?:es)?)|IPC)\b",
    re.IGNORECASE,
)
NUMBER = re.compile(r"(?<![A-Za-z0-9])\d[\d_,]*(?:\.\d+)?")
EXECUTING_ELF = re.compile(
    r"bench_elf_sha256=([0-9a-f]{64})\s*"
    r"\(\s*([1-9][\d,]*)\s+bytes\s*\)\s+(/[^\s`]+)"
)
MAINTENANCE_COMPETITIVE = re.compile(
    r"(?:\d+(?:\.\d+)?\s*[x×].{0,100}"
    r"(?:faster|slower|vs\.?|versus|against).{0,100}pandas)|"
    r"(?:pandas.{0,100}\d+(?:\.\d+)?\s*[x×])|"
    r"(?:\b(?:beats?|outperforms?|faster than|slower than)\s+pandas\b)",
    re.IGNORECASE | re.DOTALL,
)
RETRY_MARK = re.compile(
    r"retry predicate|retry condition|retry only|re-?open only|do not retry|"
    r"do not attempt|closing [^.]{0,160} needs|resume:",
    re.IGNORECASE,
)

CLASS_MARKER = "Campaign result class"
ELF_MARKER = "Executing ELF SHA-256 (self-reported by process)"
AA_MARKER = "A/A null control (same invocation)"
MEDIAN_MARKER = "Median-CI decision"
CV_MARKER = "CV role"
INCUMBENT_MARKER = "Legacy incumbent arm (same invocation)"
MECHANISM_MARKER = "Counted mechanism"


def _git(*args: str) -> str:
    try:
        result = subprocess.run(
            ["git", *args],
            cwd=REPO,
            capture_output=True,
            text=True,
            check=False,
            timeout=DEFAULT_GIT_TIMEOUT_SECONDS,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise RuntimeError(f"unable to run git {' '.join(args)}: {error}") from error
    if result.returncode:
        detail = result.stderr.strip() or f"exit {result.returncode}"
        raise RuntimeError(f"git {' '.join(args)} failed: {detail}")
    return result.stdout


def canonical_entries(text: str, spec: LedgerSpec) -> list[LedgerEntry]:
    """Parse canonical `###` entries without absorbing the next peer/parent entry."""
    lines = text.splitlines()
    entries: list[LedgerEntry] = []
    index = 0
    while index < len(lines):
        line = lines[index]
        if not line.startswith("### "):
            index += 1
            continue
        line_no = index + 1
        title = line[4:].strip()
        index += 1
        body: list[str] = []
        while index < len(lines):
            current = lines[index]
            if current.startswith(("### ", "## ")):
                break
            body.append(current)
            index += 1
        entries.append(
            LedgerEntry(
                ledger=spec.relative_path,
                schema="canonical-section",
                line_no=line_no,
                title=title,
                body="\n".join(body).rstrip(),
                verdict_text=title,
            )
        )
    return entries


def _split_table_row(line: str) -> list[str] | None:
    stripped = line.strip()
    if not stripped.startswith("|") or not stripped.endswith("|"):
        return None
    return [cell.strip() for cell in stripped[1:-1].split("|")]


def legacy_results_entries(text: str, spec: LedgerSpec) -> list[LedgerEntry]:
    """Parse one verdict per markdown row in NEGATIVE_EVIDENCE's legacy table."""
    if not spec.legacy_results_table:
        return []
    lines = text.splitlines()
    results_start = next(
        (index for index, line in enumerate(lines) if line.strip() == "## Results"),
        None,
    )
    if results_start is None:
        return []

    header: list[str] | None = None
    entries: list[LedgerEntry] = []
    for index in range(results_start + 1, len(lines)):
        line = lines[index]
        if line.startswith("#"):
            break
        cells = _split_table_row(line)
        if not cells:
            if entries:
                break
            continue
        if all(re.fullmatch(r":?-{3,}:?", cell) for cell in cells):
            continue
        if header is None:
            header = cells
            continue
        if len(cells) != len(header):
            continue
        expanded = [re.sub(r"(?i)<br\s*/?>", "\n", cell) for cell in cells]
        body = "\n".join(f"{name}: {value}" for name, value in zip(header, expanded))
        entries.append(
            LedgerEntry(
                ledger=spec.relative_path,
                schema="legacy-results-row",
                line_no=index + 1,
                title=expanded[0],
                body=body,
                verdict_text=expanded[-1],
            )
        )
    return entries


def parse_ledger(text: str, spec: LedgerSpec) -> list[LedgerEntry]:
    return canonical_entries(text, spec) + legacy_results_entries(text, spec)


def changed_entries_between(
    base_text: str, current_text: str, spec: LedgerSpec
) -> list[LedgerEntry]:
    """Return added or modified entries, using full-entry multiset comparison."""
    base_counts = Counter(entry.fingerprint() for entry in parse_ledger(base_text, spec))
    changed: list[LedgerEntry] = []
    for entry in parse_ledger(current_text, spec):
        fingerprint = entry.fingerprint()
        if base_counts[fingerprint]:
            base_counts[fingerprint] -= 1
        else:
            changed.append(entry)
    return changed


def changed_ledger_entries(base: str, *, cached: bool) -> list[LedgerEntry]:
    changed: list[LedgerEntry] = []
    for spec in LEDGER_SPECS:
        base_text = _git("show", f"{base}:{spec.relative_path}")
        if cached:
            current_text = _git("show", f":{spec.relative_path}")
        else:
            current_text = (REPO / spec.relative_path).read_text(errors="replace")
        changed.extend(changed_entries_between(base_text, current_text, spec))
    return changed


def marker_values(body: str, label: str) -> list[str]:
    """Return paragraphs belonging to exact line-anchored machine markers."""
    prefix = f"**{label}:**"
    lines = body.splitlines()
    values: list[str] = []
    for index, line in enumerate(lines):
        stripped = line.strip()
        if not stripped.startswith(prefix):
            continue
        paragraph = [stripped[len(prefix) :].strip()]
        cursor = index + 1
        while cursor < len(lines):
            following = lines[cursor].strip()
            if not following or following.startswith(("**", "##", "|")):
                break
            paragraph.append(following)
            cursor += 1
        values.append(" ".join(part for part in paragraph if part))
    return values


def _flat(value: str) -> str:
    return re.sub(r"\s+", " ", value.replace("`", "")).strip()


def has_numeric_aa(body: str) -> bool:
    values = marker_values(body, AA_MARKER)
    if len(values) != 1:
        return False
    value = _flat(values[0])
    return bool(
        NUMBER.search(value)
        and re.search(r"\b(?:median|ratio|CI|interval)\b", value, re.IGNORECASE)
        and not re.search(
            r"\b0\s+(?:pairs|rounds|samples|iterations)\b",
            value,
            re.IGNORECASE,
        )
        and not NEGATED_NULL_CTRL.search(value)
    )


def has_counted_mechanism(body: str) -> bool:
    values = marker_values(body, MECHANISM_MARKER)
    if len(values) != 1:
        return False
    value = _flat(values[0])
    numbers = NUMBER.findall(value)
    if not COUNTED_METRIC.search(value) or not numbers:
        return False
    if re.search(r"\b(?:unchanged|same|identical|zero|0)\b", value, re.IGNORECASE):
        return True
    return bool(
        len(numbers) >= 2
        and re.search(
            r"\b(?:required|predeclared|pregate|threshold|limit)\b",
            value,
            re.IGNORECASE,
        )
        and re.search(
            r"\b(?:failed?|missed?|below|short|did not (?:meet|clear))\b",
            value,
            re.IGNORECASE,
        )
    )


def result_class(body: str) -> str | None:
    values = marker_values(body, CLASS_MARKER)
    if len(values) != 1:
        return None
    match = re.fullmatch(
        r"`?(maintenance-self-speedup|incumbent-win)`?\.?",
        values[0].strip(),
    )
    return match.group(1) if match else None


def has_executing_elf(body: str) -> bool:
    values = marker_values(body, ELF_MARKER)
    return len(values) == 1 and bool(EXECUTING_ELF.search(_flat(values[0])))


def has_median_ci_decision(body: str) -> bool:
    values = marker_values(body, MEDIAN_MARKER)
    if len(values) != 1:
        return False
    value = _flat(values[0])
    return bool(
        NUMBER.search(value)
        and re.search(r"\b(?:median|CI)\b", value, re.IGNORECASE)
        and re.search(r"\beffect\b", value, re.IGNORECASE)
        and re.search(
            r"\b(?:required|threshold|interval|floor|cleared|inside)\b",
            value,
            re.IGNORECASE,
        )
    )


def valid_cv_role(body: str) -> bool:
    values = marker_values(body, CV_MARKER)
    if len(values) != 1:
        return False
    value = _flat(values[0])
    if re.search(
        r"(?:CV\s*(?:<|<=)\s*\d|CV.{0,40}\b(?:decid|admit|reject|gate(?:d)?)\b|"
        r"\bgate(?:d)?.{0,40}CV)",
        value,
        re.IGNORECASE,
    ):
        return False
    return bool(
        re.search(
            r"\b(?:no vote|provenance only|did not vote|not a gate|never votes?)\b",
            value,
            re.IGNORECASE,
        )
    )


def incumbent_contract_errors(body: str) -> list[str]:
    values = marker_values(body, INCUMBENT_MARKER)
    if len(values) != 1:
        return ["missing exact live-incumbent marker"]
    value = _flat(values[0])
    errors: list[str] = []
    if not re.search(r"(?:^|\s)name=pandas(?:\s|$)", value):
        errors.append("name=pandas")
    if not re.search(
        r"(?:^|\s)version=[0-9]+(?:\.[0-9]+)+(?:[A-Za-z0-9._+-]*)(?:\s|$)",
        value,
    ):
        errors.append("pinned pandas version")
    if not re.search(r"(?:^|\s)artifact_sha256=[0-9a-f]{64}(?:\s|$)", value):
        errors.append("lowercase pandas artifact SHA-256")
    invocation = re.search(
        r"(?:^|\s)invocation_id=([A-Za-z0-9][A-Za-z0-9._:-]*)(?:\s|$)",
        value,
    )
    if (
        invocation is None
        or len(invocation.group(1)) < 4
        or invocation.group(1).lower()
        in {
            "shared",
            "unknown",
            "none",
            "unavailable",
            "todo",
            "n",
            "na",
        }
    ):
        errors.append("non-placeholder shared invocation ID")
    ratio = re.search(r"(?:^|\s)measured_ratio=(\d+(?:\.\d+)?)x(?:\s|$)", value)
    if ratio is None:
        errors.append("measured incumbent ratio")
    elif float(ratio.group(1)) <= 1.0:
        errors.append("incumbent-win ratio must exceed 1.0x")
    aa_values = marker_values(body, AA_MARKER)
    if aa_values:
        aa_value = _flat(aa_values[0])
        if not (
            re.search(r"\b(?:FrankenPandas|FP)\b", aa_value, re.IGNORECASE)
            and re.search(r"\bpandas\b", aa_value, re.IGNORECASE)
        ):
            errors.append("A/A results for both FrankenPandas and pandas arms")
    return errors


def has_negative_verdict(text: str) -> bool:
    """Return the final decision, not every historical direction in a title.

    Ledger headings routinely say ``LOSS -> WIN`` or ``WIN, not a loss``.
    Treating the first negative word as the verdict makes the candidate
    preflight block unrelated, already-resolved work. Explicit reject/revert
    markers still dominate even when a heading also says the baseline wins.
    """
    verdict_text = ZERO_VERDICT_COUNT.sub("", text)
    verdict_text = NEGATED_DIRECTION_MARK.sub("", verdict_text)
    if EXPLICIT_REJECT_MARK.search(verdict_text):
        return True
    if re.match(r"\s*(?:fixed|resolved|closed)\b", verdict_text, re.IGNORECASE):
        return False
    final_direction: str | None = None
    for match in DIRECTION_MARK.finditer(verdict_text):
        final_direction = match.lastgroup
    return final_direction == "negative"


def verdict_flags(entry: LedgerEntry) -> tuple[bool, bool]:
    verdict_text = ZERO_VERDICT_COUNT.sub("", entry.verdict_text)
    positive = bool(KEEP_MARK.search(verdict_text) or marker_values(entry.body, CLASS_MARKER))
    negative = has_negative_verdict(verdict_text)
    return positive, negative


def validate_entries(entries: list[LedgerEntry]) -> list[PolicyViolation]:
    violations: list[PolicyViolation] = []
    for entry in entries:
        positive, negative = verdict_flags(entry)
        if negative and not (has_numeric_aa(entry.body) or has_counted_mechanism(entry.body)):
            violations.append(
                PolicyViolation(
                    "reject-basis",
                    entry,
                    "negative verdict lacks exact numeric A/A or counted-mechanism evidence",
                )
            )
        if not positive:
            continue

        classification = result_class(entry.body)
        if classification is None:
            violations.append(
                PolicyViolation(
                    "result-class",
                    entry,
                    "kept entry needs exactly one exact maintenance-self-speedup/incumbent-win marker",
                )
            )
        if not has_executing_elf(entry.body):
            violations.append(
                PolicyViolation(
                    "executing-elf",
                    entry,
                    "missing canonical in-process bench_elf_sha256=<sha> (<bytes> bytes) <absolute path>",
                )
            )
        if not has_numeric_aa(entry.body):
            violations.append(
                PolicyViolation("aa-null", entry, "missing exact numeric A/A null marker")
            )
        if not has_median_ci_decision(entry.body):
            violations.append(
                PolicyViolation(
                    "median-ci",
                    entry,
                    "missing numeric median-CI effect and decision threshold",
                )
            )
        if not valid_cv_role(entry.body):
            violations.append(
                PolicyViolation("cv-role", entry, "CV must be provenance-only and have no vote")
            )

        blob = f"{entry.title}\n{entry.body}"
        if classification == "maintenance-self-speedup":
            if re.search(
                r"\bWIN\b", entry.verdict_text, re.IGNORECASE
            ) or MAINTENANCE_COMPETITIVE.search(blob):
                violations.append(
                    PolicyViolation(
                        "maintenance-claim",
                        entry,
                        "maintenance-self-speedup may not be titled WIN or quote a pandas-facing ratio",
                    )
                )
        elif classification == "incumbent-win":
            for detail in incumbent_contract_errors(entry.body):
                violations.append(PolicyViolation("incumbent-contract", entry, detail))
    return violations


def check_new_rows(base: str, *, cached: bool) -> int:
    entries = changed_ledger_entries(base, cached=cached)
    source = "staged" if cached else "working-tree"
    if not entries:
        print(f"preflight: no added or modified {source} ledger entries vs {base} — OK")
        return 0

    violations = validate_entries(entries)
    if violations:
        print("preflight: BLOCKED — inadmissible added/modified ledger entry\n")
        for violation in violations:
            entry = violation.entry
            print(
                f"  ✗ {violation.code}: {entry.ledger}:{entry.line_no} "
                f"[{entry.schema}] {entry.title[:130]}\n"
                f"    {violation.detail}"
            )
        print(
            "\nREJECT requires an exact numeric A/A marker or exact counted mechanism. "
            "Kept rows require an exact class plus executing ELF, A/A, median-CI, and "
            "CV-role markers. incumbent-win also requires a pinned pandas artifact and "
            "shared invocation identity; maintenance-self-speedup is not campaign output."
        )
        return 2

    verdict_count = sum(any(verdict_flags(entry)) for entry in entries)
    print(
        f"preflight: {len(entries)} added/modified entries across both ledgers; "
        f"{verdict_count} verdict-bearing; policy contract satisfied — OK"
    )
    return 0


def ledger_entries() -> list[LedgerEntry]:
    spec = LEDGER_SPECS[0]
    return parse_ledger(NEGATIVE_LEDGER.read_text(errors="replace"), spec)


def terms_for(value: str) -> list[str]:
    return [term for term in re.split(r"[\s,/:]+", value.strip()) if len(term) > 2]


def contains_surface_term(blob: str, term: str) -> bool:
    """Match a surface token without treating `str` as part of `instrumented`."""
    return bool(
        re.search(
            rf"(?<![A-Za-z0-9_]){re.escape(term)}(?![A-Za-z0-9_])",
            blob,
            re.IGNORECASE,
        )
    )


def retry_predicate(body: str) -> str:
    paragraphs = re.split(r"\n\s*\n", body)
    for paragraph in paragraphs:
        match = RETRY_MARK.search(paragraph)
        if match:
            predicate = paragraph[match.start() :]
            return re.sub(r"\s+", " ", predicate).strip()[:700]
    for line in body.splitlines():
        match = RETRY_MARK.search(line)
        if match:
            return re.sub(r"\s+", " ", line[match.start() :]).strip()[:700]
    return "NOT_RECORDED"


def entry_rejects_surface(entry: LedgerEntry, surface_terms: list[str]) -> bool:
    """Require the surface and negative decision to occur in one decision unit.

    A negative entry can mention many sibling controls in its body. Those
    siblings are not rejected surfaces merely because they appear somewhere
    in the same section. Prefer the title/verdict, then bounded body lines and
    paragraphs that carry both the requested surface and a negative decision.
    """
    if all(contains_surface_term(entry.title, term) for term in surface_terms):
        return has_negative_verdict(entry.verdict_text)

    lines = [line.strip() for line in entry.body.splitlines() if line.strip()]
    decision_units = [
        clause.strip()
        for line in lines
        for sentence in re.split(
            r"(?<=[.!?])\s+(?=[A-Z*`])",
            line,
        )
        for clause in re.split(r"[|,;]+", sentence)
        if clause.strip()
    ]
    for unit in decision_units:
        if all(
            contains_surface_term(unit, term) for term in surface_terms
        ) and has_negative_verdict(unit):
            return True
    return False


def check_candidate(candidate: str, surface: str | None) -> int:
    if not NEGATIVE_LEDGER.is_file():
        print(f"preflight: ledger not found at {NEGATIVE_LEDGER}", file=sys.stderr)
        return 1

    candidate_terms = terms_for(candidate)
    surface_terms = terms_for(surface or candidate)
    if not candidate_terms or not surface_terms:
        print(
            "preflight: --candidate/--surface need at least one term of 3+ chars",
            file=sys.stderr,
        )
        return 1

    hits: list[tuple[int, str, str]] = []
    for entry in ledger_entries():
        if entry_rejects_surface(entry, surface_terms):
            hits.append((entry.line_no, entry.title, retry_predicate(entry.body)))

    if hits:
        print(
            "preflight: BLOCKED — "
            f"{len(hits)} prior REJECT row(s) match target_surface={surface or candidate!r}"
        )
        print(f"proposed_lever={candidate!r}\n")
        for line_no, title, predicate in hits[:10]:
            print(
                f"match decision=REJECT ledger=docs/NEGATIVE_EVIDENCE.md:{line_no}\n"
                f"  title={title[:180]}\n"
                f"  retry_condition={predicate}\n"
            )
        print(
            "Do not mutate source until a printed retry condition is explicitly satisfied. "
            "A missing predicate is itself a ledger defect to repair first."
        )
        return 2

    print(
        "preflight: CLEAR — no prior REJECT matches "
        f"target_surface={surface or candidate!r}; proposed_lever={candidate!r}"
    )
    return 0


def check_disk(path: Path, min_free_gib: float) -> int:
    try:
        report = subprocess.run(
            ["df", "-h", str(path)],
            capture_output=True,
            text=True,
            check=False,
            timeout=DEFAULT_DF_TIMEOUT_SECONDS,
        )
        if report.returncode:
            detail = report.stderr.strip() or f"exit {report.returncode}"
            print(f"disk_guard: df failed for {path}: {detail}", file=sys.stderr)
            return 1
        if report.stdout:
            print(report.stdout.rstrip(), flush=True)
        usage = shutil.disk_usage(path)
    except (OSError, subprocess.TimeoutExpired) as error:
        print(f"disk_guard: unable to inspect {path}: {error}", file=sys.stderr)
        return 1

    free_gib = usage.free / (1024**3)
    if free_gib < min_free_gib:
        print(
            f"disk_guard: BLOCKED — {free_gib:.1f} GiB free at {path}, "
            f"minimum is {min_free_gib:.1f} GiB",
            flush=True,
        )
        return 2
    print(
        f"disk_guard: CLEAR — {free_gib:.1f} GiB free at {path}, "
        f"minimum is {min_free_gib:.1f} GiB",
        flush=True,
    )
    return 0


def run_compile(
    command: list[str],
    *,
    target_dir: str | None,
    disk_path: Path,
    min_free_gib: float,
    timeout_seconds: float,
) -> int:
    if command and command[0] == "--":
        command = command[1:]
    if not command:
        print("preflight: --run-compile requires a command", file=sys.stderr)
        return 1
    tool = Path(command[0]).name
    if tool not in {"cargo", "rustc"}:
        print("preflight: --run-compile accepts cargo or rustc only", file=sys.stderr)
        return 1
    if any("\0" in argument for argument in command):
        print("preflight: compile arguments may not contain NUL bytes", file=sys.stderr)
        return 1
    if timeout_seconds <= 0:
        print("preflight: --compile-timeout-seconds must be positive", file=sys.stderr)
        return 1
    executable = shutil.which(tool)
    if executable is None:
        print(f"preflight: unable to locate {tool} in PATH", file=sys.stderr)
        return 1
    disk_rc = check_disk(disk_path, min_free_gib)
    if disk_rc:
        return disk_rc

    env = os.environ.copy()
    selected_target = target_dir or env.get("CARGO_TARGET_DIR")
    if not selected_target:
        print(
            "preflight: BLOCKED — guarded compiles require one explicit/shared "
            "--target-dir or CARGO_TARGET_DIR",
            file=sys.stderr,
        )
        return 2
    env["CARGO_TARGET_DIR"] = str(Path(selected_target).resolve())
    print(f"compile_guard: target_dir={env['CARGO_TARGET_DIR']}", flush=True)
    print(f"compile_guard: timeout_seconds={timeout_seconds:g}", flush=True)
    safe_command = [executable, *command[1:]]
    try:
        return subprocess.run(
            safe_command,
            cwd=REPO,
            env=env,
            check=False,
            shell=False,
            timeout=timeout_seconds,
        ).returncode
    except subprocess.TimeoutExpired:
        print(
            f"compile_guard: BLOCKED — compile exceeded {timeout_seconds:g}s",
            file=sys.stderr,
        )
        return 2
    except OSError as error:
        print(f"compile_guard: unable to execute {tool}: {error}", file=sys.stderr)
        return 1


def self_test() -> int:
    sha = "a" * 64
    upper_sha = "A" * 64
    ledger = "docs/NEGATIVE_EVIDENCE.md"

    def entry(title: str, body: str, *, schema: str = "canonical-section") -> LedgerEntry:
        return LedgerEntry(ledger, schema, 1, title, body, title)

    elf = (
        f"**{ELF_MARKER}:** "
        f"`bench_elf_sha256={sha} (123 bytes) /tmp/bench`."
    )
    aa = (
        f"**{AA_MARKER}:** 25 pairs; median ratio 1.001x; "
        "95% CI [0.990, 1.010]."
    )
    both_aa = (
        f"**{AA_MARKER}:** 25 pairs; FrankenPandas median ratio 1.001x "
        "CI [0.990, 1.010]; pandas median ratio 0.999x CI [0.985, 1.015]."
    )
    median_ci = (
        f"**{MEDIAN_MARKER}:** median effect 1.200x cleared required "
        "log-effect threshold 0.040."
    )
    cv = f"**{CV_MARKER}:** provenance only; CV had no vote."
    maintenance_class = f"**{CLASS_MARKER}:** `maintenance-self-speedup`."
    incumbent_class = f"**{CLASS_MARKER}:** `incumbent-win`."
    incumbent = (
        f"**{INCUMBENT_MARKER}:** name=pandas version=2.2.3 "
        f"artifact_sha256={sha} invocation_id=run-20260727-a "
        "measured_ratio=1.200x"
    )
    valid_maintenance = f"{maintenance_class}\n\n{elf}\n\n{aa}\n\n{median_ci}\n\n{cv}"
    valid_incumbent = (
        f"{incumbent_class}\n\n{elf}\n\n{both_aa}\n\n{median_ci}\n\n{cv}\n\n"
        f"{incumbent}"
    )

    checks = 0
    failed: list[str] = []

    def expect_block(name: str, candidate: LedgerEntry, code: str) -> None:
        nonlocal checks
        checks += 1
        codes = {violation.code for violation in validate_entries([candidate])}
        if code not in codes:
            failed.append(f"{name}: expected {code}, got {sorted(codes)}")

    def expect_pass(name: str, candidate: LedgerEntry) -> None:
        nonlocal checks
        checks += 1
        violations = validate_entries([candidate])
        if violations:
            failed.append(
                f"{name}: unexpected {[(item.code, item.detail) for item in violations]}"
            )

    expect_block(
        "reject_without_basis",
        entry("REJECT foo", "bit-identical, 0 diffs"),
        "reject-basis",
    )
    expect_block(
        "reject_zero_self_only",
        entry("REJECT foo", "target frame 0.000% self"),
        "reject-basis",
    )
    expect_block(
        "reject_negated_exact_null",
        entry(
            "REJECT foo",
            f"**{AA_MARKER}:** no A/A null control was recorded; 0 pairs.",
        ),
        "reject-basis",
    )
    expect_block(
        "reject_bare_null_prose",
        entry("REJECT foo", "A/A null control 1.001x"),
        "reject-basis",
    )
    expect_block(
        "reject_perf_stat_only",
        entry("REJECT foo", "perf stat was attempted"),
        "reject-basis",
    )
    expect_block(
        "reject_model_incident_real_loss_without_basis",
        entry(
            "uza04.218 matrix: ONE real loss (df_groupby_2strkey_sum @10k 0.86x)",
            "Four p50 ratios agree; output is bit-identical.",
        ),
        "reject-basis",
    )
    expect_pass("reject_with_exact_null", entry("REJECT foo", aa))
    expect_pass(
        "slower_with_exact_null",
        entry("df_groupby_2strkey_sum is SLOWER", aa),
    )
    expect_pass(
        "reject_with_count",
        entry(
            "REJECT foo",
            f"**{MECHANISM_MARKER}:** instructions measured 100000 versus "
            "100005, unchanged within 0.01%.",
        ),
    )
    expect_pass(
        "reject_with_numeric_counted_pregate",
        entry(
            "REJECT packed kernel",
            f"**{MECHANISM_MARKER}:** cycles measured 1191365 versus "
            "predeclared threshold 1250000; failed the counted pregate.",
        ),
    )
    expect_block(
        "reject_with_vague_counted_pregate",
        entry(
            "REJECT packed kernel",
            f"**{MECHANISM_MARKER}:** cycles missed the target.",
        ),
        "reject-basis",
    )
    expect_block("keep_without_class", entry("KEEP foo", elf), "result-class")
    expect_block(
        "keep_shell_sha",
        entry("KEEP foo", valid_maintenance.replace(elf, f"shell sha256 {sha}")),
        "executing-elf",
    )
    expect_block(
        "keep_unavailable_elf",
        entry(
            "KEEP foo",
            valid_maintenance.replace(
                elf,
                f"**{ELF_MARKER}:** bench_elf_sha256=unavailable; shell sha256 {sha}",
            ),
        ),
        "executing-elf",
    )
    expect_block(
        "keep_uppercase_elf",
        entry(
            "KEEP foo",
            valid_maintenance.replace(sha, upper_sha, 1),
        ),
        "executing-elf",
    )
    expect_block(
        "maintenance_missing_aa",
        entry("KEEP foo", valid_maintenance.replace(aa, "")),
        "aa-null",
    )
    expect_block(
        "maintenance_missing_median",
        entry("KEEP foo", valid_maintenance.replace(median_ci, "")),
        "median-ci",
    )
    expect_block(
        "maintenance_cv_gate",
        entry(
            "KEEP foo",
            valid_maintenance.replace(cv, f"**{CV_MARKER}:** CV < 5% gated admission."),
        ),
        "cv-role",
    )
    expect_block(
        "maintenance_win_title",
        entry("WIN foo", valid_maintenance),
        "maintenance-claim",
    )
    expect_block(
        "maintenance_pandas_ratio",
        entry("KEEP foo", f"{valid_maintenance}\n\n2.0x faster than pandas."),
        "maintenance-claim",
    )
    expect_pass("complete_maintenance", entry("KEEP foo", valid_maintenance))
    expect_block(
        "incumbent_bare_same_invocation",
        entry(
            "WIN foo",
            f"{incumbent_class}\n\n{elf}\n\n{both_aa}\n\n{median_ci}\n\n{cv}\n\n"
            "same invocation",
        ),
        "incumbent-contract",
    )
    for field, replacement in [
        ("name", "name=other"),
        ("version", "version=unknown"),
        ("artifact", "artifact_sha256=unavailable"),
        ("invocation", "invocation_id=shared"),
        ("ratio", "measured_ratio=unknown"),
    ]:
        bad = incumbent
        if field == "name":
            bad = bad.replace("name=pandas", replacement)
        elif field == "version":
            bad = bad.replace("version=2.2.3", replacement)
        elif field == "artifact":
            bad = bad.replace(f"artifact_sha256={sha}", replacement)
        elif field == "invocation":
            bad = bad.replace("invocation_id=run-20260727-a", replacement)
        else:
            bad = bad.replace("measured_ratio=1.200x", replacement)
        expect_block(
            f"incumbent_missing_{field}",
            entry(
                "WIN foo",
                f"{incumbent_class}\n\n{elf}\n\n{both_aa}\n\n{median_ci}\n\n{cv}"
                f"\n\n{bad}",
            ),
            "incumbent-contract",
        )
    expect_block(
        "incumbent_uppercase_artifact_sha",
        entry(
            "WIN foo",
            valid_incumbent.replace(
                f"artifact_sha256={sha}", f"artifact_sha256={upper_sha}"
            ),
        ),
        "incumbent-contract",
    )
    expect_block(
        "incumbent_ratio_below_one",
        entry(
            "WIN foo",
            valid_incumbent.replace("measured_ratio=1.200x", "measured_ratio=0.990x"),
        ),
        "incumbent-contract",
    )
    expect_block(
        "incumbent_one_arm_aa",
        entry("WIN foo", valid_incumbent.replace(both_aa, aa)),
        "incumbent-contract",
    )
    expect_pass("complete_incumbent", entry("WIN foo", valid_incumbent))
    expect_pass(
        "summary_zero_rejects_is_not_a_reject",
        entry("Integrity summary: 6 commits, 0 REJECTS", "No performance row."),
    )
    expect_pass(
        "summary_no_new_keeps_is_not_a_keep",
        entry("Audit summary: no new KEEPs", "No performance row."),
    )
    expect_pass(
        "summary_zero_losses_is_not_a_reject",
        entry("Integrity summary: zero losses", "No performance row."),
    )

    def expect_negative_direction(name: str, text: str, expected: bool) -> None:
        nonlocal checks
        checks += 1
        actual = has_negative_verdict(text)
        if actual != expected:
            failed.append(f"{name}: expected negative={expected}, got {actual}")

    expect_negative_direction(
        "resolved_not_a_loss",
        "explode is a WIN (43.8x), not a loss",
        False,
    )
    expect_negative_direction(
        "resolved_loss_to_win",
        "candidate 0.62x LOSS -> 1.49x WIN",
        False,
    )
    expect_negative_direction(
        "fixed_loss_heading",
        "Fixed: concat Int64 construction (24x loss)",
        False,
    )
    expect_negative_direction(
        "remaining_loss",
        "candidate remains a 0.62x LOSS",
        True,
    )
    expect_negative_direction(
        "explicit_reject_dominates_baseline_win",
        "candidate REJECT; baseline remains a WIN",
        True,
    )

    checks += 1
    sibling_entry = entry(
        "df.abs was the real loss",
        "| join_outer 0.71x | 2.29x WIN after clean remeasurement |",
    )
    if entry_rejects_surface(sibling_entry, ["join_outer"]):
        failed.append("sibling_surface_win: unrelated control was treated as rejected")

    checks += 1
    cross_sentence_entry = entry(
        "allocator adoption",
        (
            "Neutral controls include str_value_counts +2.5% (not release proof). "
            "Apparent regressions were rerun and rejected. Verdict: KEEP."
        ),
    )
    if entry_rejects_surface(cross_sentence_entry, ["str_value_counts"]):
        failed.append(
            "cross_sentence_surface: later negative sentence contaminated neutral surface"
        )

    checks += 1
    body_reject_entry = entry(
        "frontier survey",
        "The ewm_mean candidate is SLOWER and remains below parity.",
    )
    if not entry_rejects_surface(body_reject_entry, ["ewm_mean"]):
        failed.append("body_surface_reject: decision-local body reject was missed")

    negative_spec = LEDGER_SPECS[0]
    resurrection_spec = LEDGER_SPECS[1]

    def canonical_doc(title: str, body: str, suffix: str = "") -> str:
        return f"# Ledger\n\n### {title}\n\n{body}\n{suffix}"

    def legacy_doc(title: str, evidence: str, verdict: str) -> str:
        return (
            "# Ledger\n\n## Results\n\n"
            "| Lever | Workload | verdict |\n"
            "|---|---|---|\n"
            f"| {title} | {evidence} | {verdict} |\n\n"
            "## Appendix\n"
        )

    def expect_boundary(
        name: str,
        spec: LedgerSpec,
        base_text: str,
        current_text: str,
        *,
        blocked: bool,
    ) -> None:
        nonlocal checks
        checks += 1
        changed = changed_entries_between(base_text, current_text, spec)
        violations = validate_entries(changed)
        if blocked and not violations:
            failed.append(f"{name}: malformed changed entry escaped")
        elif not blocked and violations:
            failed.append(
                f"{name}: valid changed entry blocked by "
                f"{[(item.code, item.detail) for item in violations]}"
            )

    for spec, label in [
        (negative_spec, "negative-canonical"),
        (resurrection_spec, "resurrection-canonical"),
    ]:
        expect_boundary(
            f"{label}-added-bad",
            spec,
            "# Ledger\n",
            canonical_doc("REJECT bad", "bit-identical"),
            blocked=True,
        )
        expect_boundary(
            f"{label}-added-good",
            spec,
            "# Ledger\n",
            canonical_doc("REJECT good", aa),
            blocked=False,
        )
        expect_boundary(
            f"{label}-modified-bad",
            spec,
            canonical_doc("REJECT same", aa),
            canonical_doc("REJECT same", "bit-identical"),
            blocked=True,
        )

    expect_boundary(
        "canonical-next-h2-not-absorbed",
        negative_spec,
        "# Ledger\n",
        canonical_doc(
            "REJECT boundary",
            "bit-identical",
            f"\n## Appendix\n\n**{AA_MARKER}:** median ratio 1.001x\n",
        ),
        blocked=True,
    )
    expect_boundary(
        "canonical-next-h3-not-absorbed",
        negative_spec,
        "# Ledger\n",
        canonical_doc(
            "REJECT boundary",
            "bit-identical",
            f"\n### unrelated\n\n**{AA_MARKER}:** median ratio 1.001x\n",
        ),
        blocked=True,
    )
    expect_boundary(
        "legacy-added-bad",
        negative_spec,
        "# Ledger\n",
        legacy_doc("bad lever", "bit-identical", "REJECT"),
        blocked=True,
    )
    expect_boundary(
        "legacy-added-good",
        negative_spec,
        "# Ledger\n",
        legacy_doc(
            "good lever",
            f"<br>**{AA_MARKER}:** median ratio 1.001x",
            "REJECT",
        ),
        blocked=False,
    )
    expect_boundary(
        "legacy-modified-bad",
        negative_spec,
        legacy_doc(
            "same lever",
            f"<br>**{AA_MARKER}:** median ratio 1.001x",
            "REJECT",
        ),
        legacy_doc("same lever", "bit-identical", "REJECT"),
        blocked=True,
    )

    if failed:
        for failure in failed:
            print(f"self-test FAIL {failure}")
        return 1
    print(f"preflight self-test: PASS ({checks} cases)")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    modes = ap.add_mutually_exclusive_group()
    modes.add_argument("--candidate", help="Proposed lever description")
    modes.add_argument(
        "--check-new-rows",
        "--check-new-rejects",
        dest="check_new_rows",
        action="store_true",
        help="Validate added or modified verdict entries across both ledgers",
    )
    modes.add_argument("--check-disk", action="store_true", help="Run the free-space guard")
    modes.add_argument(
        "--run-compile",
        action="store_true",
        help="Disk-guard and run one cargo/rustc command",
    )
    modes.add_argument("--self-test", action="store_true", help="Run deterministic gate tests")
    ap.add_argument("--surface", help="Target source/workload surface for --candidate")
    ap.add_argument("--base", default="HEAD", help="Git ref to diff the ledger against")
    ap.add_argument(
        "--cached",
        action="store_true",
        help="Inspect staged ledger changes (pre-commit mode)",
    )
    ap.add_argument("--target-dir", help="Shared CARGO_TARGET_DIR for --run-compile")
    ap.add_argument(
        "--compile-timeout-seconds",
        type=float,
        default=DEFAULT_COMPILE_TIMEOUT_SECONDS,
        help="Compile timeout in seconds (default 3600)",
    )
    ap.add_argument(
        "--disk-path",
        type=Path,
        default=DEFAULT_DISK_PATH,
        help="Filesystem path checked by the disk guard (default /data)",
    )
    ap.add_argument(
        "--min-free-gib",
        type=float,
        default=DEFAULT_MIN_FREE_GIB,
        help="Minimum free GiB (default 120)",
    )
    args, command = ap.parse_known_args()
    if command and not args.run_compile:
        ap.error(f"unrecognized arguments: {' '.join(command)}")

    if args.candidate:
        return check_candidate(args.candidate, args.surface)
    if args.check_disk:
        return check_disk(args.disk_path, args.min_free_gib)
    if args.run_compile:
        return run_compile(
            command,
            target_dir=args.target_dir,
            disk_path=args.disk_path,
            min_free_gib=args.min_free_gib,
            timeout_seconds=args.compile_timeout_seconds,
        )
    if args.self_test:
        return self_test()
    return check_new_rows(args.base, cached=args.cached)


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except RuntimeError as error:
        print(f"preflight: {error}", file=sys.stderr)
        raise SystemExit(1) from None
