# SPDX-License-Identifier: AGPL-3.0-only
# Copyright (c) ibis inc.
# Additional terms under AGPL-3.0 section 7(b): see NOTICE.
"""Compare policy cases with the local openssl verifier at its current time."""

import subprocess
import tempfile
from datetime import datetime, timezone
from pathlib import Path

DATA = Path(__file__).parent / "data"
CASES = [
    ("matching-explicit", "a", "a", ["-explicit_policy"], True),
    ("mismatch-explicit", "a", "b", ["-explicit_policy"], False),
    ("mismatch-not-explicit", "a", "b", [], True),
    ("absent-explicit", "a", "absent", ["-explicit_policy"], False),
    ("absent-not-explicit", "a", "absent", [], True),
    ("any-allowed", "a", "any", ["-explicit_policy"], True),
    ("any-inhibited", "a", "any", ["-explicit_policy", "-inhibit_any"], False),
    ("mapping-allowed", "mapped", "b", ["-explicit_policy"], True),
    ("mapping-inhibited", "mapped", "b", ["-explicit_policy", "-inhibit_map"], False),
]


def main():
    print(subprocess.check_output(["openssl", "version"], text=True).strip())
    now = datetime.now(timezone.utc)
    print("Reference verifier uses host time:", now.isoformat())
    if not datetime(2026, 1, 1, tzinfo=timezone.utc) <= now <= datetime(2029, 1, 1, tzinfo=timezone.utc):
        raise SystemExit("Host time is outside the synthetic certificates' validity period")
    with tempfile.TemporaryDirectory(prefix="ibis-policy-reference-") as temp:
        pem_dir = Path(temp)
        names = ["policy-root", "policy-intermediate-a", "policy-intermediate-mapped"]
        names += [f"policy-leaf-{suffix}" for suffix in ("a", "b", "any", "absent")]
        for name in names:
            pem = subprocess.check_output(
                ["openssl", "x509", "-inform", "DER", "-in", str(DATA / f"{name}.der")]
            )
            (pem_dir / f"{name}.pem").write_bytes(pem)
        for label, issuer, leaf, options, expected in CASES:
            args = [
                "openssl", "verify", "-CAfile", str(pem_dir / "policy-root.pem"),
                "-untrusted", str(pem_dir / f"policy-intermediate-{issuer}.pem"),
                "-policy", "1.2.3.4", *options, str(pem_dir / f"policy-leaf-{leaf}.pem"),
            ]
            result = subprocess.run(args, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
            passed = result.returncode == 0
            print(f"{label}: {'accepted' if passed else 'rejected'}")
            if passed != expected or (not expected and "no explicit policy" not in result.stdout.lower()):
                raise SystemExit(f"unexpected result for {label}: {result.stdout}")


if __name__ == "__main__":
    main()
