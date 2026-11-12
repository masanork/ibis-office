#!/usr/bin/env python3
"""Run pinned upstream pinch formulas through the actual image-viewer handler.

Platform and image-viewer surroundings are lightweight stubs; this does not
execute Wayland, X11, Direct Manipulation, or the actual zoom limits.
"""
import argparse
import hashlib
import pathlib
import subprocess
import tempfile

FILES = {
    "crates/gpui_linux/src/linux/wayland/client.rs": "8f8467b709a316fade5071204864af540598cbb226a1fc7d24ff6261c756366b",
    "crates/gpui_linux/src/linux/x11/client.rs": "9312432f6a5cea2e816dea192395faf92fe8e88bc52d35e0eb13d090283fc6b4",
    "crates/gpui_windows/src/direct_manipulation.rs": "df30e9866c51ed03643fbe9c422837298e8fd5741b2f1b005f575505ae4e4acd",
    "crates/image_viewer/src/image_viewer.rs": "208d6d9d4bdf9b5d0215bbf88703d4f45edb18d48f68450ee49af89bd1228040",
}
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("zed_root", type=pathlib.Path)
args = parser.parse_args()
source = {}
for name, expected in FILES.items():
    data = (args.zed_root / name).read_bytes()
    if hashlib.sha256(data).hexdigest() != expected:
        parser.error(f"{name} differs from the audited Zed revision")
    source[name] = data.decode()
wayland = source[next(p for p in FILES if "wayland" in p)]
x11 = source[next(p for p in FILES if "x11" in p)]
windows = source[next(p for p in FILES if "windows" in p)]
viewer = source[next(p for p in FILES if "image_viewer" in p)]
linux_line = "let zoom_delta = new_absolute_scale - previous_scale;"
windows_line = "let scale_delta = scale / last_scale;"
windows_field = "delta: scale_delta - 1.0,"
for haystack, needle in [(wayland, linux_line), (x11, linux_line), (windows, windows_line), (windows, windows_field)]:
    if haystack.count(needle) != 1:
        parser.error(f"expected exactly one {needle!r}")
sig = "    fn handle_pinch(&mut self, event: &PinchEvent, _window: &mut Window, cx: &mut Context<Self>) {"
start = viewer.index(sig)
brace = viewer.index("{", start)
depth = 0
for index in range(brace, len(viewer)):
    depth += (viewer[index] == "{") - (viewer[index] == "}")
    if depth == 0:
        handler = viewer[start:index + 1]
        break
else:
    raise ValueError("unclosed upstream function")
rust = r'''
use std::marker::PhantomData;
struct PinchEvent { delta: f32, position: () }
struct Window;
struct Context<T>(PhantomData<T>);
struct ImageView { zoom_level: f32 }
impl ImageView {
    fn set_zoom(&mut self, zoom: f32, _position: Option<()>, _cx: &mut Context<Self>) { self.zoom_level = zoom; }
''' + handler + r'''
}
fn apply(deltas: [f32; 2]) -> f32 {
    let mut image = ImageView { zoom_level: 1. };
    let mut window = Window;
    let mut context = Context(PhantomData);
    for delta in deltas { image.handle_pinch(&PinchEvent {delta, position: ()}, &mut window, &mut context); }
    image.zoom_level
}
#[test] fn same_absolute_scales_produce_different_viewer_zoom() {
    let mut linux = [0.; 2];
    let mut windows = [0.; 2];
    for (i, (previous_scale, new_absolute_scale)) in [(1., 2.), (2., 3.)].into_iter().enumerate() {
        ''' + linux_line + r'''
        linux[i] = zoom_delta;
        let last_scale = previous_scale;
        let scale = new_absolute_scale;
        ''' + windows_line + r'''
        let event = PinchEvent { ''' + windows_field + r''' position: () };
        windows[i] = event.delta;
    }
    assert_eq!(linux, [1., 1.]);
    assert_eq!(windows, [1., 0.5]);
    assert_eq!(apply(linux), 4.);
    assert_eq!(apply(windows), 3.);
}
'''
with tempfile.TemporaryDirectory(prefix="gpui-pinch-scale-audit-") as directory:
    directory = pathlib.Path(directory)
    harness = directory / "audit.rs"
    harness.write_text(rust)
    subprocess.run(["rustc", "--edition=2024", "--test", str(harness), "-o", str(directory / "audit")], check=True)
    subprocess.run([str(directory / "audit")], check=True)
