#!/usr/bin/env python3
"""Generate a deterministic pandas API coverage and bead drift report.

The report compares the checked pandas API listing plus live top-level
`pd.read_*` helpers against FrankenPandas public Rust methods and the Beads
graph. It is intentionally read-only: tracker updates are reported as exact
drift findings, not applied.

Usage:
    python3 scripts/validate_api_coverage_drift.py \
      --json-out artifacts/api-coverage-drift-2026-05-08.json \
      --markdown-out artifacts/api-coverage-drift-2026-05-08.md
    python3 scripts/validate_api_coverage_drift.py \
      --check-json artifacts/api-coverage-drift-2026-05-08.json \
      --check-markdown artifacts/api-coverage-drift-2026-05-08.md
"""
from __future__ import annotations

import argparse
import json
import re
import sys
from collections import Counter, defaultdict
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Iterable

REPO_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_LISTING = REPO_ROOT / "artifacts" / "pandas_api_listing.json"
DEFAULT_ISSUES = REPO_ROOT / ".beads" / "issues.jsonl"

BACKTICK_RE = re.compile(r"`([^`]+)`")
METHOD_TOKEN_RE = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")
RUST_FN_IDENT = r"((?:r#)?[A-Za-z_][A-Za-z0-9_]*)"
PUB_FN_RE = re.compile(rf"^\s*pub\s+fn\s+{RUST_FN_IDENT}\b")
FN_RE = re.compile(rf"^\s*fn\s+{RUST_FN_IDENT}\b")
TOP_LEVEL_PUB_FN_RE = re.compile(rf"^pub\s+fn\s+{RUST_FN_IDENT}\b")

IO_DATAFRAME_METHODS = {
    "to_clipboard",
    "to_csv",
    "to_excel",
    "to_feather",
    "to_gbq",
    "to_hdf",
    "to_html",
    "to_json",
    "to_latex",
    "to_markdown",
    "to_orc",
    "to_parquet",
    "to_pickle",
    "to_sql",
    "to_stata",
    "to_string",
    "to_xml",
}
STATIC_PANDAS_READERS = {
    "read_clipboard",
    "read_csv",
    "read_excel",
    "read_feather",
    "read_fwf",
    "read_gbq",
    "read_hdf",
    "read_html",
    "read_json",
    "read_orc",
    "read_parquet",
    "read_pickle",
    "read_sas",
    "read_spss",
    "read_sql",
    "read_sql_query",
    "read_sql_table",
    "read_stata",
    "read_table",
    "read_xml",
}


def normalize_rust_identifier(name: str) -> str:
    return name.removeprefix("r#")


@dataclass(frozen=True)
class RustImpl:
    path: str
    type_name: str


@dataclass(frozen=True)
class SurfaceSpec:
    name: str
    pandas_classes: tuple[str, ...]
    rust_impls: tuple[RustImpl, ...]
    parent_ids: tuple[str, ...]
    keywords: tuple[str, ...]


SURFACES = (
    SurfaceSpec(
        name="DataFrame",
        pandas_classes=("DataFrame",),
        rust_impls=(RustImpl("crates/fp-frame/src/lib.rs", "DataFrame"),),
        parent_ids=("frankenpandas-gd11l",),
        keywords=("dataframe", "frame"),
    ),
    SurfaceSpec(
        name="SeriesGroupBy",
        pandas_classes=("SeriesGroupBy",),
        rust_impls=(RustImpl("crates/fp-frame/src/lib.rs", "SeriesGroupBy"),),
        parent_ids=("frankenpandas-nt65g",),
        keywords=("seriesgroupby", "series groupby", "groupby"),
    ),
    SurfaceSpec(
        name="MultiIndex",
        pandas_classes=("MultiIndex",),
        rust_impls=(RustImpl("crates/fp-index/src/lib.rs", "MultiIndex"),),
        parent_ids=("frankenpandas-d89fe", "frankenpandas-1q5v6"),
        keywords=("multiindex", "multi index"),
    ),
    SurfaceSpec(
        name="IndexVariants",
        pandas_classes=(
            "DatetimeIndex",
            "TimedeltaIndex",
            "PeriodIndex",
            "RangeIndex",
            "CategoricalIndex",
        ),
        rust_impls=(
            RustImpl("crates/fp-index/src/lib.rs", "DatetimeIndex"),
            RustImpl("crates/fp-index/src/lib.rs", "TimedeltaIndex"),
            RustImpl("crates/fp-index/src/lib.rs", "PeriodIndex"),
            RustImpl("crates/fp-index/src/lib.rs", "RangeIndex"),
            RustImpl("crates/fp-index/src/lib.rs", "CategoricalIndex"),
        ),
        parent_ids=("frankenpandas-k2l3y", "frankenpandas-t8hz0"),
        keywords=(
            "datetimeindex",
            "timedeltaindex",
            "periodindex",
            "rangeindex",
            "categoricalindex",
            "index variants",
        ),
    ),
)


