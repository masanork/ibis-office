# SPDX-License-Identifier: AGPL-3.0-only
"""Read package payload evidence without extracting archive paths."""

import hashlib
import stat
from urllib.parse import quote
import zipfile


def stream_sha256(stream):
    digest = hashlib.sha256()
    for chunk in iter(lambda: stream.read(1024 * 1024), b""):
        digest.update(chunk)
    return digest.hexdigest()


def sha256(path):
    with path.open("rb") as stream:
        return stream_sha256(stream)


def zip_components(archive):
    components = {}
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

    return components
