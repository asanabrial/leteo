# I measured whether my memory tool saves tokens, and it buys something else instead

When I put this project in front of people, I ended the post with the one thing I could not test on my own: *does the recall help, or does it just add noise to your context?* Nobody answered it, and neither did I. This is the answer, and it is not the one I was hoping to write.

The short version: **on questions the code already answers, the saving is not distinguishable from noise. On questions the code cannot answer, the agent without memory spends more and still answers something else, confidently.** What you buy is not tokens — the token figures are too noisy to sell. It is not being wrong.

## The setup

Somebody replying to that post gave me the protocol, and it was better than mine. *Same repo, same task, run it twice, memory on and memory off. Don't score the answers, they're too easy to read charitably. Count operations instead.*

So: two question archetypes, two arms, three repetitions each — four in one cell, for a reason I explain below. **Thirteen runs in total.** Both arms run the same read-only agent, with the same tool grant and the same shell — so both pay the same fixed cost, including the tool schemas below. What differs is that one arm was given the command that queries the store and told to use it, and the other was told not to.

- **B1** — a question the repository answers: *was ranking by literal, unstemmed match ever tried on its own, what was measured, and why was that route not taken?* The answer is in a doc comment in `src/store/search.rs`.
- **B2** — a question only the store answers: *what measurement mistake once made the search look four times worse than it is?* The answer is in one memory and nowhere in the tree.

Correctness was graded by hand against ground truth written down **before** anything ran. That turned out to matter more than I expected, and I come back to it.

## The runs

Tokens per run, and whether the answer was right.

**B1 — the repository has the answer**

```
                     runs                          median    correct
  no memory     31,701 · 37,537 · 47,802           37,537      3/3
  with memory   22,892 · 32,409 · 82,920           32,409      3/3
```

**B2 — only the store has the answer**

```
                     runs                                    median    correct
  no memory     27,394 · 85,950 · 98,733                     85,950      0/3
  with memory   29,996* · 55,189 · 71,037 · 100,417          63,113      4/4

  * off-protocol: its prompt asked for one field the others did not
```

## Three numbers, one dataset

Here is the part I want to be blunt about, because it is the whole reason this document exists.

```
  best with memory vs worst without      -70%*  (B2)     -52%   (B1)
  median vs median                       -27%*  (B2)     -14%   (B1)
  worst with memory vs best without      costs 3.7x as much  (B2)

  * depends on the off-protocol run; without it, -44% and -17%
```

All three come out of the runs above. I can write "saves 70%", "saves 14%" or "costs three times as much" and every one of them is true of some pair. The spread inside a single cell reaches 3.6x — as wide as the widest gap between the arms, which is the 3.7x on the line above it. **On B1 the median difference is 14% and I would not defend it: it is smaller than the noise it sits in.**

The 70% is real and it has a cause worth naming, which is why I am keeping it rather than hiding it: that run made **one** query, opened **zero** files, and finished in seventeen seconds, because the first search it tried landed on the right memory. That is the tool working exactly as designed. What varies between runs is not whether memory helps — it is whether the agent's first query is well aimed. When it is, one call is enough. When it is not, the agent chains up to nine and the saving evaporates.

That run used a prompt marginally different from the other three in its cell — it asked for one extra field — so it is off-protocol and I say so wherever its numbers appear. I counted it as a fourth repetition and I counted it **at both ends**: with it, the B2 best case is 70% and the median is 27%. **Without it, on the strict protocol, the best case is 44% and the median is 17%.** Both pairs are in this document; the strict pair is the conservative one and a reader who trusts only it loses nothing I would argue with.

## What actually separates the arms

Not the tokens. The correctness.

On B2, the arm without memory went **0 for 3** — and all three declared themselves satisfied. Each one found a real, documented mistake that was not the one being asked about: a tokenizer that treated `ª` and `º` as separators, a hand-copied SQL statement that drifted from the product's, a warning about comparing two experiments. Three plausible neighbours, three confident wrong answers.

**The cheapest of the seven runs in B2 — 27,394 tokens, no memory — is one of the wrong ones.** It was cheap *because* it stopped early at something that looked right. (The cheapest run of the whole thirteen is 22,892, in B1, and it *had* memory and was right — which is the opposite lesson and belongs on the same page.) The honest form of the warning is narrower than I first wrote it: **on a question the code cannot answer, measuring tokens alone rewards the arm that gets it wrong fastest.**

## What it costs to have it at all

A session pays this before anyone asks anything. Byte counts are measured; the token column is bytes divided by four, which is a rule of thumb and not a tokenizer.

```
  MCP tool schemas (19 tools, agent profile)   48,339 B   ~12,085 tokens
  MCP server instructions                       2,250 B         562
  session-start directive                         925 B         231
  the memory block itself (50 memories)        10,170 B       2,542
                                               --------   -----------
                                               61,684 B   ~15,420 tokens
```

(The column rounds per row, so it sums to 15,420 where 61,684/4 is 15,421.)

