#!/usr/bin/env python3
"""Check modular feature documentation, source ownership and PR synchronization.

Static integrity validation, not proof that the written behavior is correct.
"""
from __future__ import annotations

import argparse
import json
import re
import subprocess
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
FEATURES = ROOT / "docs/features"
CATALOG = FEATURES / "catalog.json"
LINK = re.compile(r"(?<!!)\[[^\]\n]+\]\(([^)\n]+)\)")


def managed_sources() -> set[str]:
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
        if p.is_file()
        and p.name != "tests.rs"
        and "/tests/" not in p.relative_to(ROOT).as_posix()
    }


def is_local_path(value: str) -> bool:
    return not (
        value.startswith("#")
        or value.startswith("/")
        or value.startswith("//")
        or re.match(r"^[a-z][a-z0-9+.-]*:", value, re.IGNORECASE)
    )


def check_links(md_path: Path, errors: list[str]) -> None:
    body = md_path.read_text(encoding="utf-8")
    for match in LINK.finditer(body):
        href = match.group(1).strip().split("#", 1)[0].split("?", 1)[0]
        if not href or not is_local_path(href):
            continue
        target = (md_path.parent / href).resolve()
        if not target.is_relative_to(ROOT) or not target.exists():
            errors.append(f"{md_path.relative_to(ROOT)}: broken link {href}")


def git_changed(base: str) -> set[str]:
    output = subprocess.check_output(
        ["git", "diff", "--name-only", "--diff-filter=ACMRT", f"{base}...HEAD"],
        cwd=ROOT,
        text=True,
    )
    return set(output.splitlines())


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--base",
        help="Git base ref for PR diff: changes to managed sources require feature docs updates",
    )
    args = parser.parse_args()
    data = json.loads(CATALOG.read_text(encoding="utf-8"))
    errors: list[str] = []
    if data.get("schema") != 2:
        errors.append("catalog.json: expected schema 2")

    modules = data.get("modules", [])
    features = data.get("features", [])
    mod_ids = [m["id"] for m in modules]
    ids = [f["id"] for f in features]
    docs = [f["doc"] for f in features]
    if len(mod_ids) != len(set(mod_ids)):
        errors.append("duplicate module IDs")
    if len(ids) != len(set(ids)):
        errors.append("duplicate feature IDs")
    if len(docs) != len(set(docs)):
        errors.append("duplicate canonical feature docs")

    top_index = FEATURES / "README.md"
    top_content = top_index.read_text(encoding="utf-8")
    indexes: set[str] = {"docs/features/README.md"}
    for module in modules:
        mid, index = module["id"], module["index"]
        expect = f"docs/features/{mid}/README.md"
        if not mid or mid.startswith("/") or ".." in Path(mid).parts or index != expect:
            errors.append(f"bad module index: {mid} -> {index}")
            continue
        indexes.add(index)
        idx_path = ROOT / index
        if not idx_path.is_file():
            errors.append(f"missing module README: {index}")
            continue
        parent_id = module.get("parent")
        parent_index = ROOT / (
            f"docs/features/{parent_id}/README.md"
            if parent_id else "docs/features/README.md"
        )
        if parent_id is not None and parent_id not in mod_ids:
            errors.append(f"{mid}: missing parent module {parent_id}")
        # Require an index link from parent to child.
        import os
        rel = os.path.relpath(idx_path, parent_index.parent).replace(os.sep, "/")
        if parent_index.is_file() and f"]({rel})" not in parent_index.read_text(encoding="utf-8"):
            errors.append(f"{index}: not linked from parent {parent_index.relative_to(ROOT)}")

    source_list: list[str] = []
    for feature in features:
        fid, module, doc = feature["id"], feature.get("module"), feature["doc"]
        expected_dir = f"docs/features/{module}/"
        if module not in mod_ids or not doc.startswith(expected_dir) or not doc.endswith(".md"):
            errors.append(f"{fid}: canonical doc outside its module: {doc}")
        if Path(doc).name == "README.md":
            errors.append(f"{fid}: module README cannot be the main feature doc")
        if not feature.get("status"):
            errors.append(f"{fid}: missing implementation status")
        if not feature.get("source"):
            errors.append(f"{fid}: missing source ownership")
        for path in [doc, *feature.get("source", []), *feature.get("tests", [])]:
            if path.startswith("/") or ".." in Path(path).parts or not (ROOT / path).is_file():
                errors.append(f"{fid}: missing or invalid reference: {path}")
        doc_path = ROOT / doc
        idx_path = ROOT / f"docs/features/{module}/README.md"
        if doc_path.is_file():
            body = doc_path.read_text(encoding="utf-8")
            for path in [*feature.get("source", []), *feature.get("tests", [])]:
                if path not in body:
                    errors.append(f"{fid}: source/test absent from Markdown: {path}")
        if idx_path.is_file():
            relative = doc_path.name
            if f"]({relative})" not in idx_path.read_text(encoding="utf-8"):
                errors.append(f"{fid}: not linked from {idx_path.relative_to(ROOT)}")
        source_list.extend(feature.get("source", []))

    occurrences = Counter(source_list)
    for path, count in sorted(occurrences.items()):
        if count != 1:
            errors.append(f"{path}: assigned {count} times")
    expected = managed_sources()
    for path in sorted(expected - occurrences.keys()):
        errors.append(f"unmapped main source: {path}")
    for path in sorted(occurrences.keys() - expected):
        errors.append(f"stale main source: {path}")

    all_docs = {
        p.relative_to(ROOT).as_posix()
        for p in FEATURES.rglob("*.md")
    }
    for path in sorted(all_docs - set(docs) - indexes):
        errors.append(f"orphan Markdown outside catalog/module index: {path}")
    for path in sorted(set(docs) | indexes):
        if not (ROOT / path).is_file():
            errors.append(f"missing Markdown: {path}")
    for path in sorted(all_docs):
        check_links(ROOT / path, errors)

    if args.base:
        try:
            changed = git_changed(args.base)
        except subprocess.CalledProcessError as exc:
            errors.append(f"cannot compute PR diff: {exc}")
        else:
            for feature in features:
                touched = changed.intersection(feature.get("source", []))
                if touched and feature["doc"] not in changed:
                    errors.append(
                        f"{feature['id']}: code changed ({', '.join(sorted(touched))}) "
                        f"without updating {feature['doc']}"
                    )

    if errors:
        for issue in errors:
            print("ERROR:", issue)
        raise SystemExit(f"Failed: {len(errors)} feature documentation issue(s)")
    print(
        f"OK: {len(modules)} modules, {len(features)} features, "
        f"{len(expected)} primary source files, canonical links and ownership verified"
    )


if __name__ == "__main__":
    main()
