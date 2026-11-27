#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Static checks for GPUI runtime panic hazards (e.g. synchronous WeakEntity::read_with
inside a `&mut Context<...>` render/update method causing `double_lease_panic`).

Usage:
  python3 scripts/check-gpui-hazards.py
  python3 scripts/check-gpui-hazards.py --self-test
"""

from __future__ import annotations

import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent

FN_SIG_RE = re.compile(r"\bfn\s+([A-Za-z0-9_]+)\s*(?:<[^>]*>)?\s*\(")
CONTEXT_PARAM_RE = re.compile(r"&\s*mut\s+(?:gpui::)?Context\s*<")
SYNC_READ_WITH_RE = re.compile(r"\.read_with\s*\(\s*cx\b")
ASYNC_SPAWN_RE = re.compile(r"\b(?:async\s+move|cx\.spawn|background_executor|foreground_executor|on_mouse_|on_click|on_action|on_select)\b")
ALLOW_RE = re.compile(r"gpui-lint-allow:\s*\S+")


def check_source(path: str, text: str) -> list[str]:
    """Scan Rust source text for synchronous `.read_with(cx, ...)` inside `&mut Context<...>` functions."""
    findings: list[str] = []
    lines = text.splitlines()
    in_ctx_fn = False
    fn_name = ""
    brace_depth = 0
    fn_brace_depth = 0
    in_async_or_callback_depth: int | None = None

    i = 0
    while i < len(lines):
        line = lines[i]
        stripped = line.strip()

        # Detect function signatures (possibly spanning multiple lines up to '{').
        if not in_ctx_fn and FN_SIG_RE.search(line) and not stripped.startswith("//"):
            sig_lines = [line]
            j = i
            while "{" not in lines[j] and ";" not in lines[j] and j + 1 < len(lines) and j - i < 12:
                j += 1
                sig_lines.append(lines[j])
            sig = " ".join(sig_lines)
            if "{" in sig and CONTEXT_PARAM_RE.search(sig):
                m = FN_SIG_RE.search(sig)
                fn_name = m.group(1) if m else "<unknown>"
                in_ctx_fn = True
                fn_brace_depth = brace_depth + sig.count("{") - sig.count("}")
                # Track relative depth from start of function body.
                brace_depth = fn_brace_depth
                in_async_or_callback_depth = None
                i = j + 1
                continue

        if in_ctx_fn:
            if in_async_or_callback_depth is None and ASYNC_SPAWN_RE.search(line) and not stripped.startswith("//"):
                in_async_or_callback_depth = brace_depth

            if (
                in_async_or_callback_depth is None
                and SYNC_READ_WITH_RE.search(line)
                and not stripped.startswith("//")
                and not ALLOW_RE.search(line)
            ):
                findings.append(
                    f"{path}:{i + 1}: synchronous `.read_with(cx, ...)` inside `{fn_name}` "
                    f"(which already holds `&mut Context<...>`) risks GPUI `double_lease_panic`; "
                    f"pass the needed value from `self` instead (or annotate `// gpui-lint-allow: <reason>`)."
                )

            brace_depth += line.count("{") - line.count("}")
            if in_async_or_callback_depth is not None and brace_depth <= in_async_or_callback_depth:
                in_async_or_callback_depth = None
            if brace_depth < fn_brace_depth:
                in_ctx_fn = False
                fn_name = ""
        else:
            brace_depth += line.count("{") - line.count("}")

        i += 1

    return findings


def self_test() -> int:
    bad = """
fn sidebar_width_handle(editor: WeakEntity<SlideEditor>, cx: &mut Context<SlideEditor>) -> impl IntoElement {
    let dragging = editor.read_with(cx, |this, _| this.sidebar_drag.is_some()).unwrap_or(false);
    div()
}
"""
    good_async = """
fn start_task(&mut self, cx: &mut Context<Self>) {
    cx.spawn(async move |this, cx| {
        let ok = this.read_with(cx, |editor, _| editor.active).unwrap_or(false);
        let _ = ok;
    }).detach();
}
"""
    good_param = """
fn sidebar_width_handle(dragging: bool, editor: WeakEntity<SlideEditor>, cx: &mut Context<SlideEditor>) -> impl IntoElement {
    let _ = (dragging, editor, cx);
    div()
}
"""
    f_bad = check_source("bad.rs", bad)
    f_async = check_source("good_async.rs", good_async)
    f_param = check_source("good_param.rs", good_param)
    assert len(f_bad) == 1, f"expected 1 finding for bad.rs, got {f_bad}"
    assert len(f_async) == 0, f"expected 0 findings for good_async.rs, got {f_async}"
    assert len(f_param) == 0, f"expected 0 findings for good_param.rs, got {f_param}"
    print("check-gpui-hazards.py self-test passed")
    return 0


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()

    all_findings: list[str] = []
    crates_dir = ROOT / "crates"
    for rs_path in sorted(crates_dir.rglob("*.rs")):
        rel = rs_path.relative_to(ROOT).as_posix()
        text = rs_path.read_text(encoding="utf-8", errors="ignore")
        all_findings.extend(check_source(rel, text))

    if all_findings:
        for f in all_findings:
            print(f, file=sys.stderr)
        return 1
    print("check-gpui-hazards.py: 0 hazards found")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