def parse_timestamp(value: str | None) -> datetime | None:
    if not value:
        return None
    text = value.strip()
    if not text:
        return None
    if text.endswith("Z"):
        text = text[:-1] + "+00:00"
    if "." in text:
        prefix, suffix = text.split(".", 1)
        match = re.match(r"(?P<fraction>\d+)(?P<tail>.*)", suffix)
        if match:
            fraction = match.group("fraction")[:6].ljust(6, "0")
            text = f"{prefix}.{fraction}{match.group('tail')}"
    parsed = datetime.fromisoformat(text)
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=timezone.utc)
    return parsed.astimezone(timezone.utc)


def format_timestamp(value: datetime | None) -> str | None:
    if value is None:
        return None
    return value.astimezone(timezone.utc).isoformat(timespec="seconds").replace("+00:00", "Z")


def load_json(path: Path) -> dict[str, Any]:
    decoder = json.JSONDecoder()
    return decoder.decode(path.read_text(encoding="utf-8"))


def load_issues(path: Path) -> tuple[dict[str, dict[str, Any]], list[dict[str, Any]]]:
    decoder = json.JSONDecoder()
    issues: dict[str, dict[str, Any]] = {}
    ordered: list[dict[str, Any]] = []
    with path.open(encoding="utf-8") as handle:
        for line_no, line in enumerate(handle, 1):
            if not line.strip():
                continue
            try:
                issue = decoder.decode(line)
            except json.JSONDecodeError as exc:
                raise ValueError(f"{path}:{line_no}: invalid JSON: {exc}") from exc
            issue_id = str(issue.get("id") or "")
            if not issue_id:
                raise ValueError(f"{path}:{line_no}: missing issue id")
            issues[issue_id] = issue
            ordered.append(issue)
    return issues, ordered


def max_issue_timestamp(ordered: Iterable[dict[str, Any]]) -> datetime:
    candidates: list[datetime] = []
    for issue in ordered:
        for key in ("updated_at", "closed_at", "created_at"):
            parsed = parse_timestamp(issue.get(key))
            if parsed is not None:
                candidates.append(parsed)
    if not candidates:
        return datetime.now(timezone.utc).replace(microsecond=0)
    return max(candidates)


def relpath(path: Path) -> str:
    try:
        return str(path.relative_to(REPO_ROOT))
    except ValueError:
        return str(path)


def pandas_class_members(listing: dict[str, Any], class_name: str) -> set[str]:
    entry = listing.get("classes", {}).get(class_name, {})
    members = entry.get("members", []) if isinstance(entry, dict) else []
    return {
        str(member.get("name"))
        for member in members
        if isinstance(member, dict) and member.get("name")
    }


def pandas_top_level_readers(expected_version: str | None) -> tuple[set[str], str]:
    try:
        import pandas as pd  # type: ignore
    except ImportError:
        return set(STATIC_PANDAS_READERS), "static_fallback"
    version = str(getattr(pd, "__version__", "unknown"))
    readers = {name for name in dir(pd) if name.startswith("read_")}
    if expected_version and version != expected_version:
        return readers, f"live_pandas_version_mismatch:{version}"
    return readers, f"live_pandas:{version}"


def rust_code_for_braces(line: str, in_block_comment: bool) -> tuple[str, bool]:
    out: list[str] = []
    i = 0
    in_string = False
    in_char = False
    escaped = False
    while i < len(line):
        ch = line[i]
        nxt = line[i + 1] if i + 1 < len(line) else ""
        if in_block_comment:
            if ch == "*" and nxt == "/":
                in_block_comment = False
                i += 2
            else:
                i += 1
            continue
        if in_string:
            if escaped:
                escaped = False
            elif ch == "\\":
                escaped = True
            elif ch == '"':
                in_string = False
            i += 1
            continue
        if in_char:
            if escaped:
                escaped = False
            elif ch == "\\":
                escaped = True
            elif ch == "'":
                in_char = False
            i += 1
            continue
        if ch == "/" and nxt == "*":
            in_block_comment = True
            i += 2
            continue
        if ch == "/" and nxt == "/":
            break
        if ch == '"':
            in_string = True
            i += 1
            continue
        if ch == "'":
            in_char = True
            i += 1
            continue
        out.append(ch)
        i += 1
    return "".join(out), in_block_comment


