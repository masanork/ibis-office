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


def sd_jwt_query(binding, ids):
    if not isinstance(binding, dict) or binding.keys() != {"vct", "paths"} or not isinstance(binding["paths"], dict):
        raise ValueError("The credential concept needs an sd_jwt_vc binding with vct and paths")
    vct = binding["vct"]
    if not isinstance(vct, str) or any(char.isspace() for char in vct):
        raise ValueError("This pilot requires an HTTPS vct")
    url = urlsplit(vct)
    if url.scheme != "https" or not url.hostname or url.username is not None:
        raise ValueError("This pilot requires an HTTPS vct")

    claims = []
    for logical_id in ids:
        claim_path = binding["paths"].get(logical_id)
        if claim_path is None:
            raise ValueError(f"Missing SD-JWT VC binding: {logical_id}")
        if claim_path != logical_id.split("."):
            raise ValueError(f"Binding must preserve the pilot's JSON payload path: {logical_id}")
        claims.append({"path": claim_path})
    return {"vct_values": [vct]}, claims


def mdoc_query(binding, ids, retain):
    if not isinstance(binding, dict) or binding.keys() != {"doctype", "paths"} or not isinstance(binding["paths"], dict):
        raise ValueError("The credential concept needs an mdoc binding with doctype and paths")
    doctype = binding["doctype"]
    if not isinstance(doctype, str) or not doctype or any(char.isspace() for char in doctype):
        raise ValueError("mdoc doctype must be a nonempty identifier")
    claims = []
    used = set()
    for logical_id in ids:
        path = binding["paths"].get(logical_id)
        if path is None:
            raise ValueError(f"Missing mdoc binding: {logical_id}")
        if not isinstance(path, list) or len(path) != 2 or any(not isinstance(part, str) or not part.strip() for part in path):
            raise ValueError(f"Expected an mdoc namespace and element: {logical_id}")
        element = tuple(path)
        if element in used:
            raise ValueError(f"Duplicate mdoc element: {logical_id}")
        used.add(element)
        claim = {"path": path}
        if retain is not None:
            claim["intent_to_retain"] = retain
        claims.append(claim)
    return {"doctype_value": doctype}, claims


def dcql(path, credential_format="dc+sd-jwt"):
    request = read_metadata(path, "presentation-request").get("x-ibis-presentation-request")
    required = {"version", "credential", "purpose", "claims"}
    if not isinstance(request, dict) or not required <= request.keys() <= required | {"intent_to_retain"}:
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
    mapping, definition = load_credential(credential)
    profiles = schema(mapping)["oneOf"]
    for logical_id in ids:
        if not has_path(profiles, logical_id.split(".")):
            raise ValueError(f"Unknown or non-shared logical claim: {logical_id}")
    retain = request.get("intent_to_retain")
    if "intent_to_retain" in request and type(retain) is not bool:
        raise ValueError("intent_to_retain must be a boolean")
    if credential_format == "dc+sd-jwt":
        metadata, claims = sd_jwt_query(definition.get("sd_jwt_vc"), ids)
    elif credential_format == "mso_mdoc":
        metadata, claims = mdoc_query(definition.get("mdoc"), ids, retain)
    else:
        raise ValueError(f"Unsupported credential format: {credential_format}")

    return {
        "credentials": [{
            "id": "employment",
            "format": credential_format,
            "meta": metadata,
            "require_cryptographic_holder_binding": True,
            "claims": claims,
        }]
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("request", type=Path)
    parser.add_argument("--format", choices=("dc+sd-jwt", "mso_mdoc"), default="dc+sd-jwt")
    args = parser.parse_args()
    try:
        query = dcql(args.request, args.format)
    except (OSError, ValueError) as error:
        parser.exit(1, f"{error}\n")
    print(json.dumps(query, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
