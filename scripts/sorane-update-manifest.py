#!/usr/bin/env python3
"""Prepare signed Sorane Docs update metadata; never publishes or creates a key."""
import argparse
import hashlib
import json
import re
import subprocess
import time
from pathlib import Path

TARGETS = {"macos-arm64", "macos-x64", "windows-x64", "linux-x64-appimage", "linux-arm64-appimage"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", required=True)
    parser.add_argument("--private-key", type=Path, required=True, help="offline Ed25519 PEM private key")
    parser.add_argument("--out", type=Path, required=True, help="new empty output directory")
    parser.add_argument("--openssl", default="openssl", help="OpenSSL 3 executable (macOS system LibreSSL does not support Ed25519 signing)")
    parser.add_argument("--days", type=int, default=14, choices=range(1, 32))
    parser.add_argument("artifact", nargs="+", help="target=path")
    args = parser.parse_args()
    if not re.fullmatch(r"[0-9]{1,9}(?:\.[0-9]{1,9})*", args.version):
        parser.error("invalid numeric version")
    artifacts, seen, filenames = [], set(), set()
    for item in args.artifact:
        target, sep, path = item.partition("=")
        file = Path(path)
        if not sep or target not in TARGETS or target in seen or not re.fullmatch(r"[A-Za-z0-9_-][A-Za-z0-9_.-]{0,179}", file.name) or ".." in file.name:
            parser.error("invalid target, duplicate target, or unsafe filename")
        if file.name in filenames:
            parser.error("artifact filenames must be unique across targets")
        seen.add(target)
        filenames.add(file.name)
        digest, size = hashlib.sha256(), 0
        with file.open("rb") as src:
            while chunk := src.read(1024 * 1024):
                size += len(chunk)
                digest.update(chunk)
        if not 0 < size <= 2 * 1024 * 1024 * 1024:
            parser.error("invalid artifact size")
        artifacts.append(dict(target=target, filename=file.name, size=size, sha256=digest.hexdigest()))
    if len(artifacts) > 16:
        parser.error("too many artifacts")
    args.out.mkdir(parents=True, exist_ok=False)
    now = int(time.time())
    manifest = dict(schema=1, product="Sorane Docs", channel="stable", version=args.version,
                    issued_at=now, expires_at=now + args.days * 86400, artifacts=artifacts)
    path = args.out / "sorane-update.json"
    path.write_bytes((json.dumps(manifest, ensure_ascii=False, separators=(",", ":")) + "\n").encode())
    signature = args.out / "signature.bin"
    subprocess.run([args.openssl, "pkeyutl", "-sign", "-rawin", "-inkey", str(args.private_key), "-in", str(path), "-out", str(signature)], check=True)
    raw = signature.read_bytes()
    signature.unlink()
    if len(raw) != 64:
        raise ValueError("expected an Ed25519 signature")
    (args.out / "sorane-update.json.sig").write_text(raw.hex() + "\n", encoding="ascii")
    print(f"Prepared {path} and its detached signature; no files were published.")


if __name__ == "__main__":
    main()
