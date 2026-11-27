#!/usr/bin/env python3
"""Check CycloneDX inventories against a downloaded ibisOffice build."""

import argparse
import configparser
import hashlib
import json
import subprocess
from pathlib import Path, PurePosixPath
from tempfile import TemporaryDirectory, TemporaryFile
from urllib.parse import quote, urlsplit
from urllib.request import urlopen
from zipfile import BadZipFile, ZipFile

from bom_payload import file_components, sha256, stream_sha256, zip_components

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


PACKAGES = {
    "linux-x64-appimage": "desktop-linux-x64-ci.AppImage",
    "macos-arm64": "desktop-macos-arm64-ci.zip",
    "windows-x64": "desktop-windows-x64-ci.zip",
    "android-arm64": "desktop-android-arm64-ci.apk",
    "ios-arm64": "desktop-ios-arm64-ci.zip",
    "wasm": "wasm-ci.zip",
}
FLATPAK_ARCH = {"flatpak-x64": "x86_64", "flatpak-arm64": "aarch64"}
REQUIRED_PAYLOAD = {
    "macos-arm64": {"Docs.app/Contents/MacOS/ibisOffice", "Docs.app/Contents/MacOS/ibisOfficeUpdater"},
    "windows-x64": {"ibisOffice/ibisOffice.exe", "ibisOffice/ibisOfficeUpdater.exe"},
    "android-arm64": {"AndroidManifest.xml", "classes.dex", "lib/arm64-v8a/libibis_office_app.so"},
    "ios-arm64": {"IbisOffice.app/Info.plist", "IbisOffice.app/IbisOffice"},
    "wasm": {"pkg/ibis_office_app.js", "pkg/ibis_office_app_bg.manifest.json",
             "pkg/heic/ibis_heic_web.js", "pkg/heic/ibis_heic_web_bg.wasm"},
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


def flatpak_package(directory, arch):
    matches = sorted(directory.glob(f"ibisOffice-*-Linux-{arch}.flatpak"))
    if len(matches) != 1:
        raise ValueError(f"expected one Flatpak bundle for {arch}, found {len(matches)}")
    return matches[0]


def flatpak_components(package, arch):
    with TemporaryDirectory() as work:
        repo = f"--repo={work}/repo"
        checkout = Path(work) / "checkout"
        subprocess.run(["ostree", repo, "init", "--mode=archive"], check=True)
        subprocess.run(["flatpak", "build-import-bundle", "--no-update-summary",
                        f"{work}/repo", str(package)], check=True)
        refs = subprocess.check_output(["ostree", repo, "refs"], text=True).splitlines()
        ref_parts = refs[0].split("/") if len(refs) == 1 else []
        if len(ref_parts) != 4 or ref_parts[0] != "app" or ref_parts[2] != arch:
            raise ValueError(f"unexpected Flatpak application ref for {arch}: {refs}")
        subprocess.run(["ostree", repo, "checkout", "--user-mode", refs[0], str(checkout)], check=True)
        metadata = configparser.ConfigParser(interpolation=None)
        metadata.read(checkout / "metadata")
        app = metadata["Application"]
        if app["name"] != ref_parts[1]:
            raise ValueError("Flatpak application metadata does not match the ref")
        runtime = app["runtime"]
        print(f"{refs[0]}: declared runtime {runtime}")
        components = {c["bom-ref"]: c for _, c in file_components(checkout / "files", "flatpak")}
        binary = components.get("ibisoffice:flatpak:file:bin/ibisOffice", {})
        if "hashes" not in binary:
            raise ValueError("missing Flatpak ibisOffice binary")
        return components


def verify_wasm(archive):
    prefix = "pkg/"
    with ZipFile(archive) as zipped:
        manifest = json.loads(zipped.read(prefix + "ibis_office_app_bg.manifest.json"))
        digest = hashlib.sha256()
        size = 0
        for chunk in manifest["chunks"]:
            name = f"ibis_office_app_bg.{chunk['sha256']}.chunk"
            if chunk["file"] != name:
                raise ValueError("unexpected WASM chunk filename")
            data = zipped.read(prefix + name)
            if len(data) != chunk["size"] or hashlib.sha256(data).hexdigest() != chunk["sha256"]:
                raise ValueError(f"WASM chunk does not match the manifest: {name}")
            digest.update(data)
            size += len(data)
            if "br" in chunk:
                br = chunk["br"]
                br_name = f"wasm-br.{br['sha256']}.bin"
                br_data = zipped.read(prefix + br_name)
                if len(br_data) != br["size"] or hashlib.sha256(br_data).hexdigest() != br["sha256"]:
                    raise ValueError(f"compressed WASM chunk does not match the manifest: {br_name}")
        if not size or size != manifest["size"] or digest.hexdigest() != manifest["sha256"]:
            raise ValueError("assembled WASM does not match the manifest")
        if "single" in manifest:
            single = manifest["single"]
            name = f"wasm-br.{single['sha256']}.bin"
            data = zipped.read(prefix + name)
            if len(data) != single["size"] or hashlib.sha256(data).hexdigest() != single["sha256"]:
                raise ValueError("compressed WASM does not match the manifest")


def verify_public_web(base_url, payload):
    parts = urlsplit(base_url)
    if parts.scheme != "https" or not parts.netloc or parts.query or parts.fragment:
        raise ValueError("public Web base must be an HTTPS URL without a query or fragment")
    manifest = "pkg/ibis_office_app_bg.manifest.json"
    # Start with the manifest to detect the wrong deployment before fetching modules.
    components = sorted(payload.values(), key=lambda c: (c["name"] != manifest, c["name"]))
    for component in components:
        name = component["name"]
        path = PurePosixPath(name)
        if not path.parts or path.parts[0] != "pkg" or ".." in path.parts or path.as_posix() != name:
            raise ValueError(f"unexpected browser package path: {name}")
        if "hashes" not in component:
            raise ValueError(f"expected a regular browser asset: {name}")
        url = base_url.rstrip("/") + "/" + quote(name, safe="/")
        with TemporaryFile() as response:
            result = subprocess.run(
                ["curl", "--disable", "--fail", "--silent", "--show-error", "--location",
                 "--proto", "=https", "--proto-redir", "=https", "--max-time", "45", "--url", url],
                stdout=response,
            )
            if result.returncode:
                raise ValueError(f"could not download published asset: {name}")
            response.seek(0)
            observed = stream_sha256(response)
        expected = component["hashes"][0]["content"]
        if observed != expected:
            raise ValueError(f"published asset differs from CI: {name} (expected {expected}, received {observed})")
    print(f"{base_url}: {len(components)} published browser assets match CI")


def verify(path, schema, platform, source_sha, image_digest, payload, *, crypto):
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
    if root["name"] != f"ibisOffice {platform}" or root["bom-ref"] != f"ibisoffice:{platform}":
        raise ValueError("unexpected package identity")
    if hashes != [image_digest]:
        raise ValueError("root component does not match the downloaded package SHA-256")

    components = bom["components"]
    refs = [c["bom-ref"] for c in [root, *components]]
    known = set(refs)
    if len(known) != len(refs):
        raise ValueError("duplicate component bom-ref")
    crypto_refs = {c["bom-ref"] for c in components if c["type"] == "cryptographic-asset"}
    if bool(crypto_refs) != crypto:
        raise ValueError("unexpected cryptographic asset coverage")

    if payload is not None:
        namespace = "flatpak" if platform in FLATPAK_ARCH else "archive"
        prefix = f"ibisoffice:{namespace}:file:"
        observed = {c["bom-ref"]: c for c in components if c["bom-ref"].startswith(prefix)}
        if observed != payload:
            differences = {ref for ref in observed.keys() | payload.keys() if observed.get(ref) != payload.get(ref)}
            raise ValueError(f"package payload differs from BOM: {min(differences)}")

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
        if platform in REQUIRED_PAYLOAD and ref == root["bom-ref"] and not payload.keys() <= targets:
            raise ValueError("archive payload is not linked to the package")
        linked_crypto.update(targets & crypto_refs)
    if nodes != known:
        raise ValueError(f"missing dependency node: {min(known - nodes)}")
    if linked_crypto != crypto_refs:
        raise ValueError(f"unlinked cryptographic asset: {min(crypto_refs - linked_crypto)}")
    print(f"{path.name}: validated {len(components)} components against {image_digest}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("platform", choices=[*PACKAGES, *FLATPAK_ARCH])
    parser.add_argument("artifact_dir", type=Path)
    parser.add_argument("source_sha")
    parser.add_argument("--schema-dir", type=Path, help="Use local copies of the pinned schemas")
    parser.add_argument("--public-web-base", help="Verify deployed WASM package files from an HTTPS site")
    args = parser.parse_args()
    if args.public_web_base and args.platform != "wasm":
        parser.error("--public-web-base requires the wasm platform")
    try:
        schema = validator(args.schema_dir)
        arch = FLATPAK_ARCH.get(args.platform)
        if arch:
            image = flatpak_package(args.artifact_dir, arch)
            image_digest = sha256(image)
            payload = flatpak_components(image.resolve(), arch)
        else:
            image = args.artifact_dir / PACKAGES[args.platform]
            image_digest = sha256(image)
            payload = None
        if args.platform in REQUIRED_PAYLOAD:
            payload = zip_components(image)
            files = {c["name"] for c in payload.values() if "hashes" in c}
            missing = REQUIRED_PAYLOAD[args.platform] - files
            if missing:
                raise ValueError(f"missing required package file: {min(missing)}")
            if args.platform == "wasm":
                verify_wasm(image)
    except (BadZipFile, KeyError, OSError, ValueError, configparser.Error, subprocess.CalledProcessError) as error:
        parser.exit(1, f"{error}\n")
    for kind in ("sbom", "cbom"):
        path = args.artifact_dir / f"{args.platform}.{kind}.cdx.json"
        try:
            verify(path, schema, args.platform, args.source_sha, image_digest, payload, crypto=kind == "cbom")
        except (KeyError, OSError, ValueError) as error:
            parser.exit(1, f"{path.name}: {error}\n")

    if args.public_web_base:
        try:
            verify_public_web(args.public_web_base, payload)
        except (OSError, ValueError) as error:
            parser.exit(1, f"{error}\n")


if __name__ == "__main__":
    main()
