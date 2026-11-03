from datetime import timedelta
import shlex
import time


STARTUP_TIMEOUT = timedelta(seconds=120)
BUILD_TIMEOUT = timedelta(seconds=120)
EXIT_TIMEOUT = timedelta(seconds=30)
UI_TIMEOUT = timedelta(seconds=30)
KEY_DELAY_MILLISECONDS = 100
TYPING_DELAY_MILLISECONDS = 30
SCREENSHOT_SETTLE_SECONDS = 1
WINDOW_WIDTH_PIXELS = 1800
WINDOW_HEIGHT_PIXELS = 980
WINDOW_LEFT_PIXELS = 40
WINDOW_TOP_PIXELS = 40
RESIZED_WIDTH_PIXELS = 1100
RESIZED_HEIGHT_PIXELS = 720
EDITOR_CLICK_X_PIXELS = 650
EDITOR_CLICK_Y_PIXELS = 180
SOURCE = "/home/alice/workspace/resume.tex"
PDF = "/home/alice/workspace/resume.pdf"


def desktop(command):
    environment = (
        "sudo -u alice env -u TMUX -u TMUX_PANE -u TERM_PROGRAM -u TERM_PROGRAM_VERSION "
        "DISPLAY=:0 XAUTHORITY=/home/alice/.Xauthority "
        "XDG_RUNTIME_DIR=/run/user/1000 DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus "
        "LIBGL_ALWAYS_SOFTWARE=1 "
    )
    return machine.succeed(environment + command, timeout=UI_TIMEOUT)


def key(binding):
    desktop(f"xdotool key --clearmodifiers --delay {KEY_DELAY_MILLISECONDS} {shlex.quote(binding)}")


def prefix(binding):
    key("ctrl+x")
    key(binding)


def screenshot(name):
    # Let image uploads and the terminal's redraw complete before capturing review artifacts.
    time.sleep(SCREENSHOT_SETTLE_SECONDS)
    machine.screenshot(name)


def wait_for_text(pattern, timeout=UI_TIMEOUT):
    machine.wait_for_text(pattern, timeout=timeout)


def wait_for_pdf():
    machine.wait_until_succeeds(
        f"pdftotext {PDF} - | grep -F 'Alex Example'",
        timeout=BUILD_TIMEOUT,
    )
    wait_for_text("PDF current", timeout=BUILD_TIMEOUT)


def run_smoke_tests():
    start_all()
    machine.wait_for_x()
    machine.wait_for_file("/home/alice/.Xauthority", timeout=STARTUP_TIMEOUT)
    machine.succeed("install -d -o alice -g users /home/alice/workspace")
    machine.succeed(f"install -o alice -g users -m u=rw,go=r {fixture} {SOURCE}")
    print(f"Testing {terminal_name} {terminal_version} on x86_64-linux")

    with subtest(f"launch {terminal_name} and compile offline"):
        desktop(f"sh -c {shlex.quote(terminal_command + ' > /home/alice/terminal.log 2>&1 &')}")
        machine.wait_until_succeeds("pgrep -x texo", timeout=STARTUP_TIMEOUT)
        desktop(
            f"xdotool getactivewindow windowsize {WINDOW_WIDTH_PIXELS} {WINDOW_HEIGHT_PIXELS} "
            f"windowmove {WINDOW_LEFT_PIXELS} {WINDOW_TOP_PIXELS}"
        )
        wait_for_text("texo", timeout=STARTUP_TIMEOUT)
        wait_for_pdf()
        screenshot("startup")

    with subtest("navigate from tree to editor, edit and save"):
        prefix("r")
        prefix("l")
        key("ctrl+Home")
        desktop(
            f"xdotool type --clearmodifiers --delay {TYPING_DELAY_MILLISECONDS} "
            "'% Native terminal smoke test'"
        )
        key("Return")
        prefix("w")
        machine.wait_until_succeeds(
            f"head -n 1 {SOURCE} | grep -Fx '% Native terminal smoke test'", timeout=UI_TIMEOUT
        )

        # Removing the previous PDF ensures a successful assertion requires a fresh build.
        machine.succeed(f"rm {PDF}")
        prefix("b")
        wait_for_pdf()
        screenshot("edited")

    with subtest("settings overlay and tree toggle"):
        prefix("s")
        wait_for_text("Settings")
        wait_for_text("Apply changes")
        screenshot("settings")
        key("Escape")
        wait_for_text("PDF current")

        prefix("t")
        wait_for_text("Show tree")
        screenshot("tree-hidden")
        prefix("t")
        wait_for_text("Hide tree")

    with subtest("mouse focus and system clipboard paste"):
        prefix("r")
        prefix("l")
        prefix("j")

        # This point is inside TeX at the fixed 1800 x 980 window size and 16-point font.
        desktop(
            "xdotool getactivewindow mousemove --window %1 "
            f"{EDITOR_CLICK_X_PIXELS} {EDITOR_CLICK_Y_PIXELS} click 1"
        )
        key("ctrl+Home")
        key("Home")
        clipboard = "% Native clipboard smoke\n% Clipboard second line"
        # xclip forks to own the clipboard; close its output so the VM driver can return.
        clipboard_command = f"printf %s {shlex.quote(clipboard)} | xclip -selection clipboard >/dev/null 2>&1"
        desktop(f"sh -c {shlex.quote(clipboard_command)}")
        key("ctrl+shift+v")
        prefix("w")
        machine.wait_until_succeeds(
            f"head -n 1 {SOURCE} | grep -Fx '% Native clipboard smoke'", timeout=UI_TIMEOUT
        )
        prefix("b")
        wait_for_pdf()

    with subtest("fullscreen PDF rendering"):
        prefix("r")
        prefix("l")
        prefix("l")
        prefix("z")
        wait_for_text("PDF current")

        if terminal_name in {"kitty", "ghostty", "wezterm"}:
            # The editor is hidden, so OCR must find this text in the rendered PDF itself.
            wait_for_text("Alex Example")

        screenshot("pdf-preview")
        prefix("z")

    with subtest("resize the actual terminal window"):
        desktop(f"xdotool getactivewindow windowsize {RESIZED_WIDTH_PIXELS} {RESIZED_HEIGHT_PIXELS}")
        wait_for_text(r"PDF\s+(current\s+)?1\s*/\s*1")
        screenshot("resized")

    with subtest("quit without crashing"):
        key("ctrl+q")
        machine.wait_for_file("/home/alice/texo-exit-status", timeout=EXIT_TIMEOUT)
        machine.succeed("grep -Fx 0 /home/alice/texo-exit-status")


try:
    run_smoke_tests()
except Exception:
    machine.screenshot("failure")
    print(machine.execute("pgrep -a texo || true"))
    print(machine.execute("cat /home/alice/texo-exit-status"))
    raise
finally:
    for log_path in ["/home/alice/terminal.log", "/home/alice/texo.log"]:
        status, _ = machine.execute(f"test -f {log_path}")
        if status == 0:
            machine.copy_from_machine(log_path)
