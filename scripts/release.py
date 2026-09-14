#!/usr/bin/env python3
"""Package the CLI and assemble a complete, version-pinned GitHub release."""

import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import tarfile

ROOT = Path(__file__).resolve().parents[1]
TARGETS = (
    "x86_64-unknown-linux-musl",
    "aarch64-unknown-linux-musl",
    "x86_64-apple-darwin",
    "aarch64-apple-darwin",
)


def package_version():
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--format-version", "1", "--no-deps", "--locked"],
        cwd=ROOT,
    ))
    package = next(p for p in metadata["packages"] if p["name"] == "t")
    version = package["version"]
    if not re.fullmatch(r"(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)", version):
        raise ValueError("releases require a stable MAJOR.MINOR.PATCH package version")
    return version


def validate_tag(version, tag):
    if tag and tag != f"v{version}":
        raise ValueError(f"tag {tag!r} does not match package version v{version}")


def archive_name(version, target):
    return f"t-v{version}-{target}.tar.gz"


def package(version, target, output):
    binary = ROOT / "target" / target / "release" / "t"
    if not binary.is_file():
        raise ValueError(f"missing release binary: {binary}")
    output.mkdir(parents=True, exist_ok=True)
    archive = output / archive_name(version, target)
    with tarfile.open(archive, "w:gz") as tar:
        tar.add(binary, arcname="t")
        tar.add(ROOT / "README.md", arcname="README.md")
    print(archive)


def assemble(version, output):
    expected = {archive_name(version, target) for target in TARGETS}
    actual = {path.name for path in output.glob("*.tar.gz")}
    if actual != expected:
        raise ValueError(f"archive set mismatch: missing={expected - actual}, unexpected={actual - expected}")
    for name in sorted(expected):
        with tarfile.open(output / name, "r:gz") as tar:
            members = tar.getmembers()
            if {m.name for m in members} != {"t", "README.md"} or len(members) != 2:
                raise ValueError(f"unexpected archive contents: {name}")
            if any(not m.isfile() for m in members) or not tar.getmember("t").mode & 0o111:
                raise ValueError(f"archive requires regular files and an executable binary: {name}")
    template = (ROOT / "scripts" / "install.sh").read_text()
    if template.count("@VERSION@") != 1:
        raise ValueError("installer must contain exactly one version marker")
    (output / "install.sh").write_text(template.replace("@VERSION@", f"v{version}"))
    names = sorted(expected | {"install.sh"})
    checksums = "".join(
        f"{hashlib.sha256((output / name).read_bytes()).hexdigest()}  {name}\n"
        for name in names
    )
    (output / "SHA256SUMS").write_text(checksums)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    version_parser = commands.add_parser("version")
    version_parser.add_argument("--tag", default="")
    package_parser = commands.add_parser("package")
    package_parser.add_argument("--target", choices=TARGETS, required=True)
    package_parser.add_argument("--output", type=Path, required=True)
    assemble_parser = commands.add_parser("assemble")
    assemble_parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        version = package_version()
        if args.command == "version":
            validate_tag(version, args.tag)
            print(version)
        elif args.command == "package":
            package(version, args.target, args.output)
        else:
            assemble(version, args.output)
    except (ValueError, OSError, subprocess.CalledProcessError, tarfile.TarError) as error:
        parser.exit(1, f"release: {error}\n")


if __name__ == "__main__":
    main()
