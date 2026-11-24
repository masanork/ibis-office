#!/usr/bin/env python3
"""Check the two CycloneDX inventories against the downloaded Linux AppImage."""

import argparse
import hashlib
import json
from pathlib import Path
from urllib.request import urlopen

from jsonschema import Draft7Validator
from referencing import Registry, Resource

SCHEMA_BASE = (
    "https://raw.githubusercontent.com/CycloneDX/specification/"
    "55343ba19dee1785acf1ce9191540d5fd7b590db/schema"
)
SCHEMAS = {
    "bom-1.6.schema.json": "3e92dddbc30cf7f6a02b80f0942b1a4cfd4fb1c26f1dfc4310afa9d613cafb93",
    "jsf-0.82.schema.json": "8bae002c25e723db7ee1f26afde680ae1a2b1a8f6b4b4b0fd65dc3becb090aae",
    "spdx.schema.json": "baa9d3bd1ed57b6751b0887edead6b5063ff53ff7429cf85d476c6c94af0166e",
}


def validator(schema_dir):
    schemas = {}
    for name, digest in SCHEMAS.items():
        if schema_dir is None:
            with urlopen(f"{SCHEMA_BASE}/{name}", timeout=30) as response:
                raw = response.read()
        else:
            raw = (schema_dir / name).read_bytes()
        if hashlib.sha256(raw).hexdigest() != digest:
            raise ValueError(f"{name}: unexpected schema SHA-256")
        schemas[name] = json.loads(raw)
    registry = Registry().with_resources(
        (schema["$id"], Resource.from_contents(schema)) for schema in schemas.values()
    )
    return Draft7Validator(schemas["bom-1.6.schema.json"], registry=registry)


def verify(path, schema, source_sha, image_digest, *, crypto):
    bom = json.loads(path.read_bytes())
    error = next(schema.iter_errors(bom), None)
    if error is not None:
        raise ValueError(f"{error.json_path}: {error.message[:240]}")
    if bom["specVersion"] != "1.6":
        raise ValueError("expected CycloneDX 1.6")
    metadata = bom["metadata"]
    commits = [p["value"] for p in metadata["properties"] if p["name"] == "ibisOffice:sourceCommit"]
    if commits != [source_sha]:
        raise ValueError("source commit does not match the expected checkout")
    root = metadata["component"]
    hashes = [h["content"] for h in root["hashes"] if h["alg"] == "SHA-256"]
    if root["name"] != "ibisOffice linux-x64-appimage" or hashes != [image_digest]:
        raise ValueError("root component does not match the downloaded AppImage SHA-256")

    components = bom["components"]
    refs = [c["bom-ref"] for c in [root, *components]]
    known = set(refs)
    if len(known) != len(refs):
        raise ValueError("duplicate component bom-ref")
    crypto_refs = {c["bom-ref"] for c in components if c["type"] == "cryptographic-asset"}
    if bool(crypto_refs) != crypto:
        raise ValueError("unexpected cryptographic asset coverage")

    nodes = set()
    linked_crypto = set()
    for dependency in bom["dependencies"]:
        ref = dependency["ref"]
        if ref in nodes:
            raise ValueError(f"duplicate dependency node: {ref}")
        nodes.add(ref)
        targets = set(dependency.get("dependsOn", ())) | set(dependency.get("provides", ()))
        unknown = ({ref} | targets) - known
        if unknown:
            raise ValueError(f"unknown dependency reference: {min(unknown)}")
        linked_crypto.update(targets & crypto_refs)
    if nodes != known:
        raise ValueError(f"missing dependency node: {min(known - nodes)}")
    if linked_crypto != crypto_refs:
        raise ValueError(f"unlinked cryptographic asset: {min(crypto_refs - linked_crypto)}")
    print(f"{path.name}: validated {len(components)} components against {image_digest}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("artifact_dir", type=Path)
    parser.add_argument("source_sha")
    parser.add_argument("--schema-dir", type=Path, help="Use local copies of the pinned schemas")
    args = parser.parse_args()
    try:
        schema = validator(args.schema_dir)
        image = args.artifact_dir / "desktop-linux-x64-ci.AppImage"
        digest = hashlib.sha256()
        with image.open("rb") as stream:
            for chunk in iter(lambda: stream.read(1024 * 1024), b""):
                digest.update(chunk)
        image_digest = digest.hexdigest()
    except (OSError, ValueError) as error:
        parser.exit(1, f"{error}\n")
    for kind in ("sbom", "cbom"):
        path = args.artifact_dir / f"linux-x64-appimage.{kind}.cdx.json"
        try:
            verify(path, schema, args.source_sha, image_digest, crypto=kind == "cbom")
        except (KeyError, OSError, ValueError) as error:
            parser.exit(1, f"{path.name}: {error}\n")


if __name__ == "__main__":
    main()
