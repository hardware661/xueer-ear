"""Publish all verified assets together. Called only by the tag workflow's write-scoped job."""

import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess


ROOT = Path(__file__).resolve().parent.parent


def gh(*args, check=True):
    return subprocess.run(["gh", *args], text=True, capture_output=True, check=check)


def release_notes(changelog, tag):
    """The `## v1.2.3` section of CHANGELOG.md, up to the next `## ` heading."""
    lines = changelog.splitlines()
    try:
        start = lines.index("## " + tag) + 1
    except ValueError:
        raise ValueError(f"CHANGELOG.md has no section for {tag}") from None
    end = next((i for i in range(start, len(lines)) if lines[i].startswith("## ")), len(lines))
    notes = "\n".join(lines[start:end]).strip()
    if not notes:
        raise ValueError(f"CHANGELOG.md section for {tag} is empty")
    return notes


def publish(directory, version, tag, repository, notes):
    if not re.fullmatch(r"\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?", version) or tag != "v" + version:
        raise ValueError("tag and Cargo version differ")
    if not notes.strip():
        raise ValueError("release notes are empty")
    names = {f"xueer-ear-{version}-windows-x64-portable.zip",
             f"xueer-ear-{version}-windows-x64-setup.exe", "SHA256SUMS.txt"}
    actual = {path.name for path in directory.iterdir()}
    if actual != names:
        raise ValueError("release directory must contain exactly ZIP, installer and SHA256SUMS")
    hashes = {}
    for line in (directory / "SHA256SUMS.txt").read_text(encoding="utf-8").splitlines():
        digest, name = line.split("  ", 1)
        if not re.fullmatch(r"[a-f0-9]{64}", digest) or name not in names - {"SHA256SUMS.txt"} or name in hashes:
            raise ValueError("invalid checksum entry")
        hashes[name] = digest
    if set(hashes) != names - {"SHA256SUMS.txt"}:
        raise ValueError("both distributables require checksums")
    for name, expected in hashes.items():
        path = directory / name
        if path.stat().st_size == 0 or hashlib.sha256(path.read_bytes()).hexdigest() != expected:
            raise ValueError(f"checksum mismatch: {name}")

    def info():
        result = gh("release", "view", tag, "--repo", repository, "--json", "isDraft,assets", check=False)
        if result.returncode:
            return None
        return json.loads(result.stdout)

    def check_remote(release):
        assets = release["assets"]
        if {asset["name"] for asset in assets} != names or len(assets) != len(names):
            raise ValueError("remote release assets are incomplete")
        if any(asset["size"] != (directory / asset["name"]).stat().st_size for asset in assets):
            raise ValueError("remote release asset size mismatch")

    release = info()
    if release is not None and not release["isDraft"]:
        check_remote(release)
        print("complete release already published; leaving it unchanged")
        return
    if release is None:
        arguments = ["release", "create", tag, "--repo", repository, "--verify-tag", "--draft",
                     "--title", f"雪耳 {version}", "--notes", notes]
        if "-" in version:
            arguments.append("--prerelease")
        gh(*arguments)
    gh("release", "upload", tag, "--repo", repository, "--clobber",
       *(str(directory / name) for name in sorted(names)))
    release = info()
    if release is None or not release["isDraft"]:
        raise ValueError("release must remain a draft until all uploads are checked")
    check_remote(release)
    gh("release", "edit", tag, "--repo", repository, "--draft=false",
       "--prerelease=" + str("-" in version).lower(), "--latest=" + str("-" not in version).lower())
    print("published complete release: portable ZIP + installer + checksums")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--repository", required=True)
    args = parser.parse_args()
    notes = release_notes((ROOT / "CHANGELOG.md").read_text(encoding="utf-8"), args.tag)
    publish(args.directory, args.version, args.tag, args.repository, notes)


if __name__ == "__main__":
    main()
