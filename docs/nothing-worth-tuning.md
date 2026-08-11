# I tried to teach my retrieval to say "I don't know", and there was nothing worth tuning

I maintain a small local-first memory tool for coding agents. One Rust binary, one SQLite database, full-text search over what previous sessions wrote down. The part this post is about is twenty-five lines long.

Search runs in three stages. Every word must match; failing that, all but one word; failing that — the stage in question — any of the words, ranked, keeping only what stands out relative to the rest of the result set. The floor is a ratio against the median bm25 of the sample. FTS5 scores are negative and lower is better, so multiplying the median by 1.2 makes the bar *stricter*, not looser. That trips everybody, including me, twice.

That third stage did a lot. Over 277 real prompts — the shape an agent actually types into this thing, not citation-shaped queries — the first two stages together came back empty 80.5% of the time. Adding the third took that to 7.2%.

And it has a price, which I measured the same day and wrote down next to the rest: **asked a question from a different project, with the read scoped to this one as it always is, it still speaks 67.9% of the time.** A relevance floor is dimensionless. It knows what an ordinary match looks like for this query. It does not know whether the store has the answer.

## The push

I posted the 80.5%-to-7.2% number on Reddit, and somebody replied that it measures *coverage, not usefulness*. Which is right. A stage that nearly always finds something drives its own empty rate towards zero by construction, and the 67.9% is the same fact seen from the other side. Whatever comes back goes into an agent's context, and it is read as relevant whether or not it is. The answers are marked `partial` and the wording never claims they matched, which I think helps less than I would like it to.

So the honest follow-up isn't to defend the number. It's: **is 67.9% a badly chosen operating point?** Maybe an absolute bm25 cut, sitting next to the relative one, would let the stage stay quiet when there is nothing there.

That was the hypothesis. What came back was far smaller than it needed to be.

## Setup

A copy of my real store: 4,448 memories across 18 projects. Reads are scoped to one project by default, and every query in this post was scoped to `leteo` — so the corpus each question actually ran against is the 359 memories filed under it, not all 4,448. That is worth holding on to when you get to the part about common words.

Two sets of real prompts, both asked against `leteo`:

- **home** — 296 prompts recorded while working on `leteo` itself
- **control** — 399 prompts from two other projects

If bm25 carries any information about whether an answer exists, home should speak more often than control at some threshold, on some rule. That is the whole test.

One detail mattered more than it sounds: the probe runs the product's **own** query builder. Not a copy of it — the actual SQL builder, weights, sample depth, floors and the ranking predicate, re-exported behind a Cargo feature that is off by default. I had been caught by this once already. An earlier harness kept its own copy of the ranking query and its own idea of what counts as a letter — described in its comment as "the partition the binary uses", which it was not. It treated Greek, Cyrillic and the Spanish ordinals (ª, º) as separators where the binary treats them as letters. Over 4,055 memories that one difference moved top-1 title accuracy from 78.3% to 80.1%. The harness was measuring a search nobody runs, which is the exact trap its own comment warned about.

The related rule of thumb, from a separate mess: timing does not transfer between SQLite builds; ranking does; query *construction* does not transfer at all.

## Result

Percentage of prompts where the stage returned anything. The probe drives that stage directly rather than running a whole search, so these are the stage's own rates, not what a user would see end to end — in the product it only runs when the two stricter stages have already come back empty:

```
rule                                      home       control
shipped floor (median * 1.2)              89.2%      90.2%
absolute cut, -1 to -10.75                97.3→72.0  98.0→67.2
median - best >= D, 0 to 3.25             97.3→73.6  98.0→68.9
shipped ratio swept, 1.0 to 2.3           97.3→ 9.8  98.0→ 7.5
```

The sweeps top out at 97.3% and 98.0% rather than 100% because a couple of percent of prompts never produce enough candidates to have a distribution at all, and the stage stays quiet on those whatever the rule is.

