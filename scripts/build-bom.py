#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Generate scoped CycloneDX inventories for ibisOffice CI build outputs."""

import argparse
from collections import defaultdict
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import subprocess
from tempfile import TemporaryDirectory
from urllib.parse import quote
import xml.etree.ElementTree as ET
import zipfile

PLATFORMS = {
    "linux-x64-appimage": ("Linux", "x86_64-unknown-linux-gnu", ("ibis-office-app", "ibis-office-updater", "ibis-appimage"), False),
    "flatpak-x64": ("Linux", "x86_64-unknown-linux-gnu", ("ibis-office-app",), False),
    "flatpak-arm64": ("Linux", "aarch64-unknown-linux-gnu", ("ibis-office-app",), False),
    "macos-arm64": ("macOS", "aarch64-apple-darwin", ("ibis-office-app", "ibis-office-updater"), False),
    "windows-x64": ("Windows", "x86_64-pc-windows-msvc", ("ibis-office-app", "ibis-office-updater"), False),
    "android-arm64": ("Android", "aarch64-linux-android", ("ibis-office-app",), True),
    "ios-arm64": ("iOS", "aarch64-apple-ios", ("ibis-office-app",), True),
    "wasm": ("wasm", "wasm32-unknown-unknown", ("ibis-office-app", "ibis-heic-web"), False),
}
PACKAGE = re.compile(r"(\d+)\s*(\S+) v(\S+)(?: \((.*)\))?")


def cargo_graph(target, packages, no_default_features, app_ref):
    command = ["cargo", "tree", "--locked", "--color", "never", "--target", target,
               "--edges", "normal,no-proc-macro", "--prefix", "depth", "--format", "{p}"]
    if no_default_features:
        command.append("--no-default-features")
    for package in packages:
        command.extend(("-p", package))
    tree = subprocess.check_output(command, text=True)
    components, origins, edges = {}, {}, defaultdict(set)
    ancestors = []
    roots = set()
    for line in tree.splitlines():
        if not line.strip():
            continue
        match = PACKAGE.fullmatch(line[:-4] if line.endswith(" (*)") else line)
        if not match:
            raise ValueError(f"unexpected cargo tree line: {line}")
        depth, name, version, origin = match.groups()
        depth = int(depth)
        ref = f"pkg:cargo/{name}@{version}"
        if ref in origins and origins[ref] != origin:
            raise ValueError(f"ambiguous Cargo identity: {ref}")
        origins[ref] = origin
        component = {"type": "library", "bom-ref": ref, "name": name, "version": version}
        if origin is None:
            component["purl"] = ref
        components[ref] = component
        if depth > len(ancestors):
            raise ValueError(f"unexpected Cargo tree depth: {line}")
        del ancestors[depth:]
        if ancestors:
            edges[ancestors[-1]].add(ref)
        else:
            roots.add(name)
            edges[app_ref].add(ref)
        ancestors.append(ref)
    if roots != set(packages):
        raise ValueError(f"unexpected Cargo roots: {sorted(roots)}")
    return components, edges


def stream_sha256(stream):
    digest = hashlib.sha256()
    for chunk in iter(lambda: stream.read(1024 * 1024), b""):
        digest.update(chunk)
    return digest.hexdigest()


def sha256(path):
    with path.open("rb") as stream:
        return stream_sha256(stream)


def inventory(platform, image, components, edges, scope, app_ref):
    label, target, _, _ = PLATFORMS[platform]
    revision = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
    version = next(line.split("\t", 1)[1] for line in Path("version.txt").read_text().splitlines()
                   if line.startswith(f"{label}\t"))
    return {
        "bomFormat": "CycloneDX", "specVersion": "1.6", "version": 1,
        "metadata": {
            "component": {
                "type": "application", "bom-ref": app_ref, "name": f"ibisOffice {platform}",
                "version": version, "hashes": [{"alg": "SHA-256", "content": sha256(image)}],
            },
            "properties": [
                {"name": "ibisOffice:sourceCommit", "value": revision},
                {"name": "ibisOffice:cargoTarget", "value": target},
                {"name": "ibisOffice:inventoryScope", "value": scope},
            ],
        },
        "components": [components[ref] for ref in sorted(components)],
        "compositions": [{"aggregate": "incomplete"}],
        "dependencies": [
            {"ref": ref, "dependsOn": sorted(edges.get(ref, ()))}
            for ref in sorted([app_ref, *components])
        ],
    }


