#!/usr/bin/env python3
"""Install a built Linux binary and its matching desktop integration for this user."""

import os
from pathlib import Path
import shutil
import subprocess
import tempfile


ROOT = Path(__file__).resolve().parent.parent
APP_ID = "dev.sorane.Docs"


def install(source: Path, destination: Path, mode: int) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(dir=destination.parent, delete=False) as temporary:
        name = Path(temporary.name)
    try:
        shutil.copyfile(source, name)
        name.chmod(mode)
        os.replace(name, destination)
    finally:
        name.unlink(missing_ok=True)


def main() -> None:
    home = Path.home()
    data = Path(os.environ.get("XDG_DATA_HOME") or home / ".local/share")
    binary = home / ".local/bin/ibisOffice"
    source = ROOT / "target/release/ibisOffice"
    if not source.is_file():
        raise SystemExit("Build first: cargo build --release -p ibis-office-app --locked")
    install(source, binary, 0o755)
    for size in (16, 24, 32, 48, 256):
        install(ROOT / f"crates/ibis-office-app/branding/png/{size}.png",
                data / f"icons/hicolor/{size}x{size}/apps/{APP_ID}.png", 0o644)
    # Refresh an existing theme cache so desktop shells can discover the icon.
    # The per-user hicolor directory need not have its own index.theme.
    if shutil.which("gtk-update-icon-cache"):
        subprocess.run(["gtk-update-icon-cache", "-f", "-t", str(data / "icons/hicolor")], check=True)

    # Exec values use desktop-entry escaping, then quoted-argument escaping.
    arg = str(binary)
    for char in ('\\', '"', '`', '$'):
        arg = arg.replace(char, '\\' + char)
    arg = arg.replace('\\', '\\\\').replace('%', '%%')
    template = (ROOT / "crates/ibis-office-app/linux/ibisoffice.desktop").read_text()
    entry = '\n'.join(f'Exec="{arg}" %F' if line.startswith('Exec=') else line
                      for line in template.splitlines()) + '\n'
    applications = data / "applications"
    applications.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(mode="w", encoding="utf-8", dir=applications, delete=False) as temporary:
        name = Path(temporary.name)
        temporary.write(entry)
    try:
        name.chmod(0o644)
        os.replace(name, applications / f"{APP_ID}.desktop")
    finally:
        name.unlink(missing_ok=True)
    if shutil.which("update-desktop-database"):
        subprocess.run(["update-desktop-database", str(applications)], check=True)
    print(f"Installed {binary} and {APP_ID}.desktop")


if __name__ == "__main__":
    main()
