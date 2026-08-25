#!/usr/bin/env python3
"""Draw the README's token figure from the runs in docs/does-memory-save-tokens.md.

The values below are the only place they are written for the figure. Change a
number here and regenerate; never edit assets/tokens.svg, which is output.

The figure deliberately carries no signed percentage. An earlier draft labelled
the saving "-27%" and it read at a glance as *27% worse* -- a sign is the wrong
carrier for a direction when the reader has not yet been told what is being
subtracted from what. Every figure here is a verb and a magnitude instead.

It also carries the correctness beside the tokens. The article's own conclusion
is that 0 of 3 against 4 of 4 is what survives the variance and that the tokens
are the noisier half; a token chart standing alone would invert the document it
is drawn from.
"""

import io
import os

# --- the measurement, from docs/does-memory-save-tokens.md, "The runs" -------
# B2: the questions only the store can answer. B1 is not drawn -- the article
# says its 14% median is smaller than the noise it sits in, and charting a
# difference its own author would not defend is the opposite of the point.
WITHOUT = [27_394, 85_950, 98_733]
WITH = [29_996, 55_189, 71_037, 100_417]
CORRECT_WITHOUT = f"0 of {len(WITHOUT)} right"
CORRECT_WITH = f"{len(WITH)} of {len(WITH)} right"

# The one run whose prompt asked for a field the others did not. It is counted,
# and it is marked wherever it appears -- a dashed dot, an asterisk on its
# value, and a footnote saying what the numbers become without it.
OFF_PROTOCOL = 29_996


def median(values):
    ordered = sorted(values)
    mid = len(ordered) // 2
    if len(ordered) % 2:
        return ordered[mid]
    return (ordered[mid - 1] + ordered[mid]) / 2


# Derived, never written twice. The first draft hard-coded 63,113 beside the
# list it is the median of, and the guard below caught it as a number that is
# not any run -- which is true, and is also how a hand-copied median survives a
# change to the runs it came from.
MEDIAN_WITHOUT = median(WITHOUT)
MEDIAN_WITH = median(WITH)
STRICT = [v for v in WITH if v != OFF_PROTOCOL]

# Each reading is a pair drawn from the runs above, and each is checked against
# them by check_readings() rather than trusted.
READINGS = [
    ("saves", "70%", "best run · first search landed", min(WITH), max(WITHOUT)),
    ("saves", "27%", "median of all seven runs", MEDIAN_WITH, MEDIAN_WITHOUT),
    ("saves", "17%", "median, strict protocol only", median(STRICT), MEDIAN_WITHOUT),
]
STRICT_BEST = (min(STRICT), max(WITHOUT), "44%")  # the footnote's figure

# --- canvas -----------------------------------------------------------------
W, H = 880, 414
X0, X1 = 168.0, 836.0
VMAX = 105_000.0
TOP, LANE_1, ARROW, LANE_2, BOT = 96, 128, 176, 224, 258
FOOTER = 348

BG, CARD = "#0b1220", "#101c2f"
GRID, AXIS = "#1b2941", "#5b6b85"
TITLE, SUB = "#e2e8f0", "#8496b0"
RED, RED_FILL = "#f2626b", "#7f2a30"
GREEN, GREEN_FILL, GREEN_DIM = "#5ed48a", "#245c3c", "#3f9c66"

MONO = "ui-monospace, 'SFMono-Regular', Menlo, Consolas, 'DejaVu Sans Mono', monospace"
SANS = "-apple-system, 'Segoe UI', Roboto, 'Helvetica Neue', Arial, sans-serif"


def saving(with_leteo, without):
    """The percentage a pair supports, rounded the way the figure prints it."""
    return round((1 - with_leteo / without) * 100)


