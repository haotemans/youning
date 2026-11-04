#!/usr/bin/env python3
"""Verify that each main runtime source belongs to exactly one feature document.

Structural checks only: cannot prove that behavior described in Markdown is true.
"""
from __future__ import annotations

import json
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CATALOG = ROOT / "docs/features/catalog.json"


def managed_sources():
    candidates = [
        *ROOT.glob("crates/core/src/**/*.rs"),
        *ROOT.glob("crates/adapter-qq/src/**/*.rs"),
        *ROOT.glob("crates/webui/src/**/*.rs"),
        *ROOT.glob("crates/yuantuan/src/**/*.rs"),
        *ROOT.glob("crates/plugin-api/src/**/*.rs"),
        *ROOT.glob("plugins/*/src/**/*.rs"),
        *ROOT.glob("webui-frontend/src/views/*.vue"),
    ]
    return {
        p.relative_to(ROOT).as_posix()
        for p in candidates
        if p.is_file() and p.name != "tests.rs"
        and "/tests/" not in p.relative_to(ROOT).as_posix()
    }


def main():
    data = json.loads(CATALOG.read_text(encoding="utf-8"))
    failures = []
    features = data.get("features", [])
    ids = [x["id"] for x in features]
    docs = [x["doc"] for x in features]
    if len(ids) != len(set(ids)):
        failures.append("duplicate feature IDs")
    if len(docs) != len(set(docs)):
        failures.append("duplicate primary documents")

    declared = []
    for item in features:
        fid = item["id"]
        doc = item["doc"]
        if doc != f"docs/features/{fid}.md":
            failures.append(f"{fid}: primary doc name does not match feature ID")
        if not item.get("source"):
            failures.append(f"{fid}: missing primary source mapping")
        for path in [doc, *item.get("source", []), *item.get("tests", [])]:
            if path.startswith("/") or ".." in Path(path).parts:
                failures.append(f"{fid}: unsafe path {path}")
                continue
            if not (ROOT / path).is_file():
                failures.append(f"{fid}: missing file {path}")
        if (ROOT / doc).is_file():
            content = (ROOT / doc).read_text(encoding="utf-8")
            for path in item.get("source", []):
                if path not in content:
                    failures.append(f"{fid}: doc does not mention source {path}")
            for path in item.get("tests", []):
                if path not in content:
                    failures.append(f"{fid}: doc does not mention test {path}")
        declared.extend(item.get("source", []))

    counts = Counter(declared)
    for path, count in sorted(counts.items()):
        if count > 1:
            failures.append(f"{path}: assigned to {count} features")
    actual = managed_sources()
    for path in sorted(actual - counts.keys()):
        failures.append(f"unmapped source: {path}")
    for path in sorted(counts.keys() - actual):
        failures.append(f"no longer managed source: {path}")
    if failures:
        for error in failures:
            print("ERROR:", error)
        print(f"Failed: {len(failures)} documentation mapping issue(s)")
        raise SystemExit(1)
    print(f"OK: {len(features)} features, {len(actual)} source files, unique ownership, docs/tests exist")


if __name__ == "__main__":
    main()
