# GPUI Web multi-touch keyboard candidate

Local, AI-assisted review candidate against Zed commit
`6ceb6fee8738274589b59139df27efc191ec2231`. This is not an upstream PR and
has not been reviewed or approved by Zed maintainers. To inspect the two-file
change in a clean checkout of that revision, run:

```sh
git apply --unidiff-zero /path/to/keyboard.patch
```

The zero-context patch is intended only for this exact snapshot; inspect it
before applying to any other revision.

The existing Web backend keeps a tap candidate independently of GPUI's
recognizer. The candidate patch clears that candidate on a second contact, and marks the
multi-contact sequence until all contacts end or cancel. The backend also
suppresses virtual-keyboard requests during that sequence: GPUI's current
single-touch recognizer can dispatch a click on release, and an element can
request the keyboard synchronously from that click, bypassing the Web
backend's own tap-candidate check. Normal touch delivery is otherwise
unchanged.

This is deliberately scoped to the keyboard. GPUI may still synthesize an
unwanted click for a multi-contact sequence: this patch does not claim to fix
general touch arbitration. Also, suppressing *all* keyboard requests while
multiple contacts are active could suppress an intentional explicit request;
that tradeoff and any narrower API need human review. The flag remains set
until the final pointer-up or pointer-cancel has been dispatched, including
when another contact joins before the original sequence has ended.

## Synthetic browser check

Run Zed's `crates/gpui_web/examples/hello_web` input example with its documented
`trunk serve` setup, or serve an equivalent input example build with shared
memory and COOP/COEP headers. Install this checkout's npm dependencies and run:

```sh
GPUI_UPSTREAM_WEB_URL=http://127.0.0.1:8080/input \
  node docs/upstream/zed-web-multitouch-keyboard/browser-regression.mjs
```

The script uses the hidden IME textarea `inputmode` as the observation. The
example initially focuses the editor, so it explicitly sets `inputmode=none`
before each relevant gesture; this is a controlled simulation of the dismissed
state, not an observation of a real mobile keyboard. Chrome DevTools Protocol
sends synthetic touch events. The cases cover both release orders, subsequent
single-finger tap, a third contact, and cancellation followed by a new tap.

With Chrome 157.0.8103.79 on macOS, the first case failed on the unmodified
upstream example (`text` instead of `none`); all five cases passed after the
candidate patch. The upstream sources were built on 2026-11-12 with nightly
2026-10-28 and `wasm-bindgen` 0.2.130. GitHub Git fetch was unavailable, so
fixed archives were substituted for the upstream Git dependencies and Cargo
registry dependencies were resolved at build time. The source revisions are
fixed but this is not a fully locked build. No iOS or Android device or
physical keyboard behaviour was tested.

Before submitting anything upstream, a human contributor should review the
code and design, run relevant tests, and follow the current Zed contribution
and AI policy. The policy requires human understanding and does not accept
contributions from autonomous agents.
