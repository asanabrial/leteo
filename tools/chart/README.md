# The README's token figure, as a recipe

```sh
python3 tools/chart/tokens.py
```

No dependencies. It writes `assets/tokens.svg` and prints where it put it.

`assets/tokens.svg` is **output**. Edit `tokens.py` and regenerate; a number
changed in the SVG is a number that disagrees with the runs the next time
anybody regenerates, and nothing would say so.

## Where the numbers come from

The runs at the top of `tokens.py` are copied from
[`docs/does-memory-save-tokens.md`](../../docs/does-memory-save-tokens.md),
section *"Six engines, one command"*. They are the only place the figure states
them. B2, the question only the store can answer, is drawn: one lane without
memory and one for each of six engines, sorted by median so that the order of
the list in the file cannot flatter anybody by position.

B1 has no lanes and is not left out. On that question the six engines land on
both sides of the no-memory median, so the footnote gives the range and where
this engine sits in it. A figure showing only the question memory wins would be
choosing its pair.

**Measurements are written once; sentences about them are built.** The runs and
how many answers were right in each lane are the measurements. Everything the
figure says about them is assembled from those: the medians, the number of runs
in the subtitle, every lane's verdict and its figure against no memory, which
engine is the cheapest and which the dearest, and the whole description.

Review found the same defect in this file three times over, each time in a
sentence written to fix the previous one — `63,113` hard-copied beside the four
numbers it is the median of, then `seven runs` beside two lists that add up to
it, then a `0` typed into three separate f-strings. Each was a second copy of
something a list already determined.

**What is still literal, and why each is allowed.** The three percentages on the
cards and the three in the footnote, which `check_readings()` recomputes and
rejects. The title, which is prose. The canvas geometry, which nothing else
determines. And this file, which spells things out in prose above and below —
deliberately, because a sentence explaining why B1 is not drawn is not a thing a
generator produces. Change the runs and this file is one of the places to
reread.

## What the check covers, and what it does not

`check_readings()` recomputes the three percentages on the cards and the three
in the footnote, and refuses to write if one of them is not what the runs
support. It also refuses a run that does not fit the axis, a lane recorded as
more right answers than it has runs, and two questions that do not list the same
engines.

**It does not notice every change to a run.** A card is checked against the
median of the engine it names, so only the middle run of that engine matters to
it. And two of the cards follow a role, not a name: move Engram's middle run
from `234,842` to `250,000` and nothing refuses, because server-memory at
`235,006` becomes the cheapest engine and its median also rounds to 49% fewer.
The figure that comes out is right, with a different name on the card. An engine
that is on no card, claude-mem or mem0 here, can move anywhere: its lane is
computed, so there is no literal for it to disagree with.

Nor is the footnote's wording checked beyond its three figures.

So read it as *the printed percentages agree with the printed runs*, which is
what it asserts, and not as *the figure is correct*.

### Verified by breaking it

Each of these makes it refuse, and the generator and the figure were restored
byte for byte after the last:

| broken | what it says |
| --- | --- |
| this engine's middle run (`327,398` → `300,000`) | `Leteo, median against no memory: the figure says 29% fewer, the runs say 35% fewer` |
| the cheapest engine's middle run, kept cheapest (`234,842` → `220,000`) | `the cheapest engine, Engram: the figure says 49% fewer, the runs say 52% fewer` |
| the axis shortened under a run (`VMAX = 600_000.0`) | `a run of 684,593 does not fit an axis that stops at 600,000 -- it would be drawn past the end of it, in silence` |
| a B1 run that feeds the footnote (claude-mem `118,621` → `140,000`) | `the footnote says (('13%', 'fewer'), ('21%', 'more'), ('5%', 'fewer')) on B1, the runs say (('5%', 'fewer'), ('21%', 'more'), ('5%', 'fewer'))` |
| an engine dropped from B1 only (the mem0 line) | `the two questions do not list the same engines in the same order` |
| more right answers than runs (basic-memory `3` → `4`) | `basic-memory is recorded as 4 right out of 3 runs` |

Row two keeps Engram cheapest on purpose. Raising its middle run instead hands
the role to server-memory, as the section above says, and refuses nothing. A row
a maintainer cannot reproduce from its own label is not evidence.

## The description is generated, and the README's copy of it is guarded

`assets/tokens.svg` carries an `aria-label`, and `README.md`'s `<img alt>` is
that same string — it has to be, because a figure embedded with `<img src>`
exposes the `alt` and not the SVG's own label. Only one of the two is generated,
so the other is what goes stale, which is why
`tests/repository_guards.rs::the_figure_describes_itself_the_same_way_twice`
compares them and fails the build when they part. Change the figure, regenerate,
and copy the new `aria-label` into the README rather than editing the `alt`.

That guard was broken twice to check it is one, on the figure as it stood in
August — one number drifting in the alt (`27%` to `37%`), and a clause deleted
from it, which is the defect it exists to prevent. Both failed, and `README.md`
was restored byte for byte after each. It is not in `tools/guards.json`: every case there names a file
under `src/`, and the mutation harness runs the whole suite per case, which is
ten minutes for a string comparison. Broken by hand instead, and recorded here
because that is the only place it would be recorded.

**What it does not do.** It proves the two descriptions agree with each other,
not that either agrees with the runs. Edit the same number in both and it
passes; edit a dot label or a card figure, which are outside the `aria-label`,
and it never looks. Nothing in this repository compares `assets/tokens.svg` to
what `tokens.py` would generate.

## Two choices that are not decoration

**No signed percentages.** An earlier draft labelled the saving `-27%` and it
read as *27% worse* — a sign is the wrong carrier for a direction when the
reader has not yet been told what is being subtracted from what. Every figure is
a magnitude and a word: `29% fewer`, `2% more`.

**The correctness travels with the tokens.** `0 of 3 right` and `3 of 3 right`
sit in the lane labels because the article's own conclusion is that correctness
is what survives the variance and the tokens are the noisier half. A token chart
standing alone would invert the document it is drawn from.

## What is not in the figure, and where it went

B1 has no lanes, for the reason given at the top, and the footnote carries it.

Leteo's first three runs of the day, through its own command line, are not drawn
either. They gave a different median from the three drawn here, 257,230 against
327,398, and the article says so and says what that difference means for every
other gap on the figure. The lanes show the runs in which all six engines were
asked through the same command with the same prompt, because that is the only
set in which the lanes can be compared with each other.

The August runs were counted in another unit and cannot share an axis with
these; they stay in the article, dated.

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
