"""Verify preview geometry using Kitty's real terminal graphics renderer.

The Rust test `terminal_renderer_actually_zooms_and_pans_the_page` in
src/preview_tests.rs captures terminal output for initial display, zoom, pan,
drag, and fit, then runs this helper through `kitty +runpy`. Python is needed
to access Kitty's internal Screen and graphics manager; checking Rust state
alone cannot prove that the terminal displays the intended part of the page.

The argument is a temporary directory containing each phase's .bin capture.
Replay them into one headless screen to check the visible image bounds and
confirm that all phases reuse one uploaded image. Print the observed bounds
as JSON for the Rust test's diagnostics; assertion failures fail that test.

Run from the repository root with Kitty installed:
    cargo test terminal_renderer_actually_zooms_and_pans_the_page -- --ignored --nocapture
"""

import json
import re
import sys
from pathlib import Path

from kitty.fast_data_types import Screen, set_options
from kitty.options.types import defaults


def feed(screen, data):
    # Replay the bytes that tmux forwards to its outer terminal.
    data = re.sub(
        rb"\x1bPtmux;((?:\x1b\x1b|[^\x1b])*)\x1b\\",
        lambda match: match[1].replace(b"\x1b\x1b", b"\x1b"),
        data,
    )
    remaining = memoryview(data)
    while remaining:
        destination = screen.test_create_write_buffer()
        consumed = screen.test_commit_write_buffer(remaining, destination)
        remaining = remaining[consumed:]
        screen.test_parse_written_data()


def visible_source(screen):
    screen.update_only_line_graphics_data()
    layers = screen.grman.update_layers(
        0, -1, 1, 2 / screen.columns, 2 / screen.lines,
        screen.columns, screen.lines, 10, 20,
    )
    rectangles = [layer["src_rect"] for layer in layers]
    if not rectangles:
        raise AssertionError(f"No visible image: count={screen.grman.image_count}, line={screen.line(1)}")
    return {
        "left": min(rect["left"] for rect in rectangles),
        "top": min(rect["top"] for rect in rectangles),
        "right": max(rect["right"] for rect in rectangles),
        "bottom": max(rect["bottom"] for rect in rectangles),
    }


def main(directory):
    set_options(defaults)
    screen = Screen(None, 20, 40, 0, 10, 20)
    observed = []
    for phase in ("initial", "zoom", "pan", "drag", "fit"):
        data = (directory / f"{phase}.bin").read_bytes()
        feed(screen, data)
        observed.append(visible_source(screen))
        assert screen.grman.image_count == 1, "Zoom should reuse one uploaded image"

    initial, zoom, pan, drag, fit = observed
    initial_width = initial["right"] - initial["left"]
    zoom_width = zoom["right"] - zoom["left"]
    assert abs(zoom_width / initial_width - 0.5) < 0.03, (
        f"200% zoom did not double the visible scale: {observed}"
    )
    assert pan["left"] > zoom["left"], f"Horizontal pan did not move the page: {observed}"
    assert pan["top"] > zoom["top"], f"Vertical pan did not move the page: {observed}"
    assert drag["left"] > pan["left"] and drag["top"] > pan["top"], (
        f"Dragging did not move the rendered page: {observed}"
    )
    assert fit["left"] < 0.01 and fit["top"] < 0.01, f"Fit lost the page origin: {fit}"
    assert fit["right"] > 0.99 and fit["bottom"] > 0.99, f"Fit cropped the page: {fit}"
    print(json.dumps(observed))


main(Path(sys.argv[1]))