def add_elf_links(path, source, components, edges):
    with path.open("rb") as stream:
        if stream.read(4) != b"\x7fELF":
            return
    output = subprocess.check_output(
        ["readelf", "-dW", path], text=True, env={**os.environ, "LC_ALL": "C"}
    )
    for line in output.splitlines():
        if "(NEEDED)" not in line:
            continue
        match = re.search(r"Shared library: \[([^\]]+)\]$", line.strip())
        if not match:
            raise ValueError(f"unexpected ELF NEEDED entry: {line}")
        name = match[1]
        ref = f"ibisoffice:elf:needed:{quote(name, safe='')}"
        components[ref] = {
            "type": "library", "bom-ref": ref, "name": name,
            "properties": [{"name": "ibisOffice:evidence", "value": "ELF DT_NEEDED name"}],
        }
        edges[source].add(ref)


def add_payload(appdir, components, edges, namespace):
    if not appdir.is_dir():
        raise ValueError(f"missing payload directory: {appdir}")
    for path in sorted(appdir.rglob("*")):
        name = path.relative_to(appdir).as_posix()
        ref = f"ibisoffice:{namespace}:file:{quote(name, safe='/')}"
        component = {"type": "file", "bom-ref": ref, "name": name}
        if path.is_symlink():
            component["properties"] = [
                {"name": f"ibisOffice:{namespace}:symlinkTarget", "value": os.readlink(path)},
            ]
        elif path.is_file():
            component["hashes"] = [{"alg": "SHA-256", "content": sha256(path)}]
            add_elf_links(path, ref, components, edges)
        else:
            continue
        components[ref] = component


def add_zip_payload(archive, components):
    with zipfile.ZipFile(archive) as payload:
        for entry in payload.infolist():
            if entry.is_dir():
                continue
            name = entry.filename
            ref = f"ibisoffice:archive:file:{quote(name, safe='/')}"
            if ref in components:
                raise ValueError(f"duplicate ZIP entry: {name}")
            component = {"type": "file", "bom-ref": ref, "name": name}
            with payload.open(entry) as stream:
                if stat.S_ISLNK(entry.external_attr >> 16):
                    component["properties"] = [
                        {"name": "ibisOffice:archive:symlinkTarget", "value": stream.read().decode("utf-8")},
                    ]
                else:
                    component["hashes"] = [{"alg": "SHA-256", "content": stream_sha256(stream)}]
            components[ref] = component


def add_crypto(components, edges):
    # Capabilities reviewed in source for the selected build.
    # Required direct dependencies detect manifest changes that need review.
    def record(package, required, source, usage, assets):
        consumers = [ref for ref, component in components.items() if component["name"] == package]
        if not consumers:
            return
        if len(consumers) != 1:
            raise ValueError(f"expected exactly one Cargo component for {package}")
        consumer = consumers[0]
        dependencies = {components[ref]["name"] for ref in edges[consumer]}
        if not set(required) <= dependencies:
            raise ValueError(f"review {package} crypto evidence after Cargo dependency changes")
        if not Path(source).is_file():
            raise ValueError(f"missing crypto evidence source: {source}")
        for name, properties in assets.items():
            ref = f"ibisoffice:crypto:algorithm:{name}"
            asset = components.setdefault(ref, {
                "type": "cryptographic-asset", "bom-ref": ref, "name": name,
                "cryptoProperties": {"assetType": "algorithm", "algorithmProperties": properties},
                "properties": [],
            })
            if asset["cryptoProperties"]["algorithmProperties"] != properties:
                raise ValueError(f"conflicting algorithm properties for {name}")
            asset["properties"].extend([
                {"name": "ibisOffice:evidence:source", "value": source},
                {"name": "ibisOffice:evidence:usage", "value": usage},
            ])
            edges[consumer].add(ref)

    aes = {
        f"AES-{bits}-CBC": {
            "primitive": "block-cipher", "mode": "cbc",
            "parameterSetIdentifier": str(bits), "cryptoFunctions": ["encrypt", "decrypt"],
        }
        for bits in (128, 192, 256)
    }
    record("ibis-ooxml", ("aes", "cbc"), "crates/ibis-ooxml/src/encryption.rs",
           "OOXML document encryption/decryption", aes)
    record("ibis-pdf", ("aes", "cbc", "md-5", "sha2"), "crates/ibis-pdf/src/crypt.rs",
           "PDF Standard Security Handler decryption and password/key processing", {
               "AES-128-CBC": aes["AES-128-CBC"],
               "AES-256-CBC": aes["AES-256-CBC"],
               "RC4": {"primitive": "stream-cipher", "cryptoFunctions": ["decrypt"]},
               **{name: {"primitive": "hash", "cryptoFunctions": ["digest"]}
                  for name in ("MD5", "SHA-256", "SHA-384", "SHA-512")},
           })
    record("ibis-ooxml", ("rsa", "sha1", "sha2", "p256", "p384", "x509-cert"),
           "crates/ibis-ooxml/src/signatures.rs", "OOXML XMLDSig SignedInfo signature value verification",
           {
               **{f"RSA-PKCS1-v1_5-{digest}": {"primitive": "signature", "cryptoFunctions": ["verify"]}
                  for digest in ("SHA-1", "SHA-256", "SHA-384", "SHA-512")},
               "RSA-PSS-SHA-256": {
                   "primitive": "signature", "cryptoFunctions": ["verify"],
                   "parameterSetIdentifier": "SHA-256/MGF1-SHA-256/salt-length-32",
               },
               **{f"ECDSA-P-{bits}-SHA-{bits}": {
                   "primitive": "signature", "curve": f"P-{bits}", "cryptoFunctions": ["verify"],
               } for bits in (256, 384)},
           })
    record("ibis-ofd", ("sm2", "sm3"), "crates/ibis-ofd/src/signature.rs",
           "OFD signature value and reference digest verification", {
               "SM2-with-SM3": {"primitive": "signature", "cryptoFunctions": ["verify"]},
               "SM3": {"primitive": "hash", "cryptoFunctions": ["digest"]},
           })


