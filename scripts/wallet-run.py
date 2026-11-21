#!/usr/bin/env python3
"""Run a command-line Wallet through a local OpenID4VP session."""
import argparse
from http.client import HTTPConnection, HTTPException
import json
from pathlib import Path
import subprocess
import sys
from tempfile import TemporaryDirectory
from urllib.parse import urlencode, urlsplit

CAPTURE = Path(__file__).with_name("wallet-capture.py")


def wallet_response(command, request=None):
    result = subprocess.run(
        command,
        input=json.dumps(request) if request is not None else "",
        text=True, encoding="utf-8", capture_output=True, timeout=15, check=False,
    )
    if result.returncode:
        raise ValueError(f"Wallet command exited with code {result.returncode}")
    try:
        response = json.loads(result.stdout)
    except (UnicodeError, ValueError):
        raise ValueError("Wallet command must write one JSON object to stdout") from None
    if not isinstance(response, dict):
        raise ValueError("Wallet command must write one JSON object to stdout")
    return response


def request_settings(path, format_name):
    settings = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(settings, dict) or settings.keys() - {"format", "audience", "dcql_query", "issuer_jwk"}:
        raise ValueError("settings accept only format, audience, dcql_query and issuer_jwk")
    if settings.setdefault("format", format_name) != format_name:
        raise ValueError("settings format does not match the selected format")
    if not isinstance(settings.get("audience"), str) or not settings["audience"].strip():
        raise ValueError("settings require the verifier audience")
    if not isinstance(settings.get("dcql_query"), dict):
        raise ValueError("settings require a dcql_query object")
    jwk = settings.get("issuer_jwk")
    if not isinstance(jwk, dict) or "d" in jwk:
        raise ValueError("settings require a public issuer_jwk object")
    return settings


def submit(uri, response):
    url = urlsplit(uri)
    if url.scheme != "http" or url.netloc != f"127.0.0.1:{url.port}" or url.port is None:
        raise ValueError("capture did not provide a loopback response URI")
    if set(response) != {"vp_token", "state"} or not isinstance(response["vp_token"], dict) \
            or not isinstance(response["state"], str):
        raise ValueError("Wallet response requires a vp_token object and a state string")
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


def round_trip(command, inspector, settings):
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
            response = wallet_response(command,
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
    parser.add_argument("--scenario", choices=("normal", "extra", "wrong-nonce"))
    parser.add_argument("--build-dir", type=Path, default=Path("target/debug"))
    parser.add_argument("--request-settings", type=Path, help="JSON verifier settings for an external Wallet")
    parser.add_argument("--wallet-command", nargs=argparse.REMAINDER, metavar="ARG",
                        help="external command and arguments (must come last)")
    args = parser.parse_args()
    if args.wallet_command == [] or bool(args.wallet_command) != bool(args.request_settings):
        parser.error("--request-settings and a nonempty --wallet-command must be supplied together")
    if args.wallet_command and args.scenario:
        parser.error("--scenario is only for the synthetic Wallet")
    if args.format == "mdoc" and args.scenario == "wrong-nonce":
        parser.error("wrong-nonce is an SD-JWT fixture scenario")
    try:
        examples = args.build_dir.resolve() / "examples"
        if args.wallet_command:
            command = args.wallet_command
            settings = request_settings(args.request_settings, args.format)
        else:
            wallet = str(examples / "wallet_fixture")
            settings = wallet_response([wallet, args.format, "normal", "--vp-token"])
            del settings["nonce"], settings["vp_token"]
            command = [wallet, args.format, args.scenario or "normal", "--request"]
        report = round_trip(command, examples / "presentation_compare", settings)
        failures = failed_checks(report, args.format)
        print(json.dumps({"passed": not failures, "failed_checks": failures, "report": report}))
        return int(bool(failures))
    except (OSError, ValueError, KeyError, subprocess.SubprocessError, HTTPException) as error:
        print(f"wallet run: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