Four rules, each swept across its whole useful range. At the floor that actually ships, **control speaks slightly more often than home** — 90.2% against 89.2%. The columns cross.

They do not sit on top of each other everywhere, and I am not going to pretend they do, because the table says otherwise. At the strict end of the absolute cut home leads control 72.0 to 67.2, and on the distance rule 73.6 to 68.9. That is a lean in the direction the hypothesis predicted, and it is the largest one the bm25 rules produce anywhere. It is worth being careful about what it would cost, because there are two baselines you could measure that against and I want to give the hypothesis the friendlier one.

Measured inside the sweep, from its own loose end, the trade is flat: getting to that point makes the stage 31 points quieter on questions it cannot answer and 25 points quieter on questions it can. But that is not the choice in front of me. The real choice is against the floor that ships, and there the arithmetic is better for the hypothesis than I first wrote: moving from the shipped floor to the strict absolute cut costs home 17 points of speaking and costs control 23. The gap swings from a point behind to nearly five ahead.

So the honest version is not "it does nothing". It is: you pay about three points of real coverage for every point of separation you gain, and you spend seventeen of them to end up with a lead that, as below, is inside the noise. That is not a filter. It is a volume knob with a slight tilt.

The rough check does not rescue it either. At those sample sizes a 4.8-point difference is about 1.4 standard errors — the size of gap noise hands you around one time in six. And that is the generous reading, because I did not go looking for that comparison in advance: it is the widest gap the bm25 sweeps produced, picked out after running four rules across dozens of thresholds, which is the setup that manufactures a 1-in-6 event on demand. I would not ship a threshold on it, and I would not believe someone else's.

Before anyone else catches it: 90.2% here and the 67.9% I quoted earlier are not the same measurement. That one used a different control set, six days earlier, against a smaller store. Comparing across them would be exactly the mistake this post is about — the only comparison that means anything is *within* the table, where both columns came out of the same run against the same store.

I also tried the thing the *second* stage already believes. There's a comment in that function saying a relevance floor was the wrong instrument there, because what separates a genuine rescue from noise is not the shape of the score distribution but how many of the asked-for words were actually found. So: mean word coverage — the share of a question's words the project has ever seen — comes out at 84.3% for home and 82.7% for control. Adding a coverage requirement on top of the shipped floor, and turning it up until it demands ≥90% of the words, opens the widest gap I found anywhere: 36.1% against 24.1%. Twelve points, which is more than twice what the bm25 rules could manage. Priced the same way as above — against the floor that ships, where home speaks 89.2% — it costs home 53 points to swing the gap by 13. Four points of coverage per point of separation, against three for the absolute cut.

And this one is not noise. Run the same rough check as before and twelve points at those sample sizes is about three and a half standard errors, which is not a thing chance hands you. So the comment in that function was onto something real: word coverage does carry information about whether the store has an answer, in a way the score distribution does not. It just carries it at an exchange rate nobody would take. To act on it you give up more than half of everything the stage was saying, and it still speaks on a quarter of the foreign questions.

A separate pass ran the shipped floor against eight control projects one at a time. Home speaks 89.2%. The eight come in at 100.0, 97.0, 92.0, 90.8, 89.9, 87.1, 85.0 and 80.0 — five of them louder than home itself.

Two things about that row of numbers, both of which weaken it. Four of the eight have fewer than 35 prompts, so it is a spread and not eight measurements. And the 100% is the least foreign of the lot: it is 47 prompts out of 47, from a project whose own notes are partly about this tool's Reddit launch, and two of those prompts name the tool outright. The one I would actually point at is the 92%, which comes from a project about PC hardware and video encoding with no prompt mentioning this tool at all — asked of a corpus of 359 memories about a Rust program, it gets an answer nine times in ten.

## Why

Prompts are long and almost entirely made of ordinary words. Across the eight control projects, the share of a question's words that `leteo` has seen somewhere runs from 72.1% at the lowest to 84.5% at the highest — and that highest is above home's own 84.3%. Questions from another project are, word for word, as familiar to this corpus as its own. Remember too that the corpus here is 359 memories, not the whole store. Three hundred and fifty-nine memories written by developers are already enough to contain most of the vocabulary of any question a developer asks.

