#!/usr/bin/env python3
"""Validate manual release inputs and stamp Cargo versions in the build checkout."""
import argparse
import os
from pathlib import Path
import re
import tomllib

VERSION = re.compile(
    r"v?(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)"
    r"(?:-(alpha|beta)(?:\.(0|[1-9][0-9]*))?)?"
)


def release_version(raw: str, channel: str) -> str:
    if channel not in ("stable", "beta", "alpha"):
        raise ValueError("channel must be stable, beta or alpha")
    match = VERSION.fullmatch(raw)
    if not match:
        raise ValueError("version must be X.Y.Z, optionally prefixed with v and suffixed with -alpha.N or -beta.N")
    major, minor, patch, suffix, number = match.groups()
    if suffix and suffix != channel:
        raise ValueError("version suffix must match the selected channel")
    version = f"{major}.{minor}.{patch}"
    if channel != "stable":
        version += f"-{channel}.{number if number is not None else '1'}"
    return version


def stamp_workspace(root: Path, version: str) -> None:
    manifest_path = root / "Cargo.toml"
    lock_path = root / "Cargo.lock"
    manifest_text = manifest_path.read_text()
    manifest = tomllib.loads(manifest_text)
    old = manifest["workspace"]["package"]["version"]
    lock_text = lock_path.read_text()
    lock = tomllib.loads(lock_text)
    names = {"eagler-ws", "eaglerx-pumpkin"}
    packages = [p for p in lock["package"] if p["name"] in names and "source" not in p]
    if {p["name"] for p in packages} != names or any(p["version"] != old for p in packages):
        raise ValueError("workspace versions and Cargo.lock do not match")
    manifest_text, count = re.subn(
        r'(\[workspace\.package\]\s*\nversion\s*=\s*)"[^"\n]+"',
        lambda m: f'{m[1]}"{version}"', manifest_text,
    )
    if count != 1:
        raise ValueError("could not locate the workspace package version")
    for name in names:
        pattern = rf'(\[\[package\]\]\nname = "{re.escape(name)}"\nversion = )"{re.escape(old)}"'
        lock_text, count = re.subn(pattern, lambda m: f'{m[1]}"{version}"', lock_text)
        if count != 1:
            raise ValueError(f"could not locate locked workspace package {name}")
    # Only write once both files have been validated. Dependency versions stay locked.
    manifest_path.write_text(manifest_text)
    lock_path.write_text(lock_text)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", required=True)
    parser.add_argument("--channel", required=True, choices=("stable", "beta", "alpha"))
    parser.add_argument("--workspace", type=Path, default=Path(__file__).resolve().parents[1])
    args = parser.parse_args()
    try:
        version = release_version(args.version, args.channel)
        stamp_workspace(args.workspace, version)
    except (ValueError, KeyError, OSError) as error:
        parser.exit(2, f"Release preparation failed: {error}\n")
    outputs = {"version": version, "tag": f"v{version}", "channel": args.channel}
    if output_file := os.environ.get("GITHUB_OUTPUT"):
        with open(output_file, "a") as output:
            output.writelines(f"{key}={value}\n" for key, value in outputs.items())
    print(f"Prepared {outputs['tag']} ({args.channel})")


if __name__ == "__main__":
    main()