def impl_start_re(type_name: str) -> re.Pattern[str]:
    return re.compile(
        rf"^\s*impl(?:<[^>{{}}]*>)?\s+{re.escape(type_name)}"
        rf"(?:\s*<[^>{{}}]*>)?\s*(?:where\b[^\{{]*)?\s*\{{"
    )


def trait_impl_start_re(trait_name: str, type_name: str) -> re.Pattern[str]:
    return re.compile(
        rf"^\s*impl(?:<[^>{{}}]*>)?\s+{re.escape(trait_name)}\s+for\s+"
        rf"{re.escape(type_name)}(?:\s*<[^>{{}}]*>)?\s*(?:where\b[^\{{]*)?\s*\{{"
    )


def extract_impl_methods(path: Path, type_name: str, public_only: bool = True) -> set[str]:
    lines = path.read_text(encoding="utf-8").splitlines()
    start_re = impl_start_re(type_name)
    fn_re = PUB_FN_RE if public_only else FN_RE
    methods: set[str] = set()
    in_impl = False
    for line in lines:
        if not in_impl:
            if start_re.search(line):
                in_impl = True
                match = fn_re.search(line)
                if match:
                    methods.add(normalize_rust_identifier(match.group(1)))
            continue
        if line == "}":
            in_impl = False
            continue
        match = fn_re.search(line)
        if match:
            methods.add(normalize_rust_identifier(match.group(1)))
    return methods


def extract_trait_impl_methods(path: Path, trait_name: str, type_name: str) -> set[str]:
    lines = path.read_text(encoding="utf-8").splitlines()
    start_re = trait_impl_start_re(trait_name, type_name)
    methods: set[str] = set()
    in_impl = False
    for line in lines:
        if not in_impl:
            if start_re.search(line):
                in_impl = True
                match = FN_RE.search(line)
                if match:
                    methods.add(normalize_rust_identifier(match.group(1)))
            continue
        if line == "}":
            in_impl = False
            continue
        match = FN_RE.search(line)
        if match:
            methods.add(normalize_rust_identifier(match.group(1)))
    return methods


def extract_top_level_pub_functions(path: Path) -> set[str]:
    methods: set[str] = set()
    in_block_comment = False
    depth = 0
    for line in path.read_text(encoding="utf-8").splitlines():
        code, in_block_comment = rust_code_for_braces(line, in_block_comment)
        if depth == 0:
            match = TOP_LEVEL_PUB_FN_RE.search(code)
            if match:
                methods.add(normalize_rust_identifier(match.group(1)))
        depth += code.count("{") - code.count("}")
        if depth < 0:
            depth = 0
    return methods


def collect_rust_methods(spec: SurfaceSpec) -> tuple[set[str], dict[str, list[str]]]:
    all_methods: set[str] = set()
    by_impl: dict[str, list[str]] = {}
    for impl in spec.rust_impls:
        path = REPO_ROOT / impl.path
        methods = extract_impl_methods(path, impl.type_name)
        all_methods.update(methods)
        by_impl[f"{impl.path}::{impl.type_name}"] = sorted(methods)
    return all_methods, by_impl


def collect_io_rust_methods() -> tuple[set[str], dict[str, list[str]]]:
    frame_path = REPO_ROOT / "crates/fp-frame/src/lib.rs"
    io_path = REPO_ROOT / "crates/fp-io/src/lib.rs"
    frame_methods = extract_impl_methods(frame_path, "DataFrame")
    top_level = extract_top_level_pub_functions(io_path)
    trait_impl = extract_trait_impl_methods(io_path, "DataFrameIoExt", "DataFrame")
    io_methods = {
        method
        for method in frame_methods | top_level | trait_impl
        if method.startswith(("read_", "write_", "to_", "from_"))
        or method in IO_DATAFRAME_METHODS
    }
    return io_methods, {
        "crates/fp-frame/src/lib.rs::DataFrame": sorted(
            method for method in frame_methods if method.startswith(("to_", "from_"))
        ),
        "crates/fp-io/src/lib.rs::top_level": sorted(top_level),
        "crates/fp-io/src/lib.rs::DataFrameIoExt for DataFrame": sorted(trait_impl),
    }


