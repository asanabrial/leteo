# The README's token figure, as a recipe

```sh
python3 tools/chart/tokens.py
```

No dependencies. It writes `assets/tokens.svg` and prints where it put it.

`assets/tokens.svg` is **output**. Edit `tokens.py` and regenerate; a number
changed in the SVG is a number that disagrees with the runs the next time
anybody regenerates, and nothing would say so.

## Where the numbers come from

The seven runs at the top of `tokens.py` are copied from
[`docs/does-memory-save-tokens.md`](../../docs/does-memory-save-tokens.md),
section *"The runs"*, B2 — the questions only the store can answer. They are the
only place the figure states them. B1 is deliberately not drawn: the article
says its 14% median is smaller than the noise it sits in, and charting a
difference its own author would not defend is the opposite of why the figure
exists.

Every number is derived from those lists. The medians are computed rather than
written beside them, because the first draft hard-coded `63,113` next to the four
numbers it is the median of — and a hand-copied median is exactly the kind of
second copy that survives a change to its source. The correctness counts are
`len(WITHOUT)` and `len(WITH)` for the same reason: a fourth run added to one arm
used to leave the lane still saying `0 of 3 right` beside four dots.

## What the check covers, and what it does not

`check_readings()` recomputes the four percentages the figure prints and refuses
to write if one of them is not what the runs support. It also refuses a run that
does not fit the axis, which would otherwise be drawn outside the canvas in
silence.

**It does not notice every change to a run.** The percentages are computed from
`min(WITH)`, `max(WITHOUT)`, the two medians, `median(STRICT)` and `min(STRICT)`.
Two of the seven runs feed none of those — `27,394`, which is neither the median
nor the maximum of its arm, and `100,417`, which is above the two-element middle
that forms the median of `WITH`. Move either one to another value inside the axis
and every assertion passes while its dot moves. Nor are the card labels checked
against the pair beside them: swap two of those strings and nothing objects.

So read it as *the printed percentages agree with the printed runs*, which is
what it asserts, and not as *the figure is correct*.

It has been verified by breaking what it protects. Each of these makes it refuse,
and the generator was restored byte for byte after each:

| broken | what it says |
| --- | --- |
| one run value that feeds a reading (`55,189` → `60,000`) | `median of all seven runs: the figure says 27%, the runs say 24%` |
| the off-protocol run renamed | `median, strict protocol only: the figure says 17%, the runs say 27%` |
| the two arms swapped | `median of all seven runs: the figure says 27%, the runs say -36%` |
| a run past the end of the axis (`100,417` → `250,000`) | `a run of 250,000 does not fit an axis that stops at 105,000 -- it would be drawn outside the canvas, in silence` |

The first three rows are caught by the percentage check and the fourth by the
axis assertion. The three assertions after that one — that the off-protocol run
is among the runs, that dropping it removes exactly one, and that the arrow
points the way the medians do — have not fired under any mutation tried here,
and are belt and braces rather than evidence.

(The sentence above said something else until it was checked against the table
directly beneath it, which is the point of writing the table.)

## Two choices that are not decoration

**No signed percentages.** An earlier draft labelled the saving `-27%` and it
read as *27% worse* — a sign is the wrong carrier for a direction when the
reader has not yet been told what is being subtracted from what. Every figure is
a verb and a magnitude: `SAVES 27%`.

**The correctness travels with the tokens.** `0 of 3 right` and `4 of 4 right`
sit in the lane labels because the article's own conclusion is that correctness
is what survives the variance and the tokens are the noisier half. A token chart
standing alone would invert the document it is drawn from.

## What is not in the figure, and where it went

The article supports a third reading of the same seven runs — the most expensive
run *with* Leteo against the cheapest one without, which comes out at *costs
3.7x as much*. It is cherry-picked in exactly the way the `70%` is, and a panel
holding both invites a reader to average two numbers that mean nothing averaged.

It is not hidden. The 100,417-token dot that produces it is drawn, is a *with
Leteo* run, and is the rightmost point on the chart — so the figure shows that
the most expensive run of all is ours. The ratio itself stays in the README's
*"Does it pay for itself?"* section and in the article.

## Rendering it while you work

The SVG opens in any browser. On a change worth looking at rather than
diffing:

```sh
python3 tools/chart/tokens.py && open assets/tokens.svg   # xdg-open, start
```

Text is laid out by hand at fixed coordinates, so a longer label can collide
with its neighbour without anything failing. Look at it before committing — the
first draft hid its own axis caption behind the footer card, which no assertion
would ever have caught.
