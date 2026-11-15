#!/usr/bin/env python3
"""Compare selected synthetic fixtures with the local openssl CLI at a fixed time."""
from pathlib import Path
import subprocess
import tempfile

DATA = Path(__file__).resolve().parent / "data"
TIME = "1796083200"  # 2026-12-01 00:00:00 UTC

def run(*args):
    return subprocess.run(args, text=True, capture_output=True, check=False)

with tempfile.TemporaryDirectory() as tmp:
    tmp = Path(tmp)
    for name in ("root", "leaf"):
        r = run("openssl", "x509", "-inform", "DER", "-in", str(DATA / f"{name}.der"), "-out", str(tmp / f"{name}.pem"))
        if r.returncode:
            raise SystemExit(r.stderr)
    # Error text is checked, not just exit status, so setup failures do not pass.
    cases = {
        "empty": (0, "OK"),
        "revoked": (1, "certificate revoked"),
        "wrong-key": (1, "CRL signature failure"),
        "expired": (1, "CRL has expired"),
        "future": (1, "CRL is not yet valid"),
        "critical-unknown": (1, "unhandled critical CRL extension"),
        "critical-number": (1, "unhandled critical CRL extension"),
    }
    for name, (failure, expected) in cases.items():
        r = run("openssl", "crl", "-inform", "DER", "-in", str(DATA / f"{name}.der"), "-out", str(tmp / "crl.pem"))
        if r.returncode:
            raise SystemExit(r.stderr)
        r = run("openssl", "verify", "-attime", TIME, "-crl_check", "-CAfile", str(tmp / "root.pem"), "-CRLfile", str(tmp / "crl.pem"), str(tmp / "leaf.pem"))
        output = r.stdout + r.stderr
        if (r.returncode != 0) != bool(failure) or expected not in output:
            raise SystemExit(f"{name}: unexpected result: {output}")
        print(f"{name}: {expected}")