def methods_from_description(description: str) -> set[str]:
    methods: set[str] = set()
    for span in BACKTICK_RE.findall(description or ""):
        for token in re.split(r"[,\s/]+", span):
            token = token.strip()
            if METHOD_TOKEN_RE.match(token):
                methods.add(token)
    return methods


def issue_text(issue: dict[str, Any]) -> str:
    parts = [
        str(issue.get("id") or ""),
        str(issue.get("title") or ""),
        str(issue.get("description") or ""),
        str(issue.get("close_reason") or ""),
    ]
    return "\n".join(parts).lower()


def parent_child_edges(ordered: Iterable[dict[str, Any]]) -> dict[str, list[str]]:
    children_by_parent: dict[str, list[str]] = defaultdict(list)
    for issue in ordered:
        for dep in issue.get("dependencies") or []:
            if dep.get("type") != "parent-child":
                continue
            parent = dep.get("depends_on_id")
            child = dep.get("issue_id")
            if parent and child:
                children_by_parent[str(parent)].append(str(child))
    return {parent: sorted(set(children)) for parent, children in children_by_parent.items()}


def open_child_mentions_method(
    method: str,
    spec: SurfaceSpec,
    issues: dict[str, dict[str, Any]],
    ordered: list[dict[str, Any]],
    children_by_parent: dict[str, list[str]],
) -> bool:
    method_re = re.compile(rf"(?<![A-Za-z0-9_]){re.escape(method.lower())}(?![A-Za-z0-9_])")
    candidate_ids = set()
    for parent_id in spec.parent_ids:
        candidate_ids.update(children_by_parent.get(parent_id, []))
    for issue in ordered:
        if issue.get("status") not in {"open", "in_progress"}:
            continue
        text = issue_text(issue)
        if not method_re.search(text):
            continue
        issue_id = str(issue.get("id"))
        if issue_id in candidate_ids:
            return True
        if any(keyword in text for keyword in spec.keywords):
            return True
    return False


def parent_missing_methods(
    spec: SurfaceSpec, issues: dict[str, dict[str, Any]]
) -> dict[str, list[str]]:
    out: dict[str, list[str]] = {}
    for parent_id in spec.parent_ids:
        issue = issues.get(parent_id)
        if not issue:
            continue
        out[parent_id] = sorted(methods_from_description(issue.get("description") or ""))
    return out


def build_surface_report(
    spec: SurfaceSpec,
    listing: dict[str, Any],
    issues: dict[str, dict[str, Any]],
    ordered: list[dict[str, Any]],
    children_by_parent: dict[str, list[str]],
) -> dict[str, Any]:
    target_by_class = {
        class_name: sorted(pandas_class_members(listing, class_name))
        for class_name in spec.pandas_classes
    }
    target = sorted(set().union(*(set(items) for items in target_by_class.values())))
    rust_methods, rust_by_impl = collect_rust_methods(spec)
    implemented = sorted(set(target) & rust_methods)
    missing = sorted(set(target) - rust_methods)
    parent_missing = parent_missing_methods(spec, issues)
    listed_missing = sorted(set().union(*(set(items) for items in parent_missing.values())))
    implemented_but_listed_missing = sorted(set(implemented) & set(listed_missing))
    missing_without_open_child = sorted(
        method
        for method in missing
        if not open_child_mentions_method(method, spec, issues, ordered, children_by_parent)
    )
    return {
        "pandas_classes": target_by_class,
        "target_count": len(target),
        "implemented_count": len(implemented),
        "missing_count": len(missing),
        "coverage_percent": round((len(implemented) / len(target) * 100.0) if target else 100.0, 2),
        "implemented_methods": implemented,
        "missing_methods": missing,
        "rust_methods_by_impl": rust_by_impl,
        "parent_missing_methods": parent_missing,
        "drift": {
            "implemented_but_listed_missing": implemented_but_listed_missing,
            "missing_without_open_child": missing_without_open_child,
        },
    }