def verify_workbook(app, bom, crypto):
    with TemporaryDirectory() as directory:
        workbook = Path(directory) / "inventory.xlsx"
        subprocess.run([str(app.resolve()), "--export", "sbom-table", str(bom.resolve()),
                        "--out-file", str(workbook)], check=True)
        with zipfile.ZipFile(workbook) as archive:
            root = ET.fromstring(archive.read("xl/workbook.xml"))
        sheets = {sheet.attrib["name"] for sheet in root.findall(".//{*}sheet")}
        expected = {"Components"}
        if crypto:
            expected.update(("Cryptographic assets", "Crypto relationships"))
        if not expected <= sheets:
            raise ValueError(f"missing XLSX sheets: {sorted(expected - sheets)}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("platform", choices=PLATFORMS)
    parser.add_argument("image", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--appdir", type=Path, help="Linux package payload directory")
    parser.add_argument("--verify-with", type=Path, help="desktop ibisOffice CLI for XLSX smoke test")
    args = parser.parse_args()
    _, target, packages, no_default_features = PLATFORMS[args.platform]
    app_ref = f"ibisoffice:{args.platform}"
    selections = [(package,) for package in packages] if args.platform == "wasm" else [packages]
    components, edges = {}, defaultdict(set)
    for selection in selections:
        found, links = cargo_graph(target, selection, no_default_features, app_ref)
        components.update(found)
        for source, targets in links.items():
            edges[source].update(targets)
    if args.platform == "linux-x64-appimage":
        if args.appdir is None:
            parser.error("--appdir is required for the Linux AppImage")
        if not all((args.appdir / name).is_file() for name in (
            "usr/bin/ibisOffice", "usr/bin/ibisOfficeUpdater",
        )):
            parser.error("expected the extracted AppImage binaries")
        add_elf_links(args.image, app_ref, components, edges)
        add_payload(args.appdir, components, edges, "appimage")
        payload_scope = ("Extracted AppImage files and symlinks, plus direct ELF DT_NEEDED from "
                         "the runtime and payload; host resolution and dlopen are outside scope.")
    elif args.platform.startswith("flatpak-"):
        if args.appdir is None or not (args.appdir / "bin/ibisOffice").is_file():
            parser.error("--appdir must point to the Flatpak build export's files directory")
        add_payload(args.appdir, components, edges, "flatpak")
        payload_scope = "Flatpak application files and ELF DT_NEEDED; Freedesktop runtime inventory is outside scope."
    else:
        add_zip_payload(args.image, components)
        payload_scope = "ZIP payload entries and symlinks; native system linkage is outside scope."
        if args.platform == "wasm":
            payload_scope = "Generated www/pkg WASM and glue files; deployed Worker and site assets are outside scope."
    scope = "Target Cargo normal dependencies, excluding build/dev dependencies and proc macros. " + payload_scope
    sbom = inventory(args.platform, args.image, components, edges, scope, app_ref)
    add_crypto(components, edges)
    cbom = inventory(args.platform, args.image, components, edges, scope +
                     " Reviewed OOXML/PDF encryption and OOXML/OFD signatures when present in the Cargo graph.", app_ref)
    args.output.mkdir(parents=True, exist_ok=True)
    for kind, data in (("sbom", sbom), ("cbom", cbom)):
        path = args.output / f"{args.platform}.{kind}.cdx.json"
        path.write_text(
            json.dumps(data, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        if args.verify_with:
            crypto = any(c["type"] == "cryptographic-asset" for c in data["components"])
            verify_workbook(args.verify_with, path, crypto)


if __name__ == "__main__":
    main()
