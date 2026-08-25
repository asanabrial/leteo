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

What the figure counts, it counts. The medians are computed rather than written
beside the lists they come from; so are the correctness counts, the number of
runs in the subtitle and on the middle card, and every count in the description
it generates. Three reviews found the same shape of defect three times — `63,113`
hard-copied beside the four numbers it is the median of, `0 of 3 right` beside
a list whose length determines it, `seven runs` beside two lists that add up to
it — so nothing the figure prints is spelled out by hand any more.

This file still spells them out, above and below, and that is a second copy in
the same sense. It is left as prose deliberately: a sentence explaining why B1
is not drawn is not a thing the generator can produce, and a wrong count here
misleads a maintainer rather than a reader of the front page. Change the runs
and this file is one of the places to reread.

What is literal: the four percentages, the card labels, and the canvas geometry.
The percentages are checked below. The labels are not.

## What the check covers, and what it does not

`check_readings()` recomputes the four percentages the figure prints and refuses
to write if one of them is not what the runs support. It also refuses a run that
does not fit the axis, which would otherwise be drawn past the end of it in
silence.

**It does not notice every change to a run.** The percentages come from
`min(WITH)`, `max(WITHOUT)`, the two medians, `median(STRICT)` and `min(STRICT)`,
so a run only matters while it is one of those. Swept one value at a time over
every integer up to 130,000, two of the seven can move a long way with nothing
firing:

| run | moves freely over | why |
| --- | --- | --- |
| `27,394` | `0` to `86,105` | it is the minimum of its arm and the arm's median sits above it |
| `100,417` | `70,909` to `105,000` | it is above the two-element middle that forms the median of `WITH` |

Outside those spans the percentages move and it refuses — `27,394 → 90,000`
makes its arm's median 90,000 and the figure's 27% becomes 30%. The spans are
measured, not reasoned: an earlier draft of this table reasoned them as
`85,950` and `71,037` and both edges were wrong.

Nor are the card labels checked against the pair beside them: swap two of those
strings and nothing objects.

So read it as *the printed percentages agree with the printed runs*, which is
what it asserts, and not as *the figure is correct*.

### Verified by breaking it

Each of these makes it refuse, and the generator was restored byte for byte
after each:

| broken | what it says |
| --- | --- |
| a run that feeds a reading (`55,189` → `60,000`) | `median vs median, all 7 runs: the figure says 27%, the runs say 24%` |
| the off-protocol run renamed | `median, strict protocol only: the figure says 17%, the runs say 27%` |
| the two **medians** swapped (`MEDIAN_WITHOUT = median(WITH)` and back) | `median vs median, all 7 runs: the figure says 27%, the runs say -36%` |
| a run past the end of the axis (`100,417` → `250,000`) | `a run of 250,000 does not fit an axis that stops at 105,000 -- it would be drawn past the end of it, in silence` |

Row three names the exact edit because the obvious reading — swapping the two
run *lists* — fires a different assertion, at 73% rather than −36%. A row a
maintainer cannot reproduce from its own label is not evidence.

### Two of the remaining assertions are watching nothing

`assert OFF_PROTOCOL in WITH` and `assert MEDIAN_WITH < MEDIAN_WITHOUT` cannot
fire, and that is provable rather than merely unobserved:

- If `OFF_PROTOCOL` is not in `WITH` then `STRICT` is `WITH`, so `median(STRICT)`
  is `MEDIAN_WITH` — and the readings print `27%` and `17%` for that one value,
  which cannot both hold. The percentage check always fails first.
- Reaching the second requires the `27%` reading to have passed, and
  `round((1 - a/b) * 100) == 27` already puts `a` below `b`.

`assert len(STRICT) == len(WITH) - 1` is the one that is live: two runs sharing
the off-protocol value would be double-starred, and
`WITH = [29_996, 29_996, 55_000, 71_000, 71_500, 90_000]` reaches it with all
four percentages still holding. The other two are kept as statements of intent
and are not evidence of anything.

## The description is generated, and the README's copy of it is guarded

`assets/tokens.svg` carries an `aria-label`, and `README.md`'s `<img alt>` is
that same string — it has to be, because a figure embedded with `<img src>`
exposes the `alt` and not the SVG's own label. Only one of the two is generated,
so the other is what goes stale, which is why
`tests/repository_guards.rs::the_figure_describes_itself_the_same_way_twice`
compares them and fails the build when they part. Change the figure, regenerate,
and copy the new `aria-label` into the README rather than editing the `alt`.

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