def build_io_report(
    listing: dict[str, Any],
    issues: dict[str, dict[str, Any]],
    ordered: list[dict[str, Any]],
    children_by_parent: dict[str, list[str]],
    pandas_reader_source: tuple[set[str], str],
) -> dict[str, Any]:
    dataframe_members = pandas_class_members(listing, "DataFrame")
    dataframe_targets = sorted(dataframe_members & IO_DATAFRAME_METHODS)
    read_targets = sorted(pandas_reader_source[0])
    target = sorted(set(dataframe_targets) | set(read_targets))
    rust_methods, rust_by_impl = collect_io_rust_methods()
    implemented = sorted(set(target) & rust_methods)
    missing = sorted(set(target) - rust_methods)
    spec = SurfaceSpec(
        name="IOWrappers",
        pandas_classes=("DataFrame", "pandas_top_level"),
        rust_impls=(),
        parent_ids=("frankenpandas-vqjc0",),
        keywords=("io", "read_", "to_", "csv", "json", "parquet", "excel"),
    )
    parent_missing = parent_missing_methods(spec, issues)
    listed_missing = sorted(set().union(*(set(items) for items in parent_missing.values())))
    implemented_but_listed_missing = sorted(set(implemented) & set(listed_missing))
    missing_without_open_child = sorted(
        method
        for method in missing
        if not open_child_mentions_method(method, spec, issues, ordered, children_by_parent)
    )
    return {
        "pandas_classes": {
            "DataFrame": dataframe_targets,
            "pandas_top_level": read_targets,
        },
        "pandas_reader_source": pandas_reader_source[1],
        "target_count": len(target),
        "implemented_count": len(implemented),
        "missing_count": len(missing),
        "coverage_percent": round((len(implemented) / len(target) * 100.0) if target else 100.0, 2),
        "implemented_methods": implemented,
        "missing_methods": missing,
        "rust_methods_by_impl": rust_by_impl,
        "parent_missing_methods": parent_missing,
        "drift": {
            "implemented_but_listed_missing": implemented_but_listed_missing,
            "missing_without_open_child": missing_without_open_child,
        },
    }


def build_report(listing_path: Path, issues_path: Path) -> dict[str, Any]:
    listing = load_json(listing_path)
    issues, ordered = load_issues(issues_path)
    children_by_parent = parent_child_edges(ordered)
    as_of = max_issue_timestamp(ordered)
    pandas_version = str(listing.get("pandas_version") or "")
    pandas_reader_source = pandas_top_level_readers(pandas_version)
    surfaces = {
        spec.name: build_surface_report(spec, listing, issues, ordered, children_by_parent)
        for spec in SURFACES
    }
    surfaces["IOWrappers"] = build_io_report(
        listing,
        issues,
        ordered,
        children_by_parent,
        pandas_reader_source,
    )
    status_counts = Counter(str(issue.get("status")) for issue in ordered)
    finding_counts = {
        "implemented_but_listed_missing": sum(
            len(surface["drift"]["implemented_but_listed_missing"])
            for surface in surfaces.values()
        ),
        "missing_without_open_child": sum(
            len(surface["drift"]["missing_without_open_child"])
            for surface in surfaces.values()
        ),
    }
    return {
        "generated_by": "scripts/validate_api_coverage_drift.py",
        "listing_path": relpath(listing_path),
        "issues_path": relpath(issues_path),
        "as_of": format_timestamp(as_of),
        "pandas_version": pandas_version,
        "summary": {
            "surface_count": len(surfaces),
            "status_counts": dict(sorted(status_counts.items())),
            "finding_counts": finding_counts,
            "surfaces": {
                name: {
                    "target_count": surface["target_count"],
                    "implemented_count": surface["implemented_count"],
                    "missing_count": surface["missing_count"],
                    "coverage_percent": surface["coverage_percent"],
                    "implemented_but_listed_missing_count": len(
                        surface["drift"]["implemented_but_listed_missing"]
                    ),
                    "missing_without_open_child_count": len(
                        surface["drift"]["missing_without_open_child"]
                    ),
                }
                for name, surface in sorted(surfaces.items())
            },
        },
        "surfaces": dict(sorted(surfaces.items())),
    }


def render_method_list(methods: list[str], limit: int = 40) -> str:
    if not methods:
        return "_none_"
    shown = methods[:limit]
    text = ", ".join(f"`{method}`" for method in shown)
    if len(methods) > limit:
        text += f", ... ({len(methods) - limit} more)"
    return text


