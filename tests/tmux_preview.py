import os
from pathlib import Path
import pty
import select
import subprocess
import sys
import termios
import time

from kitty.fast_data_types import Screen, set_options
from kitty.options.types import defaults


FRAME_TIMEOUT_SECONDS = 15
OUTPUT_SETTLE_SECONDS = 0.15
POLL_INTERVAL_SECONDS = 0.02
READ_SIZE_BYTES = 1024 * 1024
MAX_DIAGNOSTIC_BYTES = 2000
PROCESS_EXIT_TIMEOUT_SECONDS = 5

# Leave room for the app panes around the 40-by-20 preview. Filling a tiny
# terminal with placeholders triggers tmux slow-client throttling during redraws.
TERMINAL_ROWS = 35
TERMINAL_COLUMNS = 120
SECOND_PANE_ROWS = 2
PANE_BORDER_ROWS = 1
CELL_WIDTH_PIXELS = 10
CELL_HEIGHT_PIXELS = 20
DRIVER_ENVIRONMENT = "TEXO_TEST_TMUX_PREVIEW"
# Reply to ratatui-image's Kitty/compression probes and terminal cell-size query.
QUERY_RESPONSE = b"\x1b_Gi=31;OK\x1b\\\x1b_Gi=32;OK\x1b\\\x1b[6;20;10t\x1b[0n"


def feed(screen, data):
    remaining = memoryview(data)
    while remaining:
        destination = screen.test_create_write_buffer()
        consumed = screen.test_commit_write_buffer(remaining, destination)
        remaining = remaining[consumed:]
        screen.test_parse_written_data()


def assert_visible_page(screen, frame_index):
    screen.update_only_line_graphics_data()
    layers = screen.grman.update_layers(
        0, -1, 1, 2 / screen.columns, 2 / screen.lines,
        screen.columns, screen.lines, CELL_WIDTH_PIXELS, CELL_HEIGHT_PIXELS,
    )
    assert layers, f"Frame {frame_index}: placeholders survived, but no PDF image is visible"
    assert screen.grman.image_count == 1, "Replaced pages must release their terminal images"


def check_frames(master, process, directory, frame_count, terminal_rows):
    set_options(defaults)
    screen = Screen(None, terminal_rows, TERMINAL_COLUMNS, 0, CELL_WIDTH_PIXELS, CELL_HEIGHT_PIXELS)
    pending_queries = bytearray()
    capture = bytearray()
    last_output = time.monotonic()

    for frame_index in range(frame_count):
        deadline = time.monotonic() + FRAME_TIMEOUT_SECONDS
        ready = directory / f"ready-{frame_index}"
        while time.monotonic() < deadline:
            if select.select([master], [], [], POLL_INTERVAL_SECONDS)[0]:
                data = os.read(master, READ_SIZE_BYTES)
                capture.extend(data)
                pending_queries.extend(data)
                feed(screen, data)
                last_output = time.monotonic()

                if b"\x1b[5n" in pending_queries:
                    os.write(master, QUERY_RESPONSE)
                    pending_queries.clear()
                elif b"\x1b[c" in pending_queries:
                    os.write(master, b"\x1b[?62;22;52c")
                    pending_queries.clear()
            elif ready.exists() and time.monotonic() - last_output >= OUTPUT_SETTLE_SECONDS:
                (directory / f"frame-{frame_index}.bin").write_bytes(capture)
                assert_visible_page(screen, frame_index)
                os.write(master, b"n")
                break
            elif process.poll() is not None:
                raise AssertionError(f"Preview driver exited early: {capture[-MAX_DIAGNOSTIC_BYTES:]!r}")
        else:
            raise AssertionError(f"Preview frame {frame_index} timed out: {capture[-MAX_DIAGNOSTIC_BYTES:]!r}")


def check_layout(executable, directory, frame_count, split):
    directory.mkdir()
    configuration = directory / "tmux.conf"
    configuration.write_text('set -g status off\nset -g terminal-features "xterm*:RGB"\n')
    server = f"texo-preview-test-{os.getpid()}"
    environment = os.environ.copy()
    for name in ("TMUX", "TMUX_PANE", "TERM_PROGRAM", "TERM_PROGRAM_VERSION"):
        environment.pop(name, None)
    environment.update({"TERM": "xterm-256color", "NO_COLOR": "", DRIVER_ENVIRONMENT: str(directory)})

    terminal_rows = TERMINAL_ROWS
    command = [
        "tmux", "-L", server, "-f", str(configuration), "new-session", "-s", "preview",
        str(executable), "--exact", "preview::tmux_tests::tmux_preview_driver", "--nocapture",
    ]
    if split:
        terminal_rows += SECOND_PANE_ROWS + PANE_BORDER_ROWS
        command.extend([";", "split-window", "-v", "-l", str(SECOND_PANE_ROWS), "-d", "cat"])

    master, child = pty.openpty()
    termios.tcsetwinsize(child, (terminal_rows, TERMINAL_COLUMNS))
    process = subprocess.Popen(command, stdin=child, stdout=child, stderr=child, env=environment)
    os.close(child)

    try:
        check_frames(master, process, directory, frame_count, terminal_rows)
    finally:
        subprocess.run(
            ["tmux", "-L", server, "kill-server"],
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False,
        )
        if process.poll() is None:
            process.terminate()
        process.wait(timeout=PROCESS_EXIT_TIMEOUT_SECONDS)
        os.close(master)


for layout in ("single", "split"):
    check_layout(Path(sys.argv[1]), Path(sys.argv[2]) / layout, int(sys.argv[3]), layout == "split")
