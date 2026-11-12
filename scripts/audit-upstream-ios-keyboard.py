#!/usr/bin/env python3
"""Exercise the pinned upstream iOS keyboard recognizer with synthetic touches.

The actual handler body is extracted from the supplied Zed checkout; the Rust
harness replaces UIKit/GPUI types and observes calls to hide_keyboard. This is
not an iOS integration or device test.
"""
import argparse
import hashlib
import pathlib
import subprocess
import tempfile

EXPECTED = "1067dfeecaa94961c05f78219bb6b53f9848db589c5b202d98dbd993e5e9d1ae"
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("window_rs", type=pathlib.Path, help="pinned Zed crates/gpui_ios/src/ios/window.rs")
args = parser.parse_args()
source = args.window_rs.read_bytes()
if hashlib.sha256(source).hexdigest() != EXPECTED:
    parser.error("window.rs differs from the audited Zed revision")
text = source.decode()
signature = "    fn handle_keyboard_dismiss_touch(&self, event: &TouchEvent) {"
start = text.index(signature)
brace = text.index("{", start)
depth = 0
for index in range(brace, len(text)):
    depth += (text[index] == "{") - (text[index] == "}")
    if depth == 0:
        handler = text[start:index + 1]
        break
else:
    raise ValueError("unclosed upstream function")

prefix = r'''
use std::cell::Cell;
use std::ops::Sub;
const KEYBOARD_DISMISS_DISTANCE: f32 = 24.;
#[derive(Clone, Copy, PartialEq, Eq)] struct TouchId(u64);
#[derive(Clone, Copy)] struct Point { x: f32, y: f32 }
impl Sub for Point { type Output = Self; fn sub(self, rhs: Self) -> Self { Self { x: self.x-rhs.x, y: self.y-rhs.y } } }
#[derive(Clone, Copy)] enum TouchPhase { Started, Moved, Ended, Cancelled }
struct TouchEvent { id: TouchId, phase: TouchPhase, position: Point }
#[derive(Clone, Copy)] struct KeyboardDismissTouch { id: TouchId, start_position: Point }
struct Window { keyboard_height: Cell<f32>, keyboard_dismiss_touch: Cell<Option<KeyboardDismissTouch>>, hidden: Cell<u32> }
impl Window {
    fn new() -> Self { Self { keyboard_height: Cell::new(200.), keyboard_dismiss_touch: Cell::new(None), hidden: Cell::new(0) } }
    fn hide_keyboard(&self) { self.hidden.set(self.hidden.get() + 1); }
'''
suffix = r'''
}
fn event(id: u64, phase: TouchPhase, x: f32, y: f32) -> TouchEvent { TouchEvent { id: TouchId(id), phase, position: Point { x, y } } }
#[test] fn single_finger_downward_swipe_dismisses() {
    let w = Window::new();
    w.handle_keyboard_dismiss_touch(&event(1, TouchPhase::Started, 0., 0.));
    w.handle_keyboard_dismiss_touch(&event(1, TouchPhase::Moved, 0., 30.));
    assert_eq!(w.hidden.get(), 1);
}
#[test] fn first_finger_can_dismiss_while_second_finger_is_down() {
    let w = Window::new();
    w.handle_keyboard_dismiss_touch(&event(1, TouchPhase::Started, 0., 0.));
    w.handle_keyboard_dismiss_touch(&event(2, TouchPhase::Started, 0., 100.));
    w.handle_keyboard_dismiss_touch(&event(1, TouchPhase::Moved, 0., 30.));
    assert_eq!(w.hidden.get(), 1, "current upstream handler dismisses during this two-contact sequence");
}
#[test] fn movement_by_second_finger_does_not_dismiss() {
    let w = Window::new();
    w.handle_keyboard_dismiss_touch(&event(1, TouchPhase::Started, 0., 0.));
    w.handle_keyboard_dismiss_touch(&event(2, TouchPhase::Started, 0., 100.));
    w.handle_keyboard_dismiss_touch(&event(2, TouchPhase::Moved, 0., 130.));
    assert_eq!(w.hidden.get(), 0);
}
#[test] fn ended_or_cancelled_first_finger_clears_candidate() {
    for phase in [TouchPhase::Ended, TouchPhase::Cancelled] {
        let w = Window::new();
        w.handle_keyboard_dismiss_touch(&event(1, TouchPhase::Started, 0., 0.));
        w.handle_keyboard_dismiss_touch(&event(1, phase, 0., 0.));
        w.handle_keyboard_dismiss_touch(&event(1, TouchPhase::Moved, 0., 30.));
        assert_eq!(w.hidden.get(), 0);
    }
}
'''
with tempfile.TemporaryDirectory(prefix="gpui-ios-keyboard-audit-") as directory:
    directory = pathlib.Path(directory)
    harness = directory / "audit.rs"
    harness.write_text(prefix + handler + suffix)
    subprocess.run(["rustc", "--edition=2024", "--test", str(harness), "-o", str(directory / "audit")], check=True)
    subprocess.run([str(directory / "audit")], check=True)
