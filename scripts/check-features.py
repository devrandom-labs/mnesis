#!/usr/bin/env python3
"""Check one workspace library without unifying other members' features."""

import argparse
import json
import os
from pathlib import Path
import subprocess


def run(command):
    print("+ " + " ".join(command), flush=True)
    subprocess.run(command, check=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("package")
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    os.chdir(root)
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--no-deps", "--locked", "--format-version", "1"],
        cwd=root,
    ))
    package = next((item for item in metadata["packages"]
                    if item["name"] == args.package
                    and item["id"] in metadata["workspace_members"]), None)
    if package is None:
        parser.error("package must be a workspace member")
    modes = [[], ["--no-default-features"], ["--all-features"]]
    modes.extend(["--no-default-features", "--features", feature]
                 for feature in sorted(package["features"]) if feature != "default")
    for mode in modes:
        run(["cargo", "check", "-p", args.package, "--lib", "--locked", *mode])
    # Unit tests use dev dependencies, which can add features. The library
    # checks above deliberately exclude that graph; target no_std gates in
    # flake.nix additionally check whether dependencies require std.
    for mode in modes[:3]:
        run(["cargo", "test", "-p", args.package, "--lib", "--locked", *mode])


if __name__ == "__main__":
    main()
