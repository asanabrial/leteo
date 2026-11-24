#!/usr/bin/env python3
"""Draw the README's token figure from the runs in docs/does-memory-save-tokens.md.

The values below are the only place they are written for the figure. Change a
number here and regenerate; never edit assets/tokens.svg, which is output.

The figure deliberately carries no signed percentage. An earlier draft labelled
the saving "-27%" and it read at a glance as *27% worse* -- a sign is the wrong
carrier for a direction when the reader has not yet been told what is being
subtracted from what. Every figure here is a magnitude and a word instead.

It also carries the correctness beside the tokens. The article's own conclusion
is that the correctness is what survives the variance and that the tokens are
the noisier half; a token chart standing alone would invert the document it is
drawn from.

Every engine has a lane, not only this one. The first figure drew Leteo against
no memory and nothing else, which answers a question nobody choosing a memory
engine is asking, and it let a reader assume the saving was Leteo's. Measured,
it is not: on this question three engines spent less than Leteo did.
"""

import io
import os

# --- the measurement, from docs/does-memory-save-tokens.md -------------------
# "Six engines, one command": B2, the question only the store can answer.
# Tokens per run, every model call of the run summed.
WITHOUT = [384_028, 461_868, 684_593]
# How many answers were right. A measurement like the token counts, so it is
# written once and every sentence about it is built from here.
RIGHT_WITHOUT = 0

# name, runs, how many were right. The order here is not the order drawn: the
# lanes are sorted by median, so the list cannot flatter anybody by position.
ENGINES = [
    ("Leteo", [154_469, 327_398, 370_348], 3),
    ("Engram", [207_802, 234_842, 382_985], 3),
    ("server-memory", [230_895, 235_006, 340_772], 3),
    ("claude-mem", [290_273, 314_557, 337_273], 3),
    ("mem0", [324_451, 390_714, 446_152], 3),
    ("basic-memory", [369_167, 469_391, 666_401], 3),
]
OURS = "Leteo"

# B1, the question the repository answers. It has no lanes, and it is not left
# out either: the footnote says how the same engines did on it, because a figure
# that drew only the question memory wins would be choosing its pair.
B1_WITHOUT = [110_812, 137_128, 154_766]
B1_ENGINES = [
    ("Leteo", [122_701, 130_836, 158_182]),
    ("Engram", [118_893, 151_672, 213_151]),
    ("server-memory", [131_060, 132_393, 160_700]),
    ("claude-mem", [102_333, 118_621, 196_233]),
    ("mem0", [73_959, 156_387, 179_845]),
    ("basic-memory", [96_743, 166_046, 227_622]),
]


def median(values):
    ordered = sorted(values)
    mid = len(ordered) // 2
    if len(ordered) % 2:
        return ordered[mid]
    return (ordered[mid - 1] + ordered[mid]) / 2


# Derived, never written twice. The first draft hard-coded a median beside the
# list it is the median of, which is how a hand-copied median survives a change
# to the runs it came from.
MEDIAN_WITHOUT = median(WITHOUT)
BY_MEDIAN = sorted(ENGINES, key=lambda engine: median(engine[1]))
CHEAPEST, DEAREST = BY_MEDIAN[0], BY_MEDIAN[-1]
RUNS_OF = {name: runs for name, runs, _ in ENGINES}
RUNS = len(WITHOUT) + sum(len(runs) for _, runs, _ in ENGINES)


def against(value, baseline):
    """A median beside the one without memory, as `(magnitude, word)`."""
    change = round((value / baseline - 1) * 100)
    return (f"{abs(change)}%", "fewer" if change < 0 else "more")


# What the three cards print, each a literal that check_readings() recomputes
# from the runs and refuses when the two disagree.
READINGS = [
    ("29%", "fewer", f"{OURS}, median against no memory", OURS),
    ("49%", "fewer", f"the cheapest engine, {CHEAPEST[0]}", CHEAPEST[0]),
    ("2%", "more", f"the dearest engine, {DEAREST[0]}", DEAREST[0]),
]
# The footnote's three figures for B1: the best and the worst engine median
# against no memory, and this engine's.
B1_READINGS = (("13%", "fewer"), ("21%", "more"), ("5%", "fewer"))

# --- canvas -----------------------------------------------------------------
W = 880
X0, X1 = 168.0, 690.0
SIDE = 708
VMAX = 700_000.0
TICK = 100_000  # gridline spacing; the last one drawn is the last that fits VMAX
TOP, FIRST_LANE, LANE_GAP = 96, 126, 46
LANES = 1 + len(ENGINES)
BOT = FIRST_LANE + LANE_GAP * (LANES - 1) + 26
FOOTER = BOT + 92
H = FOOTER + 66