def check_readings():
    """Every printed percentage is derivable from the runs, or this refuses."""
    for _, printed, label, a, b in READINGS + [("", STRICT_BEST[2], "the footnote's best run",
                                                STRICT_BEST[0], STRICT_BEST[1])]:
        got = f"{saving(a, b)}%"
        assert got == printed, f"{label}: the figure says {printed}, the runs say {got}"
    assert max(WITHOUT + WITH) <= VMAX, (
        f"a run of {max(WITHOUT + WITH):,} does not fit an axis that stops at "
        f"{VMAX:,.0f} -- it would be drawn outside the canvas, in silence"
    )
    assert OFF_PROTOCOL in WITH, "the off-protocol run is not among the runs"
    assert len(STRICT) == len(WITH) - 1, "the strict protocol dropped more than one run"
    assert MEDIAN_WITH < MEDIAN_WITHOUT, "the arrow points the wrong way"


def x(value):
    return X0 + (value / VMAX) * (X1 - X0)


def render():
    out = []
    add = out.append

    runs_without = ", ".join(f"{v:,}" for v in WITHOUT)
    runs_with = ", ".join(f"{v:,}" for v in WITH)
    alt = (
        "Tokens per run on the questions the code cannot answer. Without Leteo: three runs at "
        f"{runs_without} tokens, {CORRECT_WITHOUT}. With Leteo: four runs at {runs_with} tokens, "
        f"{CORRECT_WITH}. The median falls from {MEDIAN_WITHOUT:,.0f} to {MEDIAN_WITH:,.0f}, which is "
        f"{saving(MEDIAN_WITH, MEDIAN_WITHOUT)}% fewer tokens."
    )
    add(
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" '
        f'viewBox="0 0 {W} {H}" font-family="{SANS}" role="img" aria-label="{alt}">'
    )
    add(f'<rect width="{W}" height="{H}" rx="10" fill="{BG}"/>')

    add(
        f'<text x="24" y="36" fill="{TITLE}" font-size="17" font-weight="600">'
        "Tokens per run — the questions the code cannot answer</text>"
    )
    add(
        f'<text x="24" y="57" fill="{SUB}" font-size="12.5">seven runs · same agent, '
        "same questions, with and without Leteo · further left is cheaper</text>"
    )

    for value in range(0, 100_001, 25_000):
        gx = x(value)
        add(
            f'<line x1="{gx:.1f}" y1="{TOP - 8}" x2="{gx:.1f}" y2="{BOT}" '
            f'stroke="{GRID}" stroke-width="1"/>'
        )
        add(
            f'<text x="{gx:.1f}" y="{BOT + 19}" fill="{AXIS}" font-size="11" '
            f'font-family="{MONO}" text-anchor="middle">{value // 1000}k</text>'
        )

    # Full-height guides, so the distance between the two medians is the picture
    # rather than something the reader has to measure by eye between two dots.
    for mid, colour in ((MEDIAN_WITHOUT, RED), (MEDIAN_WITH, GREEN)):
        gx = x(mid)
        add(
            f'<line x1="{gx:.1f}" y1="{TOP - 8}" x2="{gx:.1f}" y2="{BOT}" stroke="{colour}" '
            'stroke-width="1.5" stroke-dasharray="4 4" opacity="0.5"/>'
        )
        add(
            f'<text x="{gx:.1f}" y="{TOP - 16}" fill="{colour}" font-size="10.5" '
            f'font-family="{MONO}" text-anchor="middle" opacity="0.95">median {mid:,.0f}</text>'
        )

    def lane(y, label, verdict, runs, stroke, fill):
        add(
            f'<line x1="{X0:.1f}" y1="{y}" x2="{X1:.1f}" y2="{y}" '
            f'stroke="{GRID}" stroke-width="1"/>'
        )
        add(
            f'<text x="152" y="{y - 4}" fill="{TITLE}" font-size="13.5" font-weight="600" '
            f'text-anchor="end">{label}</text>'
        )
        add(
            f'<text x="152" y="{y + 14}" fill="{stroke}" font-size="12" font-family="{MONO}" '
            f'text-anchor="end">{verdict}</text>'
        )
        for value in runs:
            cx = x(value)
            dashed = ' stroke-dasharray="2.5 2.5"' if value == OFF_PROTOCOL else ""
            star = "*" if value == OFF_PROTOCOL else ""
            add(
                f'<circle cx="{cx:.1f}" cy="{y}" r="8.5" fill="{fill}" stroke="{stroke}" '
                f'stroke-width="2"{dashed}/>'
            )
            add(
                f'<text x="{cx:.1f}" y="{y + 26}" fill="{SUB}" font-size="10" '
                f'font-family="{MONO}" text-anchor="middle">{value:,}{star}</text>'
            )

    lane(LANE_1, "without Leteo", CORRECT_WITHOUT, WITHOUT, RED, RED_FILL)
    lane(LANE_2, "with Leteo", CORRECT_WITH, WITH, GREEN, GREEN_FILL)

    xa, xb = x(MEDIAN_WITH), x(MEDIAN_WITHOUT)
    ay = ARROW + 14
    add(
        f'<text x="{(xa + xb) / 2:.1f}" y="{ARROW - 2}" fill="{GREEN}" font-size="14.5" '
        f'font-weight="700" text-anchor="middle">'
        f"{saving(MEDIAN_WITH, MEDIAN_WITHOUT)}% fewer tokens</text>"
    )
    add(
        f'<line x1="{xb:.1f}" y1="{ay}" x2="{xa + 9:.1f}" y2="{ay}" '
        f'stroke="{GREEN}" stroke-width="2"/>'
    )
    add(f'<path d="M {xa:.1f} {ay} l 11 -6 l 0 12 z" fill="{GREEN}"/>')
    add(
        f'<text x="{(xa + xb) / 2:.1f}" y="{ay + 15}" fill="{SUB}" font-size="10" '
        'text-anchor="middle">median to median</text>'
    )

    add(
        f'<rect x="24" y="{FOOTER - 42}" width="{W - 48}" height="70" rx="7" '
        f'fill="{CARD}" stroke="{GRID}"/>'
    )
    cell = (W - 48) / len(READINGS)
    for i, (verb, figure, label, _, _) in enumerate(READINGS):
        cx = 24 + cell * i + cell / 2
        colour = GREEN_DIM if "strict" in label else GREEN
        if i:
            edge = 24 + cell * i
            add(
                f'<line x1="{edge:.1f}" y1="{FOOTER - 32}" x2="{edge:.1f}" '
                f'y2="{FOOTER + 18}" stroke="{GRID}"/>'
            )
        add(
            f'<text x="{cx:.1f}" y="{FOOTER - 20}" fill="{colour}" font-size="11" '
            f'font-weight="600" letter-spacing="1.6" text-anchor="middle">{verb.upper()}</text>'
        )
        add(
            f'<text x="{cx:.1f}" y="{FOOTER + 6}" fill="{colour}" font-size="26" '
            f'font-weight="700" font-family="{MONO}" text-anchor="middle">{figure}</text>'
        )
        add(
            f'<text x="{cx:.1f}" y="{FOOTER + 23}" fill="{SUB}" font-size="10.5" '
            f'text-anchor="middle">{label}</text>'
        )

    add(
        f'<text x="24" y="{H - 13}" fill="{AXIS}" font-size="10">* off-protocol: its prompt '
        "asked for one field the others did not. Drop it and the best run saves "
        f"{STRICT_BEST[2]}, the median saves {READINGS[2][1]}, and it is {len(STRICT)} of "
        f"{len(STRICT)} right against 0 of {len(WITHOUT)}.</text>"
    )
    add("</svg>")
    return "\n".join(out)


if __name__ == "__main__":
    check_readings()
    root = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    target = os.path.join(root, "assets", "tokens.svg")
    io.open(target, "w", encoding="utf-8", newline="\n").write(render())
    print(f"wrote {target}")
