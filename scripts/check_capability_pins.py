#!/usr/bin/env python3
"""Fail unless every capability crate is one exact public git revision (owner decision, #40).

Parses Cargo.toml and Cargo.lock (not raw text):
- every `moenarch-*` dependency (any dependency table) and every `[patch.crates-io]`
  `moenarch-*` entry is a git dependency on an allowed owner repository with a full commit
  `rev` and no version/branch/tag/path;
- each owner repository is pinned to exactly one rev across dependencies and patches;
- every resolved `moenarch-*` package in Cargo.lock appears once, from its repository's pin.
"""

from __future__ import annotations

import re
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OWNERS = {
    f"https://github.com/moritzbrantner/{name}"
    for name in ("nlp-stack", "moenarch-foundation", "visual-analysis")
}
FULL_REV = re.compile(r"^[0-9a-f]{40}$")
ALLOWED_KEYS = {"package", "git", "rev", "features", "default-features", "optional"}


def main() -> int:
    errors: list[str] = []
    manifest = tomllib.loads((ROOT / "Cargo.toml").read_text())
    pins: dict[str, set[str]] = {}

    entries: list[tuple[str, str, object]] = []
    tables = {name: manifest.get(name, {}) for name in ("dependencies", "dev-dependencies", "build-dependencies")}
    for target, spec in manifest.get("target", {}).items():
        for name in ("dependencies", "dev-dependencies", "build-dependencies"):
            tables[f"target.{target}.{name}"] = spec.get(name, {})
    for table, dependencies in tables.items():
        for key, spec in dependencies.items():
            package = spec.get("package", key) if isinstance(spec, dict) else key
            if package.startswith("moenarch-"):
                entries.append((f"[{table}] {key}", package, spec))
    for key, spec in manifest.get("patch", {}).get("crates-io", {}).items():
        package = spec.get("package", key) if isinstance(spec, dict) else key
        if package.startswith("moenarch-"):
            entries.append((f"[patch.crates-io] {key}", package, spec))

    for label, _package, spec in entries:
        if not isinstance(spec, dict):
            errors.append(f"{label}: must be a git table, not a version string")
            continue
        git, rev = spec.get("git"), spec.get("rev")
        if git not in OWNERS:
            errors.append(f"{label}: git must be one of {sorted(OWNERS)}")
        if not isinstance(rev, str) or not FULL_REV.fullmatch(rev):
            errors.append(f"{label}: rev must be a full 40-hex commit")
        extra = set(spec) - ALLOWED_KEYS
        if extra:
            errors.append(f"{label}: unexpected keys {sorted(extra)}")
        if git in OWNERS and isinstance(rev, str):
            pins.setdefault(git, set()).add(rev)

    for git, revs in sorted(pins.items()):
        if len(revs) != 1:
            errors.append(f"{git} is pinned to several revisions: {sorted(revs)}")

    lock = tomllib.loads((ROOT / "Cargo.lock").read_text())
    seen: dict[str, list[str]] = {}
    for package in lock.get("package", []):
        if package["name"].startswith("moenarch-"):
            seen.setdefault(package["name"], []).append(package.get("source", "<path>"))
    allowed_sources = {f"git+{git}?rev={rev}#{rev}" for git, revs in pins.items() for rev in revs}
    for name, sources in sorted(seen.items()):
        if len(sources) != 1 or sources[0] not in allowed_sources:
            errors.append(f"Cargo.lock: {name} must resolve once from a pinned git source, found {sources}")

    for error in errors:
        print(f"error: {error}", file=sys.stderr)
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
