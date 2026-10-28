#!/usr/bin/env python3
"""Scripted provider boundary: real Texo tools and compilation still run."""
import json
from pathlib import Path
import sys
import time

WAIT_TIMEOUT_SECONDS = 20
POLL_INTERVAL_SECONDS = 0.02


def request_context():
    objects = []
    for line in sys.stdin.read().splitlines():
        if line.startswith("{"):
            objects.append(json.loads(line))

    context = next(item for item in objects if "current_tex" in item)
    protocol = next(item for item in objects if "tool_history" in item)
    return context, protocol["tool_history"]


def tool(identifier, name, arguments):
    return {"id": identifier, "name": name, "arguments_json": json.dumps(arguments)}


def respond():
    if sys.argv[1:] == ["login", "status"]:
        return

    context, history = request_context()
    prompt = next(
        message["content"]
        for message in reversed(context["conversation"])
        if message["role"] == "user"
    )
    controls = Path(__file__).parent
    (controls / "context.json").write_text(json.dumps(context))

    if prompt == "fail":
        sys.exit(1)

    if prompt == "hold" and not history:
        (controls / "started").touch()
        deadline = time.monotonic() + WAIT_TIMEOUT_SECONDS
        while not (controls / "release").exists():
            if time.monotonic() >= deadline:
                raise TimeoutError("Test did not release the held reply")
            time.sleep(POLL_INTERVAL_SECONDS)

    calls = []
    if prompt != "advice" and not history:
        content = "First draft" if prompt != "refine" else "Refined draft"
        if prompt == "repair":
            content = r"\undefinedTestCommand"
        source = "\\documentclass{article}\n\\begin{document}\n" + content + "\n\\end{document}\n"
        calls = [
            tool("edit", "propose_edit", {"tex": source}),
            tool("build", "compile_resume", {}),
        ]
    elif prompt == "repair" and len(history) == 1:
        source = "\\documentclass{article}\n\\begin{document}\nRepaired draft\n\\end{document}\n"
        calls = [
            tool("repair", "propose_edit", {"tex": source}),
            tool("retry", "compile_resume", {}),
        ]

    response = {"reply": "Fixture completed " + prompt, "proposed_tex": None, "tool_calls": calls}
    output = Path(sys.argv[sys.argv.index("-o") + 1])
    output.write_text(json.dumps(response))


if __name__ == "__main__":
    respond()
