#!/usr/bin/env python3
"""Exercise the offline Wallet process, CLI inspector and MCP server over stdio."""
from contextlib import contextmanager
import json
from pathlib import Path
import subprocess
import sys
from tempfile import TemporaryDirectory
from urllib.error import HTTPError
from urllib.request import Request, urlopen
from urllib.parse import urlencode

build = Path(sys.argv[1] if len(sys.argv) > 1 else "target/debug")


def run(program, *args, input_text=None):
    return subprocess.run(
        [str(build / program), *args], input=input_text, text=True,
        capture_output=True, check=True, timeout=15,
    ).stdout


def mcp_compare(args):
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
    return result


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
    result = mcp_compare(args)
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

for format_name in ("sd-jwt", "mdoc"):
    args = json.loads(run("examples/wallet_fixture", format_name, "extra"))
    del args["expected_paths"]
    mdoc = format_name == "mdoc"
    ns = "org.iso.18013.5.1"
    path = [ns, "age_over_18"] if mdoc else ["age_over_18"]
    prefix = f"/{ns}/" if mdoc else ""
    meta = {"doctype_value": f"{ns}.mDL"} if mdoc else {
        "vct_values": ["https://credentials.example.com/identity_credential"],
    }
    claim = {"path": path, "values": [True]}
    credential = {"id": "age", "format": "mso_mdoc" if mdoc else "dc+sd-jwt",
                  "meta": meta, "claims": [claim]}
    args["dcql_query"] = {"credentials": [credential]}
    result = mcp_compare(args)
    assert not result["isError"], result
    report = result["structuredContent"]
    cli = json.loads(run("examples/presentation_compare", input_text=json.dumps(args)))
    assert cli == report, report
    assert report["dcql"] == {"credential_type_matches": True, "value_mismatch_paths": []}, report
    assert report["missing_paths"] == [] and report["unexpected_paths"] == [prefix + "birth_date"], report

    claim["values"] = [False]
    report = mcp_compare(args)["structuredContent"]
    assert report["dcql"]["value_mismatch_paths"] == [prefix + "age_over_18"], report
    if mdoc:
        claim["path"] = ["another.namespace", "age_over_18"]
        report = mcp_compare(args)["structuredContent"]
        assert report["missing_paths"] == ["/another.namespace/age_over_18"], report
        assert report["unexpected_paths"] == [prefix + "age_over_18", prefix + "birth_date"], report
    else:
        claim["path"] = ["unknown_claim"]
        report = mcp_compare(args)["structuredContent"]
        assert report["missing_paths"] == ["unknown_claim"], report

    credential["meta"] = {"doctype_value": "another.type"} if mdoc else {"vct_values": ["urn:example:other"]}
    report = mcp_compare(args)["structuredContent"]
    assert report["dcql"]["credential_type_matches"] is False, report
    credential["claim_sets"] = [["some-claim"]]
    result = mcp_compare(args)
    assert result["isError"], result
    print(f"{format_name} / DCQL type, values, paths and unsupported query: OK")

for format_name, scenario, expected_nonce in [
    ("sd-jwt", "normal", True),
    ("sd-jwt", "wrong-nonce", False),
    ("mdoc", "normal", None),
]:
    args = json.loads(run("examples/wallet_fixture", format_name, scenario, "--vp-token"))
    result = mcp_compare(args)
    assert not result["isError"], result
    report = result["structuredContent"]
    assert report == json.loads(run("examples/presentation_compare", input_text=json.dumps(args))), report
    assert report["vp_token_credential_id"] == "age_check", report
    assert report["missing_paths"] == report["unexpected_paths"] == [], report
    assert report["dcql"] == {"credential_type_matches": True, "value_mismatch_paths": []}, report
    assert report["nonce_matches"] is expected_nonce, report
    token = args["vp_token"]["age_check"][0]
    raw_args = {key: value for key, value in args.items() if key != "vp_token"}
    raw_args["token"] = token
    raw = mcp_compare(raw_args)["structuredContent"]
    assert {**raw, "vp_token_credential_id": "age_check"} == report, raw
    for malformed, expected_error in [
        ({"wrong_id": [token]}, "credential id does not match"),
        ({}, "one credential id"),
        ({"age_check": [token], "other": [token]}, "one credential id"),
        ({"age_check": token}, "one presentation string"),
        ({"age_check": []}, "one presentation string"),
        ({"age_check": [token, token]}, "one presentation string"),
        ({"age_check": [None]}, "one presentation string"),
    ]:
        result = mcp_compare({**args, "vp_token": malformed})
        assert result["isError"], result
        assert expected_error in result["structuredContent"]["error"]["message"], result
    assert mcp_compare({**args, "token": token})["isError"]
    assert mcp_compare({**args, "expected_paths": ["age_over_18"]})["isError"]
    print(f"{format_name} / {scenario} / OpenID4VP envelope and invalid responses: OK")