So the floor is not looking at whether the store knows something. It is looking at the shape of a distribution of matches on common words, and that shape is nearly the same whoever is asking. There is a little information left in it, and in word coverage there is more than a little — but every cut that quiets the questions the store cannot answer quiets the ones it can by nearly as much, so none of it is worth what it costs. The stage is not mistuned. There is no setting of it worth moving to.

## Where this is weak

**"home" is not a clean label for "the store has the answer."** Plenty of home prompts have no answer in there either. It is a proxy.

**The control is weaker than it looks.** A prompt from a neighbouring project is not noise — it is a related question, often about the same language and the same class of problem. The eight-project pass is where I went looking for a harsher one, and the harshest I have, a project about PC hardware, still got answered nine times in ten. A genuinely out-of-distribution control — questions from a domain sharing no vocabulary at all with software — is the experiment I have not run.

And I should say what those two cost, rather than waving them: both of them push in the direction of the result I got. A control made of neighbouring questions looks more like home than a real out-of-distribution control would, and a home set full of questions the store also can't answer looks more like the control. Either way, separation gets harder to see. A null result under those conditions is weaker evidence than a null result against a clean control, and anybody telling you otherwise is selling something.

What keeps me where I am anyway is not that the lean is absent. I showed you both of them: 4.8 points on the best bm25 rule, which noise could have produced, and twelve on word coverage, which it could not. It is that neither ever becomes something you could buy. Priced at the widest-gap setting of each rule, the exchange rates come out at 2.7 points of real coverage per point of separation for the distance rule, 3.0 for the absolute cut and 4.1 for word coverage. The fourth rule is not in that league at all: pushing the shipped ratio to its strict end costs 79 points of home coverage to move the gap by three. A signal you could act on has a sweet spot: somewhere the curves pull apart faster than the price rises. At every setting I priced, they didn't.

## What I'm doing about it

Nothing, and that is the point. There are other points on those curves and not one of them is worth moving to: the best of them buys a point of separation for not quite three points of coverage, and I do not want a memory that has gone quiet on a fifth of the questions it was answering in order to be marginally less talkative about the ones it can't. The floor stays exactly where it is, and the marking stays the only honest thing I can put on those answers.

The obvious next move is the one I can't take cheaply. What is left in the lexical layer is real — word coverage proved that — but it is priced out of reach, so anything better would have to come from somewhere else, and the only somewhere else is semantic. This store does not embed anything. There are `embedding` columns in the schema, inherited from Engram, which this is a reimplementation of, and never written by a single code path. That was deliberate: retrieval is FTS5 with weighted BM25 and the semantic half is a language model judging one pair at a time, which needs no vector index and keeps the whole thing one file with no server. Changing that is not a tuning pass, it is a different product decision, and I don't yet know whether it earns its keep.

What I can say is that the version of the problem I could have fixed by changing a constant does not exist. I went looking for a better number and came back with the one I already had.

## The bit that generalizes

If you have retrieval with a relative threshold, you probably also have an empty rate you are pleased with, and it probably measures coverage. The test is cheap: take real queries from somewhere your index knows nothing about, run them through the same code path as the real ones, and see whether your threshold can tell them apart.

Mine can, a little, and only at settings that cost three or four times more in real answers than they buy in silence. At the one it ships with, it leans about a point the wrong way. I would rather know that than keep quoting the other number.

---

*The tool is [Leteo](https://github.com/asanabrial/leteo) — MIT, `cargo install leteo`. The general retrieval harness lives in `tools/retrieval`; the throwaway probe for this particular sweep does not, because it opens a copy of a real store full of somebody's working days. What makes either of them trustworthy is `--features measure`, off by default, which exposes the query builder the ranking actually runs through so a measurement cannot quietly drift from the product.*