The schema figure is the one [`openspec/specs/mcp-tools.md`](../openspec/specs/mcp-tools.md) publishes and owns.

**78% of that is the tool schemas**, and I am not going to pretend they belong to somebody else. [`openspec/specs/mcp-tools.md`](../openspec/specs/mcp-tools.md) already calls that figure what it is — *the largest fixed cost Leteo imposes* — and breaks it down there. Only one thing in it says nothing about my tools: the JSON-Schema dialect declaration, which that spec calls *the only pure ceremony left* and prices at 610 tokens. Even that is one line per schema, so it grows with how many tools I expose — what a server pays regardless is the declaration, not its size. Everything else is mine: the descriptions I wrote, the keywords carrying the shape of my own nineteen tools, and the names and structure under them. So the split is not schemas-versus-memory. It is 12,085 tokens of tool surface against 3,335 of context, and **both of those are mine**.

That is the part worth taking away. I had spent weeks trimming the block's 2,542 tokens and had never once measured the 12,000 sitting next to it. The spec records what it took to get that number this small, and argues against cutting it further; either way it was never somebody else's number.

## Why the saving is small here, and where it would not be

I sampled twelve of the fifty memories the block delivers and asked, one agent per memory, whether the repository could recover it — source, comments, `openspec/`, tests, README, **and the full git history**.

```
  FULL       7 of 12      the substance is recoverable
  PARTIAL    4 of 12      the outcome is there, the road to it is not
  NONE       1 of 12      the code contradicts it outright
```

One of those FULLs is circular — I wrote that memory the same day by reading the code — so the figure I would quote is **6 of 11**.

The pattern in the PARTIALs is the same every time: **the repository keeps the result and the store keeps the road.** What was tried and rejected does not get committed. The clearest case was a memory recording that I had proposed a set of guards, been told they were unwanted, and reverted them: the guard that survived is in the tree since the first commit, and the proposal, the refusal and the reversal appear nowhere — not in the log, not in dangling commits, not in the reflog. What you said no to exists in one place only.

That is also why the number here is a floor rather than an average. This repository is 22.8% comments, keeps its specs current, and writes its reasons into commit messages. The store is competing against an author who already writes down what it would have remembered. In a codebase that does not do that, the FULL share drops and the memory wins more — **but I have not measured that, so I am not claiming it.**

## The liability I was not looking for

Recoverability and staleness are different axes, and the second one is worse. Only one of the twelve is contradicted so thoroughly that the repository is a better source than the memory — that is the NONE above. But **two** of the twelve assert something the code contradicts *today*, and both ride in the block that opens every session; the second is recoverable enough to grade PARTIAL and wrong anyway.

One says a partial index was reverted as useless, that it belonged on an expression rather than a raw column, and that the raw column would not work. The migration says the opposite in all three respects, in a comment that ends *"only this one is used by the statement the code prepares"*. The other names a Windows configuration path that the code records as a bug already fixed.

Three of the twelve also cite commit hashes that resolve to nothing.

None of that is caught by anything. Review windows are a calendar, and the calendar was never what made these wrong — the world moved underneath them. Contradiction judging only fires when another *memory* disagrees, and here the thing that disagrees is the code. The cheap half of the fix is obvious in hindsight and does not exist yet: check that the paths, constants and commit hashes a memory cites still resolve. On this sample that alone would have raised its hand four times out of twelve — three dead hashes and one stale path.

## Where this is weak

**n = 3**, thirteen runs, two question archetypes, one repository, one model. This is a direction, not a magnitude, and the variance is large enough that I would not defend any single percentage as *the* number.

**"The repository has the answer" is a proxy** for what a real working day looks like, and I do not know the real mix. That mix is what decides whether this pays for itself, and it is the one thing I have not measured.

**The grader was me.** I wrote the ground truth before running, which is the part that matters, but I also built the questions, and one of them turned out to be answerable from the code when I had predicted it would not be. I found that by running it rather than by thinking about it, which is the argument for running things.

## What I am doing about it

Publishing this, and not adjusting the number. What I will say out loud is **four right answers out of four where the same agent without memory got none out of three** — three out of three against none out of three if you drop the off-protocol run, which is the same answer in a smaller sample. That is the finding that survives the variance. Beside it, and only beside it: up to 70% fewer tokens when the first search lands, 27% median on B2, 44% and 17% if you keep only the strict protocol, and *costs 3.7x as much* if you pick the opposite pair from the same thirteen runs.

If a memory tool tells you it saves you half your tokens and does not show you its runs, its baseline and its error bars, that number came from choosing a pair. Mine did too. The difference is that this page shows you which pair.

---

*The tool is [Leteo](https://github.com/asanabrial/leteo) — MIT, `cargo install leteo`. Everything above was measured against a copy of a real store of 4,756 memories across 27 projects; the real one was never opened. The companion write-up is [*there was nothing worth tuning*](nothing-worth-tuning.md), about the retrieval floor that produces the searches counted here.*
