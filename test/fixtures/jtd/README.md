`synthetic.jtd` is a generated CFB container with a single synthetic
`DocumentText` stream containing the text `日本語 test`. It is not derived
from a real user's document. It tests the native import path and its
read-only behavior; it does not establish fidelity for arbitrary JTD files.

`synthetic.jttc` wraps that same synthetic JTD in a generated LH5 member.
Regenerate it with `python3 scripts/build-jttc-fixture.py`. It tests the
compressed import and read-only behavior, not compatibility with all JTTC.
