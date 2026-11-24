#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Generate scoped CycloneDX inventories for the Linux AppImage CI build."""

import argparse
from collections import defaultdict
import hashlib
import json
from pathlib import Path
import re
import subprocess

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


def add_crypto(components, edges):
    # Curated source evidence: decrypt_cbc / encrypt_cbc dispatch by AES key size.
    # This describes code paths in the selected build, not observed operations.
    consumers = [ref for ref, component in components.items() if component["name"] == "ibis-ooxml"]
    if len(consumers) != 1:
        raise ValueError("the OOXML AES-CBC inventory requires one ibis-ooxml component")
    consumer = consumers[0]
    dependencies = {components[ref]["name"] for ref in edges[consumer]}
    if not {"aes", "cbc"} <= dependencies:
        raise ValueError("review OOXML AES-CBC evidence after Cargo dependency changes")
    for bits in (128, 192, 256):
        name = f"AES-{bits}-CBC"
        ref = f"ibisoffice:crypto:algorithm:{name}"
        components[ref] = {
            "type": "cryptographic-asset", "bom-ref": ref, "name": name,
            "cryptoProperties": {
                "assetType": "algorithm",
                "algorithmProperties": {
                    "primitive": "block-cipher", "mode": "cbc",
                    "parameterSetIdentifier": str(bits), "cryptoFunctions": ["encrypt", "decrypt"],
                },
            },
            "properties": [
                {"name": "ibisOffice:evidence:source", "value": "crates/ibis-ooxml/src/encryption.rs"},
                {"name": "ibisOffice:evidence:usage", "value": "OOXML document encryption/decryption code"},
            ],
        }
        edges[consumer].add(ref)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("image", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    components, edges = cargo_graph()
    scope = ("Linux release build Cargo normal dependency graph for the app, updater and AppImage "
             "package; excludes build/dev dependencies, proc macros, non-Cargo payloads and system libraries")
    sbom = inventory(args.image, components, edges, scope)
    add_crypto(components, edges)
    cbom = inventory(args.image, components, edges, scope + "; curated OOXML AES-CBC source evidence only")
    args.output.mkdir(parents=True, exist_ok=True)
    for kind, data in (("sbom", sbom), ("cbom", cbom)):
        (args.output / f"linux-x64-appimage.{kind}.cdx.json").write_text(
            json.dumps(data, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
