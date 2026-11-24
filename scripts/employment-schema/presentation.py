#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Compile an OKF presentation request to an OpenID4VP DCQL query."""

import argparse
import json
from pathlib import Path
from urllib.parse import urlsplit

from okf import load_credential, local_reference, read_metadata
from pilot import schema


def has_path(profiles, path):
    for profile in profiles:
        field = profile
        for key in path:
            field = field.get("properties", {}).get(key)
            if field is None:
                return False
    return True


def dcql(path):
    request = read_metadata(path, "presentation-request").get("x-ibis-presentation-request")
    if not isinstance(request, dict) or request.keys() != {"version", "credential", "purpose", "claims"}:
        raise ValueError("Expected version, credential, purpose and claims in x-ibis-presentation-request")
    if type(request["version"]) is not int or request["version"] != 1:
        raise ValueError("Unsupported x-ibis-presentation-request version")
    if not isinstance(request["purpose"], str) or not request["purpose"].strip():
        raise ValueError("A presentation purpose is required")
    ids = request["claims"]
    if not isinstance(ids, list) or not ids or any(not isinstance(item, str) or not item for item in ids):
        raise ValueError("claims must be a nonempty list of logical claim IDs")
    if len(ids) != len(set(ids)):
        raise ValueError("Duplicate logical claim IDs")

    credential = local_reference(path, request["credential"], ".md", "credential")
    mapping, binding = load_credential(credential)
    if not isinstance(binding, dict) or binding.keys() != {"vct", "paths"} or not isinstance(binding["paths"], dict):
        raise ValueError("The credential concept needs an sd_jwt_vc binding with vct and paths")
    vct = binding["vct"]
    if not isinstance(vct, str) or any(char.isspace() for char in vct):
        raise ValueError("This pilot requires an HTTPS vct")
    url = urlsplit(vct)
    if url.scheme != "https" or not url.hostname or url.username is not None:
        raise ValueError("This pilot requires an HTTPS vct")

    profiles = schema(mapping)["oneOf"]
    claims = []
    for logical_id in ids:
        logical_path = logical_id.split(".")
        if not has_path(profiles, logical_path):
            raise ValueError(f"Unknown or non-shared logical claim: {logical_id}")
        claim_path = binding["paths"].get(logical_id)
        if claim_path is None:
            raise ValueError(f"Missing SD-JWT VC binding: {logical_id}")
        if claim_path != logical_path:
            raise ValueError(f"Binding must preserve the pilot's JSON payload path: {logical_id}")
        claims.append({"path": claim_path})

    return {
        "credentials": [{
            "id": "employment",
            "format": "dc+sd-jwt",
            "meta": {"vct_values": [vct]},
            "require_cryptographic_holder_binding": True,
            "claims": claims,
        }]
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("request", type=Path)
    args = parser.parse_args()
    try:
        query = dcql(args.request)
    except (OSError, ValueError) as error:
        parser.exit(1, f"{error}\n")
    print(json.dumps(query, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
