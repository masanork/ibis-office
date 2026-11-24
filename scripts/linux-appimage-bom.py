#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Generate scoped CycloneDX inventories for the Linux AppImage CI build."""

import argparse
from collections import defaultdict
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
from urllib.parse import quote

TARGET = "x86_64-unknown-linux-gnu"
ROOTS = ("ibis-office-app", "ibis-office-updater", "ibis-appimage")
PACKAGE = re.compile(r"(\d+)\s*(\S+) v(\S+)(?: \((.*)\))?")
APP_REF = "ibisoffice:linux-x64-appimage"


def cargo_graph():
    command = ["cargo", "tree", "--locked", "--target", TARGET,
               "--edges", "normal,no-proc-macro", "--prefix", "depth", "--format", "{p}"]
    for package in ROOTS:
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
            edges[APP_REF].add(ref)
        ancestors.append(ref)
    if roots != set(ROOTS):
        raise ValueError(f"unexpected Cargo roots: {sorted(roots)}")
    return components, edges


def sha256(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def inventory(image, components, edges, scope):
    revision = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
    version = next(line.split("\t", 1)[1] for line in Path("version.txt").read_text().splitlines()
                   if line.startswith("Linux\t"))
    return {
        "bomFormat": "CycloneDX", "specVersion": "1.6", "version": 1,
        "metadata": {
            "component": {
                "type": "application", "bom-ref": APP_REF, "name": "ibisOffice Linux x64 AppImage",
                "version": version, "hashes": [{"alg": "SHA-256", "content": sha256(image)}],
            },
            "properties": [
                {"name": "ibisOffice:sourceCommit", "value": revision},
                {"name": "ibisOffice:cargoTarget", "value": TARGET},
                {"name": "ibisOffice:inventoryScope", "value": scope},
            ],
        },
        "components": [components[ref] for ref in sorted(components)],
        "compositions": [{"aggregate": "incomplete"}],
        "dependencies": [
            {"ref": ref, "dependsOn": sorted(edges.get(ref, ()))}
            for ref in sorted([APP_REF, *components])
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


def add_payload(image, appdir, components, edges):
    if not appdir.is_dir() or not all((appdir / name).is_file() for name in (
        "usr/bin/ibisOffice", "usr/bin/ibisOfficeUpdater",
    )):
        raise ValueError("expected the extracted Linux AppImage payload")
    add_elf_links(image, APP_REF, components, edges)
    for path in sorted(appdir.rglob("*")):
        name = path.relative_to(appdir).as_posix()
        ref = f"ibisoffice:appimage:file:{quote(name, safe='/')}"
        component = {"type": "file", "bom-ref": ref, "name": name}
        if path.is_symlink():
            component["properties"] = [
                {"name": "ibisOffice:appimage:symlinkTarget", "value": os.readlink(path)},
            ]
        elif path.is_file():
            component["hashes"] = [{"alg": "SHA-256", "content": sha256(path)}]
            add_elf_links(path, ref, components, edges)
        else:
            continue
        components[ref] = component


def add_crypto(components, edges):
    # Capabilities reviewed in source for the selected build.
    # Required direct dependencies detect manifest changes that need review.
    def record(package, required, source, usage, assets):
        consumers = [ref for ref, component in components.items() if component["name"] == package]
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


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("image", type=Path)
    parser.add_argument("appdir", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    components, edges = cargo_graph()
    add_payload(args.image, args.appdir, components, edges)
    scope = ("Linux release Cargo normal dependency graph, extracted AppImage files and symlinks, "
             "and direct ELF DT_NEEDED names from the runtime and payload. "
             "Excludes build/dev dependencies and proc macros. ELF names describe link requirements "
             "without resolving host library versions, indirect host dependencies, or dlopen.")
    sbom = inventory(args.image, components, edges, scope)
    add_crypto(components, edges)
    cbom = inventory(args.image, components, edges, scope + " Curated source coverage: OOXML encryption, PDF encryption, and OOXML/OFD signatures.")
    args.output.mkdir(parents=True, exist_ok=True)
    for kind, data in (("sbom", sbom), ("cbom", cbom)):
        (args.output / f"linux-x64-appimage.{kind}.cdx.json").write_text(
            json.dumps(data, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