BG, CARD = "#0b1220", "#101c2f"
GRID, AXIS = "#1b2941", "#5b6b85"
TITLE, SUB = "#e2e8f0", "#8496b0"
RED, RED_FILL = "#f2626b", "#7f2a30"
GREEN, GREEN_FILL = "#5ed48a", "#245c3c"
OTHER, OTHER_FILL = "#8aa0c2", "#2a3a57"

MONO = "ui-monospace, 'SFMono-Regular', Menlo, Consolas, 'DejaVu Sans Mono', monospace"
SANS = "-apple-system, 'Segoe UI', Roboto, 'Helvetica Neue', Arial, sans-serif"


def correct(right, runs):
    """How many of an arm's runs answered correctly, as the lanes say it."""
    return f"{right} of {len(runs)} right"


def check_readings():
    """Every printed percentage is derivable from the runs, or this refuses."""
    for figure, word, label, name in READINGS:
        got = against(median(RUNS_OF[name]), MEDIAN_WITHOUT)
        assert got == (figure, word), (
            f"{label}: the figure says {figure} {word}, the runs say {got[0]} {got[1]}"
        )
    b1 = sorted(B1_ENGINES, key=lambda engine: median(engine[1]))
    b1_without = median(B1_WITHOUT)
    measured = (
        against(median(b1[0][1]), b1_without),
        against(median(b1[-1][1]), b1_without),
        against(median(dict(B1_ENGINES)[OURS]), b1_without),
    )
    assert measured == B1_READINGS, f"the footnote says {B1_READINGS} on B1, the runs say {measured}"
    assert [name for name, _ in B1_ENGINES] == [name for name, _, _ in ENGINES], (
        "the two questions do not list the same engines in the same order"
    )
    for name, runs, right in [("without memory", WITHOUT, RIGHT_WITHOUT)] + ENGINES:
        assert 0 <= right <= len(runs), f"{name} is recorded as {right} right out of {len(runs)} runs"
    longest = max(WITHOUT + [run for _, runs, _ in ENGINES for run in runs])
    assert longest <= VMAX, (
        f"a run of {longest:,} does not fit an axis that stops at "
        f"{VMAX:,.0f} -- it would be drawn past the end of it, in silence"
    )


def x(value):
    return X0 + (value / VMAX) * (X1 - X0)


def footnote():
    (best, best_word), (worst, worst_word), (ours, ours_word) = B1_READINGS
    return (
        f"On the question the code does answer, not drawn, the six medians run from {best} "
        f"{best_word} to {worst} {worst_word} than without memory, {OURS}'s is {ours} {ours_word}, "
        "and every run was right."
    )


