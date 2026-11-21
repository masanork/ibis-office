#!/usr/bin/env python3
"""Run the synthetic age-check Wallet through a local OpenID4VP session."""
import argparse
from http.client import HTTPConnection, HTTPException
import json
from pathlib import Path
import subprocess
import sys
from tempfile import TemporaryDirectory
from urllib.parse import urlencode, urlsplit

CAPTURE = Path(__file__).with_name("wallet-capture.py")


def wallet_response(wallet, format_name, scenario, flag, request=None):
    result = subprocess.run(
        [str(wallet), format_name, scenario, flag],
        input=json.dumps(request) if request is not None else None,
        text=True, capture_output=True, timeout=15, check=True,
    )
    return json.loads(result.stdout)


def submit(uri, response):
    url = urlsplit(uri)
    if url.scheme != "http" or url.netloc != f"127.0.0.1:{url.port}" or url.port is None:
        raise ValueError("capture did not provide a loopback response URI")
    data = urlencode({"vp_token": json.dumps(response["vp_token"]), "state": response["state"]})
    connection = HTTPConnection("127.0.0.1", url.port, timeout=10)
    try:
        connection.request("POST", url.path, body=data,
                           headers={"Content-Type": "application/x-www-form-urlencoded"})
        reply = connection.getresponse()
        if reply.status != 200:
            raise ValueError(f"capture rejected the response (HTTP {reply.status})")
        reply.read()
    finally:
        connection.close()


def round_trip(wallet, inspector, format_name, scenario):
    settings = wallet_response(wallet, format_name, "normal", "--vp-token")
    del settings["nonce"], settings["vp_token"]
    with TemporaryDirectory() as tmp:
        path = Path(tmp) / "request.json"
        path.write_text(json.dumps(settings), encoding="utf-8")
        capture = subprocess.Popen(
            [sys.executable, str(CAPTURE), str(path), "--generate-request",
             "--inspector", str(inspector), "--timeout", "30"],
            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True,
        )
        try:
            ready = json.loads(capture.stdout.readline())
            response = wallet_response(wallet, format_name, scenario, "--request",
                                       ready["authorization_request"])
            submit(ready["response_uri"], response)
            stdout, _ = capture.communicate(timeout=10)
            if capture.returncode:
                raise ValueError("capture exited without a diagnostic report")
            return json.loads(stdout)
        finally:
            if capture.poll() is None:
                capture.kill()
                capture.communicate()


def failed_checks(report, format_name):
    checks = {
        "missing_paths": report["missing_paths"] == [],
        "unexpected_paths": report["unexpected_paths"] == [],
        "credential_type": report["dcql"]["credential_type_matches"] is True,
        "claim_values": report["dcql"]["value_mismatch_paths"] == [],
        "issuer_signature": report["issuer_signature_valid"] is True,
        "disclosure_digests": report["supplied_disclosure_digests_match"] is True,
        "state": report["state_matches"] is True,
    }
    if format_name == "sd-jwt":
        checks.update(
            holder_signature=report["holder_signature_valid"] is True,
            holder_sd_hash=report["holder_sd_hash_valid"] is True,
            nonce=report["nonce_matches"] is True,
            audience=report["audience_matches"] is True,
        )
    return [name for name, passed in checks.items() if not passed]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("format", choices=("sd-jwt", "mdoc"))
    parser.add_argument("--scenario", choices=("normal", "extra", "wrong-nonce"), default="normal")
    parser.add_argument("--build-dir", type=Path, default=Path("target/debug"))
    args = parser.parse_args()
    if args.format == "mdoc" and args.scenario == "wrong-nonce":
        parser.error("wrong-nonce is an SD-JWT fixture scenario")
    try:
        examples = args.build_dir.resolve() / "examples"
        report = round_trip(examples / "wallet_fixture", examples / "presentation_compare",
                            args.format, args.scenario)
        failures = failed_checks(report, args.format)
        print(json.dumps({"passed": not failures, "failed_checks": failures, "report": report}))
        return int(bool(failures))
    except (OSError, ValueError, KeyError, subprocess.SubprocessError, HTTPException) as error:
        print(f"wallet run: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