def render_markdown(report: dict[str, Any]) -> str:
    lines = [
        "# API Coverage Drift Report",
        "",
        f"Generated by `scripts/validate_api_coverage_drift.py` from `{report['listing_path']}` and `{report['issues_path']}`.",
        "",
        f"- As of: `{report['as_of']}`",
        f"- Pandas version: `{report['pandas_version']}`",
        f"- Surfaces: `{report['summary']['surface_count']}`",
        "",
        "## Summary",
        "",
        "| Surface | Targets | Implemented | Missing | Coverage | Implemented but listed missing | Missing without open child |",
        "|---|---:|---:|---:|---:|---:|---:|",
    ]
    for name, summary in sorted(report["summary"]["surfaces"].items()):
        lines.append(
            f"| `{name}` | {summary['target_count']} | {summary['implemented_count']} | "
            f"{summary['missing_count']} | {summary['coverage_percent']:.2f}% | "
            f"{summary['implemented_but_listed_missing_count']} | "
            f"{summary['missing_without_open_child_count']} |"
        )
    lines.extend(["", "## Drift Findings", ""])
    for name, surface in sorted(report["surfaces"].items()):
        drift = surface["drift"]
        lines.extend([f"### `{name}`", ""])
        lines.append(
            "**Implemented methods still listed as missing:** "
            + render_method_list(drift["implemented_but_listed_missing"])
        )
        lines.append("")
        lines.append(
            "**Missing pandas methods without an open child bead:** "
            + render_method_list(drift["missing_without_open_child"])
        )
        lines.append("")
    lines.extend(["## Scanner Inputs", ""])
    for name, surface in sorted(report["surfaces"].items()):
        lines.append(f"### `{name}`")
        lines.append("")
        if "pandas_reader_source" in surface:
            lines.append(f"- Pandas reader source: `{surface['pandas_reader_source']}`")
        lines.append(f"- Implemented methods: {render_method_list(surface['implemented_methods'])}")
        lines.append(f"- Missing methods: {render_method_list(surface['missing_methods'])}")
        lines.append("")
    return "\n".join(lines).rstrip() + "\n"


def write_text(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


def compare_expected(path: Path, expected: str, label: str) -> bool:
    if not path.is_file():
        print(f"error: {label} check target missing: {path}", file=sys.stderr)
        return False
    actual = path.read_text(encoding="utf-8")
    if actual != expected:
        print(f"error: {label} is stale: {path}", file=sys.stderr)
        return False
    print(f"{label} up to date: {path}")
    return True


def main(argv: Iterable[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--listing", type=Path, default=DEFAULT_LISTING)
    parser.add_argument("--issues", type=Path, default=DEFAULT_ISSUES)
    parser.add_argument("--json-out", type=Path)
    parser.add_argument("--markdown-out", type=Path)
    parser.add_argument("--check-json", type=Path)
    parser.add_argument("--check-markdown", type=Path)
    args = parser.parse_args(list(argv) if argv is not None else None)

    listing_path = args.listing if args.listing.is_absolute() else REPO_ROOT / args.listing
    issues_path = args.issues if args.issues.is_absolute() else REPO_ROOT / args.issues
    try:
        report = build_report(listing_path, issues_path)
    except (OSError, ValueError, ImportError) as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 2

    rendered_json = json.dumps(report, indent=2, sort_keys=True) + "\n"
    rendered_markdown = render_markdown(report)
    ok = True
    if args.json_out:
        out = args.json_out if args.json_out.is_absolute() else REPO_ROOT / args.json_out
        write_text(out, rendered_json)
    if args.markdown_out:
        out = args.markdown_out if args.markdown_out.is_absolute() else REPO_ROOT / args.markdown_out
        write_text(out, rendered_markdown)
    if args.check_json:
        target = args.check_json if args.check_json.is_absolute() else REPO_ROOT / args.check_json
        ok = compare_expected(target, rendered_json, "json") and ok
    if args.check_markdown:
        target = (
            args.check_markdown
            if args.check_markdown.is_absolute()
            else REPO_ROOT / args.check_markdown
        )
        ok = compare_expected(target, rendered_markdown, "markdown") and ok
    if not (args.json_out or args.markdown_out or args.check_json or args.check_markdown):
        print(rendered_json, end="")
    return 0 if ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
