# GPUI direct-touch pinch: upstream input-path audit

This is an investigation note for the ibis prototype, not an upstream design
agreement. Source snapshot: Zed `6ceb6fee8738274589b59139df27efc191ec2231`
(checked 2026-11-12). Links below are pinned to that revision. Findings are
from source inspection unless a test result is explicitly identified.

## Input paths

| Backend | Direct-touch path in the inspected code | Native or separate pinch path | Implication |
| --- | --- | --- | --- |
| [iOS](https://github.com/zed-industries/zed/blob/6ceb6fee8738274589b59139df27efc191ec2231/crates/gpui_ios/src/ios/window.rs#L170-L187) | Multiple UIKit touches are enabled and each `UITouch` is forwarded as `PlatformInput::Touch` ([setup](https://github.com/zed-industries/zed/blob/6ceb6fee8738274589b59139df27efc191ec2231/crates/gpui_ios/src/ios/window.rs#L345-L348), [delivery](https://github.com/zed-industries/zed/blob/6ceb6fee8738274589b59139df27efc191ec2231/crates/gpui_ios/src/ios/window.rs#L527-L548)). | No `PlatformInput::Pinch` producer was found in `gpui_ios/src` at this snapshot. Its scroll recognizer excludes finger touch; a separate edge recognizer can claim the back gesture ([setup](https://github.com/zed-industries/zed/blob/6ceb6fee8738274589b59139df27efc191ec2231/crates/gpui_ios/src/ios/window.rs#L360-L397)). | Portable pinch could consume these raw contacts. Edge-gesture cancellation and the keyboard dismissal path require separate tests. |
| [Web](https://github.com/zed-industries/zed/blob/6ceb6fee8738274589b59139df27efc191ec2231/crates/gpui_web/src/events.rs#L241-L278) | Pointer events whose type is `touch` are mapped to `PlatformInput::Touch`; canvas has `touch-action: none` ([canvas](https://github.com/zed-industries/zed/blob/6ceb6fee8738274589b59139df27efc191ec2231/crates/gpui_web/src/window.rs#L120-L126)). | No `PlatformInput::Pinch` producer was found in `gpui_web/src`; wheel input is forwarded as scroll ([wheel handler](https://github.com/zed-industries/zed/blob/6ceb6fee8738274589b59139df27efc191ec2231/crates/gpui_web/src/events.rs#L641-L673)). | Raw touch is available, but browser and OS behaviour still needs actual-device qualification. Wheel-to-pinch mapping is a separate issue. |
| [macOS](https://github.com/zed-industries/zed/blob/6ceb6fee8738274589b59139df27efc191ec2231/crates/gpui_macos/src/events.rs#L250-L268) | No `PlatformInput::Touch` producer was found in the inspected `gpui_macos/src` Rust input files. | AppKit `Magnify` becomes `PlatformInput::Pinch`. | Preserve native pinch and its existing routing; do not interpret support for native trackpad pinch as a reason to suppress a separate touchscreen stream. |
| [Windows](https://github.com/zed-industries/zed/blob/6ceb6fee8738274589b59139df27efc191ec2231/crates/gpui_windows/src/direct_manipulation.rs#L95-L105) | No `PlatformInput::Touch` producer was found in `gpui_windows/src`. This does not establish what Windows or third-party backends can support. | Direct Manipulation registers `PT_TOUCHPAD` contacts and emits pinch events ([updates](https://github.com/zed-industries/zed/blob/6ceb6fee8738274589b59139df27efc191ec2231/crates/gpui_windows/src/direct_manipulation.rs#L296-L346)). | Existing touchpad pinch is a separate native path. No conclusion about a future touchscreen implementation follows from this. |
| Linux [Wayland](https://github.com/zed-industries/zed/blob/6ceb6fee8738274589b59139df27efc191ec2231/crates/gpui_linux/src/linux/wayland/client.rs#L2728-L2798) / [X11](https://github.com/zed-industries/zed/blob/6ceb6fee8738274589b59139df27efc191ec2231/crates/gpui_linux/src/linux/x11/client.rs#L1448-L1506) | No `PlatformInput::Touch` producer was found in `gpui_linux/src`. | Both backends emit native pinch events from their respective gesture protocols. | Preserve native events. The inspected code alone does not prove what every compositor, X server, or physical input device produces. |

No GPUI Android backend was present in the inspected upstream tree. Do not
extrapolate this inventory to independently maintained GPUI mobile backends.

## Semantic questions to resolve before proposing an API

1. **Scale convention.** `PinchEvent` describes `0.1` as a 10% increase
   ([definition](https://github.com/zed-industries/zed/blob/6ceb6fee8738274589b59139df27efc191ec2231/crates/gpui/src/interactive.rs#L592-L605)).
   The ibis prototype and Windows compute `new_scale / old_scale - 1`;
   upstream Wayland and X11 currently compute `new_absolute_scale - old_absolute_scale`.
   For absolute scales `1 -> 2 -> 3`, the former yields `1, 0.5` and the latter
   `1, 1`. This is a source-level discrepancy, not proof of a user-visible bug;
   inspect consumers and test real events before choosing a contract. The
   upstream [image viewer](https://github.com/zed-industries/zed/blob/6ceb6fee8738274589b59139df27efc191ec2231/crates/image_viewer/src/image_viewer.rs#L354-L357)
   multiplies the current zoom by `1 + delta`: on this example it would
   produce factors of `4` and `3`, respectively, before zoom limits. This is
   a mathematical consequence of the code, not a physical-device observation.
   Avoid silently changing existing platform behaviour as part of an unrelated
   patch.
2. **Arbitration.** The prototype immediately cancels an active pan, long
   press, or even a claimed touch drag when a second contact starts. Decide
   whether a claimed drag retains ownership, whether a movement threshold is
   needed, and how a pinch target is chosen and retained as the midpoint moves.
   The public raw-touch dispatch contract itself is marked pending
   ([definition](https://github.com/zed-industries/zed/blob/6ceb6fee8738274589b59139df27efc191ec2231/crates/gpui/src/interactive.rs#L111-L117)).
3. **Native and raw duplication.** At this snapshot the inspected in-tree
   backends do not show one direct-touch stream producing both raw `Touch` and
   native `Pinch`. The prototype's platform-wide opt-out is therefore not yet
   justified by an observed in-tree case. Prefer an adapter contract that
   specifies a single authoritative source per direct-touch stream; investigate
   source identity or finer-grained arbitration if a mixed backend is found.
   Keep native trackpad pinch independent. Do not claim deduplication merely
   because the two event types can be dispatched in a unit test.
4. **IME and system gestures.** Web tracks a tap candidate outside the core
   recognizer and may show the keyboard on release
   ([handler](https://github.com/zed-industries/zed/blob/6ceb6fee8738274589b59139df27efc191ec2231/crates/gpui_web/src/events.rs#L256-L278),
   [release](https://github.com/zed-industries/zed/blob/6ceb6fee8738274589b59139df27efc191ec2231/crates/gpui_web/src/events.rs#L325-L370)).
   A second contact does not visibly clear this candidate in that path; verify
   whether a pinch release opens the keyboard in a real browser before calling
   it a confirmed upstream bug. iOS independently tracks a touch for downward
   keyboard dismissal ([code](https://github.com/zed-industries/zed/blob/6ceb6fee8738274589b59139df27efc191ec2231/crates/gpui_ios/src/ios/window.rs#L600-L637));
   test whether movement during a two-finger pinch can dismiss an open keyboard.
   Also test edge-back cancellation while contacts remain down.
5. **Lifecycle and ordering.** Specify `Started`, incremental `Moved`,
   `Ended` versus `Cancelled`, third contacts, coincident or non-finite
   positions, missing or reordered end events, and recovery when all contacts
   leave. UIKit provides an `NSSet` of touches and the backend iterates it
   ([code](https://github.com/zed-industries/zed/blob/6ceb6fee8738274589b59139df27efc191ec2231/crates/gpui_ios/src/ios/window.rs#L220-L224));
   simultaneous contacts must not rely on their iteration order. Confirm routing
   and behavior when focus, capture, or the window is lost.

## Evidence and next checkpoints

- In ibis, the prototype passed 39 recognizer and 26 window tests locally on
  macOS and in an iPhone simulator test process. The test process is not the
  UIKit backend or a real-device touch test. The ibis integration PR has also
  passed a synthetic Chrome touch smoke; see its live CI for remaining checks:
  [ibis-office #37](https://github.com/masanork/ibis-office/pull/37).
- Reproduce the relevant cases on a current upstream GPUI build. Add tests
  that assert actual event order, one stream per gesture, zoom delta composition,
  cancellation and post-pinch recovery; add Web keyboard and iOS edge/keyboard
  checks at their respective adapters.
- Run UIKit input in a simulator and, where available, test on physical iOS and
  Android browsers, with a record of device, OS, browser, and observed behavior.
  Desktop synthetic touch does not establish those results.
- Discuss the contract before preparing a small upstream-only patch. Zed's
  [contribution policy](https://github.com/zed-industries/zed/blob/6ceb6fee8738274589b59139df27efc191ec2231/CONTRIBUTING.md)
  asks for feature discussion first, human understanding of AI-assisted work,
  and human-authored communication; it does not accept autonomous-agent
  contributions. This document is AI-assisted investigation, not a message to
  upstream maintainers or evidence of their approval.

## Reproductions on the pinned snapshot

On 2026-11-12, the following tests were run locally on macOS. The first two
extract the indicated upstream function or expressions, verify complete source
file hashes, and execute them in small Rust harnesses with stub surroundings:

```sh
python3 scripts/audit-upstream-ios-keyboard.py "$ZED_ROOT/crates/gpui_ios/src/ios/window.rs"
python3 scripts/audit-upstream-pinch-scale.py "$ZED_ROOT"
```

- The iOS harness passed four cases. With the keyboard considered open, the
  actual `handle_keyboard_dismiss_touch` calls `hide_keyboard` when contact 1
  moves down 30 pixels while contact 2 remains down. It does not call it when
  only contact 2 moves. UIKit delivery, rendering and the actual keyboard are
  not part of this harness. The handler runs before the input callback in the
  inspected iOS backend, but the test does not establish device behaviour.
- The scale harness feeds hypothetical absolute scales 1, 2, 3 through the
  exact Wayland/X11 and Windows arithmetic and the actual image-viewer pinch
  handler, with zoom limits stubbed out. It obtains 4 and 3 respectively. It
  does not inject platform events or establish what real drivers report.
- `.github/workflows/web-fonts.yml` runs these two observations against files
  downloaded from the pinned upstream revision, not against current `main`.
  A passing observation documents existing behaviour; it does not mean the
  behaviour has been fixed or that it is necessarily a product bug.

The upstream `gpui_web` input example was also built from that Zed source and
run in Chrome 157.0.8103.79 on macOS. Since Git fetch returned HTTP 403 here,
fixed source archives were substituted for the upstream `wasm_thread`, `wgpu`
(including `naga`), and `font-kit` Git dependencies; locally cached exact
`oo7` and `scap` revisions were used. Cargo registry dependencies were resolved
at build time, so the build is not a fully locked reproduction. No GPUI input
or example source was changed. The standalone example used shared-memory WASM,
`wasm-bindgen` 0.2.130, and local COOP/COEP headers.

With the example already focused, the test explicitly sets its hidden IME
textarea's `inputmode` to `none` to model a controlled dismissed state. It then
sends two Chrome CDP contacts on the editable area, moves only the second
contact, and releases both; the first contact stays still. The pointer trace
shows two downs, one move, and two ups; after release the actual upstream Web
example sets `inputmode` to `text`. This demonstrates a browser-side state
change for that controlled synthetic sequence, not that a real mobile
keyboard opened, nor that the current upstream recognizes a pinch.

For a source checkout and running upstream gallery, start the documented
`crates/gpui_web/examples/hello_web` gallery with `trunk serve`, then run:

```sh
GPUI_UPSTREAM_WEB_URL=http://127.0.0.1:8080/input \\
  node scripts/audit-upstream-web-keyboard.mjs
```

The browser audit asserts the observed pinned behaviour. Check the example's
layout and source revision before interpreting it against another version.
The gallery build requires its documented nightly toolchain and cross-origin
isolation. Physical iOS/Android browser and UIKit gesture tests remain open.
