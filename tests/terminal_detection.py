import errno
import os
from pathlib import Path
import pty
import select
import subprocess
import sys
import tempfile
import termios
import time


STARTUP_TIMEOUT_SECONDS = 8
QUIT_TIMEOUT_SECONDS = 2
POLL_INTERVAL_SECONDS = 0.02
READ_SIZE_BYTES = 64 * 1024
TERMINAL_ROWS = 35
TERMINAL_COLUMNS = 120
ENTER_ALTERNATE_SCREEN = b"\x1b[?1049h"
QUERY_COMPLETE = b"\x1b[5n"
KITTY_RESPONSE = b"\x1b_Gi=31;OK\x1b\\\x1b_Gi=32;OK\x1b\\"
SIZE_AND_STATUS_RESPONSE = b"\x1b[6;20;10t\x1b[0n"
QUIT_KEY = b"\x11"  # Ctrl+Q


def check_startup(master, process, scenario):
    output = bytearray()
    query_answered = False
    quit_sent = False
    deadline = time.monotonic() + STARTUP_TIMEOUT_SECONDS

    while time.monotonic() < deadline:
        if select.select([master], [], [], POLL_INTERVAL_SECONDS)[0]:
            try:
                data = os.read(master, READ_SIZE_BYTES)
            except OSError as error:
                if error.errno != errno.EIO:
                    raise
                break

            if not data:
                break

            output.extend(data)

        if not query_answered and QUERY_COMPLETE in output:
            if scenario == "supported":
                assert ENTER_ALTERNATE_SCREEN not in output, (
                    "Graphics detection ran after entering the alternate screen; "
                    "tmux can discard the query while redrawing"
                )
                os.write(master, KITTY_RESPONSE + SIZE_AND_STATUS_RESPONSE)
            elif scenario == "unsupported":
                os.write(master, SIZE_AND_STATUS_RESPONSE)

            # A silent terminal must time out without leaving a reader behind.
            query_answered = True

        if not quit_sent and b"No resumes yet" in output:
            assert query_answered, "The app did not query terminal graphics support"
            os.write(master, QUIT_KEY)
            quit_sent = True
            deadline = time.monotonic() + QUIT_TIMEOUT_SECONDS

        if process.poll() is not None:
            break

    assert quit_sent, "Texo did not finish startup"
    assert process.wait(timeout=QUIT_TIMEOUT_SECONDS) == 0, "Texo did not exit cleanly"


def run(executable, scenario):
    with tempfile.TemporaryDirectory(prefix="texo-detection-") as temporary:
        root = Path(temporary)
        workspace = root / "resumes"
        workspace.mkdir()
        environment = os.environ.copy()
        for name in (
            "TMUX", "TMUX_PANE", "TERM_PROGRAM", "TERM_PROGRAM_VERSION",
            "ITERM_SESSION_ID", "WEZTERM_EXECUTABLE", "KITTY_WINDOW_ID", "KONSOLE_VERSION",
        ):
            environment.pop(name, None)
        environment.update({
            "TERM": "xterm-256color",
            "XDG_CONFIG_HOME": str(root / "config"),
            "XDG_STATE_HOME": str(root / "state"),
        })

        master, child = pty.openpty()
        termios.tcsetwinsize(child, (TERMINAL_ROWS, TERMINAL_COLUMNS))
        original_input_mode = termios.tcgetattr(child)
        process = subprocess.Popen(
            [executable, str(workspace)], stdin=child, stdout=child, stderr=child, env=environment,
        )
        os.close(child)

        try:
            check_startup(master, process, scenario)
            assert termios.tcgetattr(master) == original_input_mode, "Texo changed the shell's input mode"
        finally:
            if process.poll() is None:
                process.kill()
            process.wait(timeout=QUIT_TIMEOUT_SECONDS)
            os.close(master)


run(sys.argv[1], sys.argv[2])
