#!/usr/bin/env python3
"""Exercise the offline Wallet process, CLI inspector and MCP server over stdio."""
import json
from pathlib import Path
import subprocess
import sys

build = Path(sys.argv[1] if len(sys.argv) > 1 else "target/debug")


def run(program, *args, input_text=None):
    return subprocess.run(
        [str(build / program), *args], input=input_text, text=True,
        capture_output=True, check=True, timeout=15,
    ).stdout


cases = [
    ("sd-jwt", "normal", [], True),
    ("sd-jwt", "extra", ["birth_date"], True),
    ("sd-jwt", "wrong-nonce", [], False),
    ("mdoc", "normal", [], None),
    ("mdoc", "extra", ["birth_date"], None),
]
for format_name, scenario, extra, nonce in cases:
    wallet = run("examples/wallet_fixture", format_name, scenario)
    args = json.loads(wallet)
    cli = json.loads(run("examples/presentation_compare", input_text=wallet))
    requests = [
        {"id": 1, "method": "initialize", "params": {"protocolVersion": "2025-11-25"}},
        {"id": 2, "method": "tools/list"},
        {"id": 3, "method": "tools/call", "params": {
            "name": "presentation_compare", "arguments": args,
        }},
    ]
    messages = "".join(json.dumps({"jsonrpc": "2.0", **req}) + "\n" for req in requests)
    _, listed, response = map(json.loads, run("ibisOffice-mcp", "--read-only", input_text=messages).splitlines())
    assert "presentation_compare" in {tool["name"] for tool in listed["result"]["tools"]}
    result = response["result"]
    assert not result["isError"], result
    assert cli == result["structuredContent"], result
    assert cli["missing_paths"] == [] and cli["unexpected_paths"] == extra, cli
    assert cli["disclosed_paths"] == ["age_over_18", *extra], cli
    assert cli["issuer_signature_valid"] is True, cli
    assert cli["disclosure_digests_verified"] == 1 + len(extra), cli
    assert cli["supplied_disclosure_digests_match"] is True, cli
    assert cli["nonce_matches"] is nonce, cli
    assert cli["issuer_trust"] == "not_checked", cli
    assert "2000-01-01" not in json.dumps(result)
    if format_name == "sd-jwt":
        assert cli["holder_signature_valid"] is True, cli
        assert cli["holder_sd_hash_valid"] is True, cli
        assert cli["audience_matches"] is True, cli
    else:
        assert cli["device_authentication"] == "not_checked", cli
    print(f"{format_name} / {scenario}: OK")

# Malformed inputs must not crash the long-running server.
for format_name, token in [
    ("mdoc", "81" * 100 + "f6"),
    ("mdoc", "5bffffffffffffffff"),
]:
    params = {"name": "presentation_compare", "arguments": {
        "format": format_name, "token": token, "expected_paths": [],
    }}
    messages = [
        {"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": params},
        {"jsonrpc": "2.0", "id": 2, "method": "ping"},
    ]
    replies = run("ibisOffice-mcp", "--read-only",
                  input_text="".join(json.dumps(message) + "\n" for message in messages))
    invalid, ping = map(json.loads, replies.splitlines())
    assert invalid["result"]["isError"], invalid
    assert ping["result"] == {}, ping
