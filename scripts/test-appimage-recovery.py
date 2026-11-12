#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Exercise the bundled updater against an isolated, built Linux AppImage.

This intentionally corrupts the candidate ELF machine field after copying the
real image, then passes its actual hash to test launch failure and rollback.
It never installs software or accesses the user's HOME or application data.
"""
import hashlib
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tempfile
import time


def digest(path):
    result = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            result.update(chunk)
    return result.hexdigest()


def main(image):
    image = Path(image).resolve(strict=True)
    if not sys.platform.startswith("linux"):
        raise SystemExit("This test requires Linux")
    with tempfile.TemporaryDirectory(prefix="sorane-appimage-recovery-") as directory:
        root = Path(directory)
        home = root / "home"
        home.mkdir()
        env = os.environ.copy()
        for name in ("DISPLAY", "WAYLAND_DISPLAY", "APPIMAGE", "APPDIR", "ARGV0", "OWD", "XDG_RUNTIME_DIR"):
            env.pop(name, None)
        env.update(HOME=str(home), XDG_CONFIG_HOME=str(home / "config"), XDG_CACHE_HOME=str(home / "cache"), XDG_DATA_HOME=str(home / "data"))
        installed = root / "installed.AppImage"
        candidate = root / "candidate.AppImage"
        shutil.copy2(image, installed)
        shutil.copy2(image, candidate)
        before = digest(installed)
        subprocess.run([str(installed), "--appimage-extract", "usr/bin/ibisOfficeUpdater"], cwd=root, env=env, check=True, stdout=subprocess.DEVNULL, timeout=60)
        helper = root / "squashfs-root/usr/bin/ibisOfficeUpdater"
        if not helper.is_file() or not os.access(helper, os.X_OK):
            raise RuntimeError("bundled updater was not extracted")
        with candidate.open("r+b") as stream:
            header = stream.read(20)
            if header[:4] != b"\x7fELF" or header[8:11] != b"AI\x02":
                raise RuntimeError("candidate is not a type 2 AppImage")
            stream.seek(18)
            stream.write(b"\xff\xff")  # invalid ELF machine: deterministic exec failure
        after = digest(candidate)
        if after == before:
            raise RuntimeError("candidate corruption did not change its hash")
        parent = subprocess.Popen(["/bin/sleep", "1"], env=env)
        updater = None
        try:
            updater = subprocess.Popen([
                str(helper), "--pid", str(parent.pid), "--version", "99.0.0",
                "--old", str(installed), "--new", str(candidate),
                "--log", str(root / "update.log"), "--wait-secs", "10",
                "--size", str(candidate.stat().st_size), "--sha256", after,
            ], cwd=root, env=env, start_new_session=True)
            # Keep the test-owned leader unreaped while terminating its isolated
            # process group, so its PID cannot be recycled for another process.
            deadline = time.monotonic() + 30
            while not os.waitid(os.P_PID, updater.pid, os.WEXITED | os.WNOHANG | os.WNOWAIT):
                if time.monotonic() >= deadline:
                    raise TimeoutError("updater did not finish")
                time.sleep(0.1)
        finally:
            if updater is not None:
                try:
                    os.killpg(updater.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                status = updater.wait(timeout=10)
            parent.wait(timeout=10)
        log = (root / "update.log").read_text()
        if status != 1 or " launch FAIL " not in log or " rollback ok " not in log:
            raise AssertionError(f"unexpected updater result {status}:\n{log}")
        if digest(installed) != before or Path(str(installed) + ".update-backup").exists():
            raise AssertionError("the original AppImage was not restored")
        if digest(candidate) != after:
            raise AssertionError("the candidate changed unexpectedly")
        print("Bundled updater restored the original AppImage after an exec failure")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {sys.argv[0]} /path/to/built.AppImage")
    main(sys.argv[1])
