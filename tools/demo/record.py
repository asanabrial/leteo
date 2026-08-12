# Builds the asciicast the README's GIF is rendered from.
#
#   bash tools/demo/build-store.sh
#   python3 tools/demo/record.py
#   agg --theme ... tools/demo/loop.cast assets/leteo-loop.gif
#
# The full command line is in tools/demo/README.md.
#
# # Why this and not vhs
#
# vhs was what recorded the earlier GIFs, and it stopped working on the machine
# this project is developed on: it prints the `Set` directives and then hangs,
# before executing a single line, on a two-line tape as much as on a long one.
# It drives a headless browser to rasterise the terminal, and that is the part
# that broke — the same failure with and without `Set Shell bash`, from a real
# terminal and from an automated one.
#
# `agg` renders an asciicast to a GIF directly, with no browser anywhere. What
# it needs is a cast, and a cast is a JSON header followed by one line per
# output event. So this writes one.
#
# # What is real here and what is not
#
# The typing is animated: characters are emitted one at a time with a delay,
# the way vhs types into a shell. Nothing else is invented. Every command below
# is actually run, and what lands in the recording is what the binary printed —
# so a promise that stops holding shows up in the next recording instead of
# being hidden by it.

import json
import os
import subprocess
from pathlib import Path

HERE = Path(__file__).resolve().parent
STORE = HERE / "store"
CAST = HERE / "loop.cast"

# 20 rows is what the longest screen needs — two comments, a command and the
# eleven lines of the memory block, three of which wrap. Taller than that is
# empty terminal, and empty terminal is most of a GIF's weight for none of its
# meaning: the first cut of this was 30 rows and half the frame was background.
COLS, ROWS = 110, 20
TYPE = 0.022  # seconds per character

ESC = "\x1b"
PROMPT = f"{ESC}[38;5;141m${ESC}[0m "
GREY = f"{ESC}[38;5;245m"
RESET = f"{ESC}[0m"

env = dict(
    os.environ,
    LETEO_DATABASE=str(STORE / "demo.db"),
    # Leteo speaks the language of the machine it runs on, and this recording
    # goes in an English README.
    LETEO_SYSTEM_LANGUAGE="en",
)

events: list = []
clock = 0.0


def emit(text: str, delay: float = 0.0) -> None:
    global clock
    clock += delay
    events.append([round(clock, 3), "o", text])


def hold(seconds: float) -> None:
    global clock
    clock += seconds


def comment(text: str, after: float = 1.0) -> None:
    emit(PROMPT)
    for character in text:
        emit(GREY + character + RESET, TYPE)
    emit("\r\n", after)


def run(command: str, after: float = 1.2) -> None:
    emit(PROMPT)
    for character in command:
        emit(character, TYPE)
    emit("\r\n", 0.2)
    result = subprocess.run(
        ["bash", "-lc", command],
        cwd=STORE / "payments",
        env=env,
        capture_output=True,
        text=True,
    )
    body = (result.stdout or result.stderr).rstrip("\n")
    if not body:
        raise SystemExit(f"`{command}` printed nothing — the recording would show an empty screen")
    emit(body.replace("\n", "\r\n") + "\r\n", 0.25)
    hold(after)


def clear() -> None:
    emit(f"{ESC}[2J{ESC}[H", 0.3)


if not (STORE / "demo.db").exists():
    raise SystemExit("no store — run `bash tools/demo/build-store.sh` first")

comment("# An agent worked here yesterday. Nobody asked it to remember.")
comment("# A new session opens. This is what it gets, unprompted:", after=0.6)
# `context` rather than the session-start hook: the hook's payload opens with
# the protocol the agent is told to follow, thirty-one lines of instructions
# before the part worth showing. This is the same block without them.
run("leteo context payments | jq -r .context", after=5.0)
clear()
comment("# And the one line you see:", after=0.6)
run("leteo hook session-start | jq -r .systemMessage", after=1.8)
comment("# Mid-task it asks the store, not you:", after=0.6)
run("leteo search webhooks | jq -r '.[].title'", after=1.6)
run("leteo search 'rounding money' | jq -r '.[].title'", after=2.2)
comment("# One binary. One SQLite file. Nothing left the machine.", after=2.0)

header = {
    "version": 2,
    "width": COLS,
    "height": ROWS,
    "env": {"SHELL": "/bin/bash", "TERM": "xterm-256color"},
}
with CAST.open("w", encoding="utf-8", newline="\n") as handle:
    handle.write(json.dumps(header) + "\n")
    for event in events:
        handle.write(json.dumps(event) + "\n")

print(f"{CAST}: {len(events)} events, {events[-1][0]:.1f} s")