for format_name in ("sd-jwt", "mdoc"):
    args = json.loads(run("examples/wallet_fixture", format_name, "normal", "--vp-token"))
    envelope = mcp_compare(args)["structuredContent"]
    vp_token = json.dumps(args.pop("vp_token"))
    state = "request + 検査&="
    body = urlencode({"vp_token": vp_token, "state": state})
    args["direct_post_body"] = body
    args["expected_state"] = state
    result = mcp_compare(args)
    assert not result["isError"], result
    report = result["structuredContent"]
    assert report == {**envelope, "state_matches": True}, report
    assert report == json.loads(run("examples/presentation_compare", input_text=json.dumps(args))), report
    assert state not in json.dumps(report, ensure_ascii=False)
    for next_body in (
        urlencode({"vp_token": vp_token, "state": "another request"}),
        urlencode({"vp_token": vp_token}),
    ):
        report = mcp_compare({**args, "direct_post_body": next_body})["structuredContent"]
        assert report["state_matches"] is False, report
        assert report["nonce_matches"] == envelope["nonce_matches"], report
    for bad_body in (
        body + "&%73tate=duplicate",
        body + "&vp_token=duplicate",
        body + "&unknown",
        body.replace("state=", "state=%ZZ"),
        body.replace("state=", "state=%FF"),
        "vp_token=invalid-json",
        "response=encrypted",
        "error=access_denied",
        "state=without-presentation",
    ):
        result = mcp_compare({**args, "direct_post_body": bad_body})
        assert result["isError"], result
    assert mcp_compare({**args, "vp_token": json.loads(vp_token)})["isError"]
    print(f"{format_name} / direct_post form and state: OK")

@contextmanager
def start_capture(settings, *flags):
    with TemporaryDirectory() as tmp:
        config = Path(tmp) / "request.json"
        config.write_text(json.dumps(settings), encoding="utf-8")
        capture = subprocess.Popen(
            [sys.executable, "scripts/wallet-capture.py", str(config),
             "--inspector", str(build / "examples/presentation_compare"),
             "--timeout", "15", *flags],
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
        )
        try:
            ready = json.loads(capture.stdout.readline())
            assert ready["response_uri"].startswith("http://127.0.0.1:"), ready
            yield capture, ready
        finally:
            if capture.poll() is None:
                capture.kill()
                capture.communicate()


def post(target, data, media_type="application/x-www-form-urlencoded"):
    request = Request(target, data=data.encode(), headers={"Content-Type": media_type})
    try:
        with urlopen(request, timeout=5) as reply:
            return reply.status, reply.read()
    except HTTPError as error:
        with error:
            return error.code, error.read()


def capture_report(capture):
    stdout, stderr = capture.communicate(timeout=5)
    assert capture.returncode == 0 and not stderr, stderr
    return json.loads(stdout)


for format_name, scenario, nonce_matches in (
    ("sd-jwt", "wrong-nonce", False),
    ("mdoc", "extra", None),
):
    wallet = json.loads(run("examples/wallet_fixture", format_name, scenario, "--vp-token"))
    state = "http-session"
    body = urlencode({"vp_token": json.dumps(wallet.pop("vp_token")), "state": state})
    wallet["expected_state"] = state
    expected = mcp_compare({**wallet, "direct_post_body": body})["structuredContent"]
    assert expected["state_matches"] is True and expected["nonce_matches"] is nonce_matches, expected
    with start_capture(wallet) as (capture, ready):
        url = ready["response_uri"]
        assert post(url + "/wrong", body) == (404, b"{}")
        assert post(url, body, "application/json") == (415, b"{}")
        assert post(url, "vp_token=invalid") == (400, b"{}")
        assert post(url, body) == (200, b"{}")
        report = capture_report(capture)
        assert report == expected, report
        assert "2000-01-01" not in json.dumps(report) and state not in json.dumps(report)
    print(f"{format_name} / loopback HTTP capture: OK")


for format_name in ("sd-jwt", "mdoc"):
    settings = json.loads(run("examples/wallet_fixture", format_name, "normal", "--vp-token"))
    del settings["vp_token"], settings["nonce"]
    settings["dcql_query"]["credentials"][0]["id"] = "custom-age-check"
    settings["audience"] = "https://local-test.example"
    with start_capture(settings, "--generate-request") as (capture, ready):
        auth = ready["authorization_request"]
        assert auth["response_uri"] == ready["response_uri"]
        assert auth["client_id"] == settings["audience"]
        response = json.loads(run("examples/wallet_fixture", format_name, "normal", "--request",
                                  input_text=json.dumps(auth)))
        assert response["state"] == auth["state"]
        assert set(response["vp_token"]) == {"custom-age-check"}
        body = urlencode({"vp_token": json.dumps(response["vp_token"]), "state": response["state"]})
        assert post(ready["response_uri"], body) == (200, b"{}")
        report = capture_report(capture)
        assert report["state_matches"] is True, report
        assert report["nonce_matches"] is (True if format_name == "sd-jwt" else None), report
        assert report["audience_matches"] is (True if format_name == "sd-jwt" else None), report
        assert report["missing_paths"] == report["unexpected_paths"] == [], report
    with start_capture(settings, "--generate-request") as (capture, next_ready):
        next_auth = next_ready["authorization_request"]
        assert auth["nonce"] != next_auth["nonce"] and auth["state"] != next_auth["state"]
        assert post(next_ready["response_uri"], body) == (200, b"{}")
        report = capture_report(capture)
        assert report["state_matches"] is False, report
        assert report["nonce_matches"] is (False if format_name == "sd-jwt" else None), report
    print(f"{format_name} / generated request and cross-session replay diagnostics: OK")
