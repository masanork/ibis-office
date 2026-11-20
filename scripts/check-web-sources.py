#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Check direct uses of browser APIs incompatible with the embed contract."""
import argparse
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
JS_SINK = re.compile(
    r"\b(?:new\s+Function|eval)\s*\(|\.(?:innerHTML|outerHTML)\s*=(?!=)|\.insertAdjacentHTML\s*\("
)
RUST_SINK = re.compile(r"\b(?:js_sys::)?Function::new[_\w]*\s*\(")
DOCUMENT_SINK = re.compile(r"\bdocument\s*\.\s*(?:write|writeln)\s*\(")
STORAGE = re.compile(r"\b(?:localStorage|sessionStorage)\b|\bdocument\s*\.\s*cookie\b")
ALLOW = re.compile(r"//\s*host-lint-allow:\s*\S")
INLINE_SCRIPT = re.compile(r"<script\b(?![^>]*\bsrc\s*=)[^>]*>\s*\S", re.I)


def violations(lines, *, check_storage, rust=False):
    for number, line in enumerate(lines, 1):
        if line.lstrip().startswith(("//", "/*", "*")):
            continue
        if (RUST_SINK if rust else JS_SINK).search(line):
            yield number, "dynamic JavaScript or HTML"
        elif DOCUMENT_SINK.search(line):
            yield number, "document.write"
        if check_storage and STORAGE.search(line) and not ALLOW.search(line):
            yield number, "shared page storage needs an explicit host exclusion"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dist", action="store_true")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        assert list(violations(["new Function(body)"], check_storage=False))
        assert list(violations(["document.write(body)"], check_storage=False))
        assert list(violations(["el.innerHTML = html"], check_storage=False))
        assert list(violations(["localStorage.getItem(key)"], check_storage=True))
        assert not list(violations(
            ["localStorage.getItem(key); // host-lint-allow: playground only"],
            check_storage=True,
        ))
        assert INLINE_SCRIPT.search("<script>init()</script>")
        assert not INLINE_SCRIPT.search('<script src="main.js"></script>')
        return 0

    rust_sources = list((ROOT / "crates").glob("ibis-office-*/js/*.js"))
    rust_sources += list((ROOT / "crates").glob("ibis-office-*/src/*.rs"))
    names = ("boot", "home-redirect", "main", "host-loader", "ibisoffice-host", "chunked-fetch", "subset-fetch")
    files = [ROOT / "www" / f"{name}.ts" for name in names] + rust_sources
    if args.dist:
        files = [ROOT / "www" / f"{name}.js" for name in names]
        files.extend((ROOT / "www/pkg").rglob("*.js"))
    issues = []
    for path in files:
        data = path.read_text(encoding="utf-8")
        check_storage = path.stem in names and not args.dist
        if path == ROOT / "crates/ibis-office-base/src/settings.rs":
            check_storage = True
        for number, reason in violations(data.splitlines(), check_storage=check_storage, rust=path.suffix == ".rs"):
            issues.append(f"{path.relative_to(ROOT)}:{number}: {reason}")
    for name in ("index.html", "editor.html"):
        path = ROOT / "www" / name
        if INLINE_SCRIPT.search(path.read_text(encoding="utf-8")):
            issues.append(f"www/{name}: inline script")
    for issue in issues:
        print(issue)
    return bool(issues)


if __name__ == "__main__":
    raise SystemExit(main())
