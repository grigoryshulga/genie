#!/usr/bin/env python3
"""Validate the source and release metadata of a PR into main."""

from __future__ import annotations

import json
import os
import re
import subprocess
import sys
import tomllib


def fail(message: str) -> None:
    print(f"::error::{message}")
    sys.exit(1)


def git_show(revision: str, path: str) -> str:
    return subprocess.check_output(["git", "show", f"{revision}:{path}"], text=True)


def cargo_version(contents: str) -> str:
    data = tomllib.loads(contents)
    return data["workspace"]["package"]["version"]


def semver(version: str) -> tuple[int, int, int]:
    match = re.fullmatch(r"(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)", version)
    if not match:
        fail(f"Expected a stable SemVer version, got {version!r}")
    return tuple(map(int, match.groups()))


def version_at(revision: str) -> str:
    cargo = cargo_version(git_show(revision, "Cargo.toml"))
    package = json.loads(git_show(revision, "package.json"))["version"]
    lock = tomllib.loads(git_show(revision, "Cargo.lock"))
    cargo_lock_versions = {
        item["version"] for item in lock["package"] if item["name"] in {"genie", "genie-core"}
    }
    npm_lock = json.loads(git_show(revision, "package-lock.json"))
    npm_lock_version = npm_lock["packages"][""]["version"]
    if cargo_lock_versions != {cargo}:
        fail(f"Cargo.lock workspace versions {sorted(cargo_lock_versions)} do not match Cargo.toml {cargo}")
    if package != cargo or npm_lock_version != package:
        fail(
            "Cargo.toml, package.json, and package-lock.json must all use the same version "
            f"(got {cargo}, {package}, {npm_lock_version})"
        )
    return cargo


def main() -> None:
    if os.environ.get("HEAD_REF") != "dev":
        fail("Pull requests into main must come from the dev branch")

    base_sha = os.environ["BASE_SHA"]
    head_sha = os.environ["HEAD_SHA"]
    changed = set(
        subprocess.check_output(
            ["git", "diff", "--name-only", f"{base_sha}...{head_sha}"], text=True
        ).splitlines()
    )
    required_files = {"Cargo.toml", "Cargo.lock", "package.json", "package-lock.json", "CHANGELOG.md"}
    missing = sorted(required_files - changed)
    if missing:
        fail(f"Release PR must update these version/release files: {', '.join(missing)}")

    old_version = version_at(base_sha)
    new_version = version_at(head_sha)
    if semver(new_version) <= semver(old_version):
        fail(f"Release version must increase from {old_version}; got {new_version}")

    changelog = git_show(head_sha, "CHANGELOG.md")
    release_heading = re.compile(
        rf"^## \[{re.escape(new_version)}\] — \d{{4}}-\d{{2}}-\d{{2}}$", re.MULTILINE
    )
    if not release_heading.search(changelog):
        fail(f"CHANGELOG.md must contain a dated '## [{new_version}] — YYYY-MM-DD' section")

    print(f"PR from dev prepares release {new_version}; Cargo/npm versions and changelog agree.")


if __name__ == "__main__":
    main()
