# SPDX-License-Identifier: AGPL-3.0-only
# Copyright (c) ibis inc.
# Additional terms under AGPL-3.0 section 7(b): see NOTICE.
"""Compare synthetic certificate signatures with local openssl at host time."""

import subprocess
import tempfile
from datetime import datetime, timezone
from pathlib import Path

DATA = Path(__file__).resolve().parents[2] / "crates/ibis-ooxml/tests/fixtures/signatures"
CASES = [
    ("p256-sha256", "ec-leaf", "ec-ca", True),
    ("p384-sha384", "p384-normal-leaf", "p384-ca", True),
    ("p384-sha256", "p384-sha256-leaf", "p384-ca", True),
    ("pss-sha256-salt32", "pss-normal-leaf", "pss-ca", True),
    ("pss-salt20", "pss-salt20-leaf", "pss-ca", True),
    ("pss-mgf384", "pss-mgf384-leaf", "pss-ca", True),
    ("pss-other-issuer", "pss-normal-leaf", "pss-other", False),
    ("pss-damaged", "pss-damaged", "pss-ca", False),
]


def main():
    print(subprocess.check_output(["openssl", "version"], text=True).strip())
    now = datetime.now(timezone.utc)
    print("Reference verifier uses host time:", now.isoformat())
    if not datetime(2026, 11, 15, 1, 22, 6, tzinfo=timezone.utc) <= now <= datetime(2027, 11, 15, 1, 22, 6, tzinfo=timezone.utc):
        raise SystemExit("Host time is outside the common validity window of the test certificates")
    with tempfile.TemporaryDirectory(prefix="ibis-algorithm-reference-") as temp:
        pem_dir = Path(temp)
        names = {item for _, leaf, ca, _ in CASES for item in (leaf, ca)}
        for name in sorted(names):
            der = bytearray((DATA / ("pss-normal-leaf.der" if name == "pss-damaged" else f"{name}.der")).read_bytes())
            if name == "pss-damaged":
                der[-1] ^= 1
            pem = subprocess.run(["openssl", "x509", "-inform", "DER"], input=der, stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=True).stdout
            (pem_dir / f"{name}.pem").write_bytes(pem)
        for label, leaf, ca, expected in CASES:
            result = subprocess.run(["openssl", "verify", "-CAfile", str(pem_dir / f"{ca}.pem"), str(pem_dir / f"{leaf}.pem")], stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
            accepted = result.returncode == 0
            print(f"{label}: {'accepted' if accepted else 'rejected'}")
            if accepted != expected or (not expected and "certificate signature failure" not in result.stdout.lower()):
                raise SystemExit(f"unexpected result for {label}: {result.stdout}")


if __name__ == "__main__":
    main()