def render():
    out = []
    add = out.append

    def listed(runs):
        return ", ".join(f"{v:,}" for v in runs)

    lanes_said = " ".join(
        f"{name}: {listed(runs)}, {correct(right, runs)}, median {median(runs):,.0f}, "
        f"{' '.join(against(median(runs), MEDIAN_WITHOUT))}."
        for name, runs, right in BY_MEDIAN
    )
    alt = (
        f"Tokens per run on the one question the code cannot answer, {RUNS} runs. Without memory: "
        f"{listed(WITHOUT)} tokens, {correct(RIGHT_WITHOUT, WITHOUT)}, median {MEDIAN_WITHOUT:,.0f}. "
        f"With each of six memory engines, cheapest first, each against that median. {lanes_said} "
        f"{footnote()}"
    )
    add(
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" '
        f'viewBox="0 0 {W} {H}" font-family="{SANS}" role="img" aria-label="{alt}">'
    )
    add(f'<rect width="{W}" height="{H}" rx="10" fill="{BG}"/>')

    add(
        f'<text x="24" y="36" fill="{TITLE}" font-size="17" font-weight="600">'
        "Tokens per run — the one question the code cannot answer</text>"
    )
    add(
        f'<text x="24" y="57" fill="{SUB}" font-size="12.5">{RUNS} runs · same agent, same prompt, '
        f"same memories in every engine · further left is cheaper</text>"
    )
    add(
        f'<text x="{SIDE}" y="{TOP - 16}" fill="{AXIS}" font-size="10.5" '
        f'font-family="{MONO}">median · against none</text>'
    )

    for value in range(0, int(VMAX) + 1, TICK):
        gx = x(value)
        add(
            f'<line x1="{gx:.1f}" y1="{TOP - 8}" x2="{gx:.1f}" y2="{BOT}" '
            f'stroke="{GRID}" stroke-width="1"/>'
        )
        add(
            f'<text x="{gx:.1f}" y="{BOT + 19}" fill="{AXIS}" font-size="11" '
            f'font-family="{MONO}" text-anchor="middle">{value // 1000}k</text>'
        )

    # A full-height guide at the median without memory, so each lane is read
    # against one line rather than against a dot three lanes above it.
    gx = x(MEDIAN_WITHOUT)
    add(
        f'<line x1="{gx:.1f}" y1="{TOP - 8}" x2="{gx:.1f}" y2="{BOT}" stroke="{RED}" '
        'stroke-width="1.5" stroke-dasharray="4 4" opacity="0.5"/>'
    )
    add(
        f'<text x="{gx:.1f}" y="{TOP - 16}" fill="{RED}" font-size="10.5" '
        f'font-family="{MONO}" text-anchor="middle" opacity="0.95">median without</text>'
    )

    def lane(y, label, verdict, runs, stroke, fill, side):
        add(
            f'<line x1="{X0:.1f}" y1="{y}" x2="{X1:.1f}" y2="{y}" '
            f'stroke="{GRID}" stroke-width="1"/>'
        )
        add(
            f'<text x="152" y="{y - 3}" fill="{TITLE}" font-size="13.5" font-weight="600" '
            f'text-anchor="end">{label}</text>'
        )
        add(
            f'<text x="152" y="{y + 13}" fill="{stroke}" font-size="11.5" font-family="{MONO}" '
            f'text-anchor="end">{verdict}</text>'
        )
        mid = median(runs)
        for value in runs:
            # The middle run is drawn solid and the other two hollow, so the
            # median named at the end of the lane is a dot the eye can find.
            inside = fill if value != mid else stroke
            add(
                f'<circle cx="{x(value):.1f}" cy="{y}" r="7" fill="{inside}" stroke="{stroke}" '
                f'stroke-width="2"/>'
            )
        add(
            f'<text x="{SIDE}" y="{y - 3}" fill="{TITLE}" font-size="12.5" '
            f'font-family="{MONO}">{mid:,.0f}</text>'
        )
        add(
            f'<text x="{SIDE}" y="{y + 13}" fill="{stroke}" font-size="11.5" '
            f'font-family="{MONO}">{side}</text>'
        )

    lane(FIRST_LANE, "without memory", correct(RIGHT_WITHOUT, WITHOUT), WITHOUT, RED, RED_FILL, "the baseline")
    for i, (name, runs, right) in enumerate(BY_MEDIAN, start=1):
        stroke, fill = (GREEN, GREEN_FILL) if name == OURS else (OTHER, OTHER_FILL)
        lane(
            FIRST_LANE + LANE_GAP * i,
            name,
            correct(right, runs),
            runs,
            stroke,
            fill,
            " ".join(against(median(runs), MEDIAN_WITHOUT)),
        )

    add(
        f'<rect x="24" y="{FOOTER - 42}" width="{W - 48}" height="70" rx="7" '
        f'fill="{CARD}" stroke="{GRID}"/>'
    )
    cell = (W - 48) / len(READINGS)
    for i, (figure, word, label, name) in enumerate(READINGS):
        cx = 24 + cell * i + cell / 2
        colour = GREEN if name == OURS else OTHER
        if i:
            edge = 24 + cell * i
            add(
                f'<line x1="{edge:.1f}" y1="{FOOTER - 32}" x2="{edge:.1f}" '
                f'y2="{FOOTER + 18}" stroke="{GRID}"/>'
            )
        add(
            f'<text x="{cx:.1f}" y="{FOOTER - 20}" fill="{colour}" font-size="11" '
            f'font-weight="600" letter-spacing="1.6" text-anchor="middle">{word.upper()} TOKENS</text>'
        )
        add(
            f'<text x="{cx:.1f}" y="{FOOTER + 6}" fill="{colour}" font-size="26" '
            f'font-weight="700" font-family="{MONO}" text-anchor="middle">{figure}</text>'
        )
        add(
            f'<text x="{cx:.1f}" y="{FOOTER + 23}" fill="{SUB}" font-size="10.5" '
            f'text-anchor="middle">{label}</text>'
        )

    add(f'<text x="24" y="{H - 13}" fill="{AXIS}" font-size="10">{footnote()}</text>')
    add("</svg>")
    return "\n".join(out)


if __name__ == "__main__":
    check_readings()
    root = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    target = os.path.join(root, "assets", "tokens.svg")
    io.open(target, "w", encoding="utf-8", newline="\n").write(render())
    print(f"wrote {target}")
