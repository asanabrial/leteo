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

Everything else is derived. The medians are computed from the run lists rather
than written beside them, because the first draft hard-coded `63,113` next to
the four numbers it is the median of — and a hand-copied median is exactly the
kind of second copy that survives a change to its source. `check_readings()`
then recomputes every percentage the figure prints and refuses to render if one
of them is not what the runs support.

That check has been verified by breaking what it claims to protect. Changing one
run value, renaming the off-protocol run, and swapping the two arms each make it
refuse, naming the figure it disagrees with:

```
AssertionError: median of all seven runs: the figure says 27%, the runs say 24%
```

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
