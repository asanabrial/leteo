# I measured whether my memory tool saves tokens, and it buys something else instead

When I put this project in front of people, I ended the post with the one thing I could not test on my own: *does the recall help, or does it just add noise to your context?* Nobody answered it, and neither did I. This is the answer, and it is not the one I was hoping to write.

The short version: **on questions the code already answers, the saving is not distinguishable from noise. On questions the code cannot answer, the agent without memory spends more and never gets an answer: in August it answered something else, confidently, and in November it said it could not find it.** What you buy is not tokens — the token figures are too noisy to sell. It is not being wrong.

The experiment was run again in November on the current release, [with Leteo](#the-same-two-questions-again-in-november) and then [with five other engines beside it](#six-engines-one-command). Every engine got the store-only question right every time, where the agent without one never did; on tokens Leteo is in the middle of the six, and the difference between its own two sets of runs is larger than most of the gaps between engines.

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

## The same two questions again, in November

**2026-11-24**, Leteo 0.5.0 (`0235ac4`), `claude-opus-5-5`. The same two questions and the same two arms, three runs a cell, **twelve runs**. Everything above is from August and is left as it was; these are a second measurement, not a correction of the first.

What is the same: one read-only agent, one tool grant for both arms (read, search, list, and a shell that may run `leteo` and nothing else), ground truth written down before anything ran, answers graded by hand.

What is not, and each one matters when reading the two sets side by side:

- **The unit.** These are every model call of a run summed, input and output, as the headless agent reports them. A run of twenty calls re-reads its own context twenty times and counts it each time. The August figures were counted another way, so 461,868 here and 85,950 there are not the same agent spending five times as much. **Compare an arm with the other arm of the same month and nothing else.**
- **The isolation.** Each run was a fresh headless session with no hooks, no MCP servers and no user settings. The arm without memory could not have reached the store: the command was not on its path.
- **The checkout.** A copy of the tree with no git history and with this page removed, because this page states both questions and says where each answer is.
- **The store.** One project's 475 memories, without the four that are write-ups of the August experiment and name the memory that answers B2.
- **The prompt.** It ended with *if you cannot find the answer, say so plainly rather than offering the nearest thing*. August's did not.

**B1 — the repository has the answer**

```
                     runs                              median    correct
  no memory     110,812 · 137,128 · 154,766           137,128      3/3
  with memory    75,502 · 204,065 · 426,894           204,065      3/3
```

**B2 — only the store has the answer**

```
                     runs                              median    correct
  no memory     384,028 · 461,868 · 684,593           461,868      0/3
  with memory   137,665 · 257,230 · 349,122           257,230      3/3
```

**On the question the code answers, memory cost more: 49% more at the median.** All six runs found the table in the doc comment of `fused_observations` and all six were right. The three with memory also queried the store, found two memories about the same decision, and reported them beside the table, with figures that do not match it and that they could not reconcile. That is the August observation again, the store holding the plan and the code holding the outcome, and here it has a price. The spread says how much to trust the 49%: the with-memory runs span 5.7x and the cheapest of the six is one of them.

**On the question only the store answers, memory cost less in every pairing: 44% less at the median, 80% less best against worst, and 9% less even taking its most expensive run against the cheapest run without it.** The two ranges do not overlap, which they did in August. Three right out of three against none out of three.

**The agent without memory was not wrong this time. It said it could not find it.** All three no-memory runs on B2 ended by saying the repository does not record the mistake, and then listed the nearest things they had found, labelled as not the answer: the two SQLite builds that differ fourfold in speed, the harness that split words differently, the hand-copied query. Those are the same neighbours the August runs presented as the answer. So the August finding, *confidently answering something else*, did not repeat, and the sentence added to the prompt is the likeliest reason; I changed the prompt, the model and the harness at once and cannot say which of them did it. What repeats is narrower and still the point: without the store the answer is not available at any price, and here the price paid for not finding it was the highest on the page.

Twelve runs cost $8.47 in all.

## Six engines, one command

The same day, the same harness, and the question the runs above cannot answer: is any of that Leteo, or is it what any store does? **Thirty-six more runs**: six memory engines, two questions, three runs a cell. The no-memory arm is the one above; it was not run again.

Each engine was loaded with the same 475 memories through its own write path and asked through its own search. Because only two of the six have a command line, the agent was given one command for all of them, `mem search "<words>"` and `mem get <id>`, which hands the words to the engine's own search tool and prints the reply as the engine wrote it. The prompt is the one above with that command in place of `leteo search`, identical for all six. Leteo was run again this way too, so that it is measured like the others: these are its second three runs of the day, and they are not the three above.

Versions: Leteo 0.5.0 without its semantic model, Engram 3.7.2, server-memory 2026.8.31, claude-mem 16.1.2, mem0 2.2.1 storing verbatim with `nomic-embed-text` embeddings, basic-memory 1.0.0. Each at its defaults; mem0 was asked for ten results because its search has no default to leave alone.

**B2 — only the store has the answer** (no memory: median 461,868, 0/3)

```
                    runs                              median    against none   correct
  Engram          207,802 · 234,842 · 382,985        234,842     49% fewer       3/3
  server-memory   230,895 · 235,006 · 340,772        235,006     49% fewer       3/3
  claude-mem      290,273 · 314,557 · 337,273        314,557     32% fewer       3/3
  Leteo           154,469 · 327,398 · 370,348        327,398     29% fewer       3/3
  mem0            324,451 · 390,714 · 446,152        390,714     15% fewer       3/3
  basic-memory    369,167 · 469,391 · 666,401        469,391      2% more        3/3
```

**B1 — the repository has the answer** (no memory: median 137,128, 3/3)

```
                    runs                              median    against none   correct
  claude-mem      102,333 · 118,621 · 196,233        118,621     13% fewer       3/3
  Leteo           122,701 · 130,836 · 158,182        130,836      5% fewer       3/3
  server-memory   131,060 · 132,393 · 160,700        132,393      3% fewer       3/3
  Engram          118,893 · 151,672 · 213,151        151,672     11% more        3/3
  mem0             73,959 · 156,387 · 179,845        156,387     14% more        3/3
  basic-memory     96,743 · 166,046 · 227,622        166,046     21% more        3/3
```

**Every engine got B2 right every time, eighteen out of eighteen, against none out of three without one.** That includes server-memory, whose search is an unranked substring match and which answers 13 of 56 queries on [the comparison's corpus](comparison.md). The agent does not ask once. A run took between 23 and 82 turns, most of them another search with the words changed, and a store that answers one query in four is enough for an agent willing to ask forty times. So the correctness finding is real and it is not Leteo's: it belongs to having the memory at all.

**Leteo is fourth of six on the question it should win, and I cannot tell you it is really fourth.** Its own two sets of three runs, a few hours apart, give medians of 257,230 and 327,398 on B2 and of 204,065 and 130,836 on B1: the same engine, the same store, 27% apart on one question and 56% on the other, and on B1 on opposite sides of the no-memory median. That is the size of what three runs cannot see, and most of the gaps between engines in these tables are smaller than it. What the tables do support is coarser. On B2 five of the six engines spent less than no memory and one spent the same. On B1 the six medians sit within 21% of the no-memory one on both sides, which is no effect.

Two things under the medians are worth more than the order:

- **The engines with the weakest search were the cheapest.** Engram and server-memory return little or nothing to most queries, so each miss costs a few dozen tokens and the agent tries again. Leteo and mem0 return several whole memories to every query, and the agent pays for all of them whether or not the right one is there: mem0's replies ran to 23,000 characters. Ranking quality, which is where Leteo leads the comparison page, is not what the token count rewards when the caller can retry for almost nothing.
- **Turns and tokens moved apart.** claude-mem took 56 to 61 turns on B2 and Engram 40 to 67, against 23 to 28 for mem0, which was the second most expensive. Many cheap questions beat a few expensive ones.

Thirty-six runs cost $23.34.

## What it costs to have it at all

A session pays the schemas and the instructions before anyone asks anything, and some of it again every time the context is compacted; the session totals at the end of each table add the replies and the per-prompt hook to that. Everything here is what the model is sent, measured by running the shipped hooks and the MCP server over a copy of a store and counting the text they return, with tiktoken's `cl100k_base` as a proxy for the model's tokenizer. It is a proxy: the real count differs, and what it is good for is setting two engines side by side under the same ruler.

Conditions, because the figures are meaningless without them: **2026-10-13**, Leteo 0.3.0 at the head of this branch (`394d997`) and Engram 3.4.0, each over a copy of the same 1,048-memory store (1,017 of them in the project the session runs in), `--tools=agent` for both servers, a 20-prompt session, and a fixed mix of one `mem_context`, six `mem_search` and three `mem_save` calls. The prompts, queries and saves came from the project the store was copied from, so they are not published; the committed ones are synthetic and give different absolute numbers.

```
                                   Leteo    Engram     Leteo vs Engram
  session start                    1,586     1,300     +22.0%
  per-prompt hook, 20 prompts      1,410       421     +234.9%   (Leteo spoke on 17 prompts, Engram on 1)
  post-compaction                  1,266     1,629     -22.3%
  mem_context                      2,670     2,594     +2.9%
  mem_save (median of 5)             299       395     -24.3%
  mem_search (mean of 20)            726       909     -20.1%
  tools/list (input schemas)       5,195     5,028     +3.3%     (20 tools against 19)
  server instructions                411       412     -0.2%
                                  ------    ------
  one session, schemas upfront    17,791    18,023     -1.3%
  one session, schemas deferred   12,185    12,583     -3.2%
```

*Upfront* pays every tool's schema in every session. *Deferred* is for a client that loads schemas only when a tool is first used, so they cost nothing until then; it is the same sum without the `tools/list` and `server instructions` lines. Engram's per-prompt figure is one 387-token reminder on the first prompt plus 34 for the save nudge it fires once in a session, which the harness triggers by ageing the session in the copy.

The same session that morning, before the changes between then and this page, was 22,565 upfront and 16,959 deferred for Leteo (at `78bc173`) against 17,734 and 12,294 for Engram, a gap of +27% and +38%. Engram's own figure moved by 1.6% between the two runs with no Engram change, mostly in `mem_context`; I did not isolate why, and it is the size of the difference to treat as noise when reading the rest.

**The per-prompt hook is the row where Leteo is worst, and it is not fixed.** On this store it speaks on 17 of 20 prompts, including "continue", "yes, go with that" and "looks good, commit it", each time with three memories chosen on the words of the prompt. Against a hand-labelled sample of 150 prompts over a real store the hint is now silent on 54 of the 90 that need nothing, where it was silent on 21 before the single relevance bar, and it names a right memory on 20 of the 60 that have one, where it named 21. On the imported copy used here that bar buys nothing yet: it is silent on 2 of 10 neutral prompts, before and after.

### Measured again on 0.5.0

**2026-11-24**, Leteo 0.5.0 (`0235ac4`) and Engram 3.7.2, the same harness and the same mix. The store is not the one above: it is the 479 live memories of one project, exported from a Leteo store and imported into an empty store of each engine, so both hold exactly the same memories. The twenty prompts, six queries and three saves are again from that project and not published. Different store, different prompts and different queries, so these columns are read against each other and not against the October table.

```
                                   Leteo    Engram     Leteo vs Engram
  session start                    1,987     1,630     +21.9%
  per-prompt hook, 20 prompts        947       439     +115.7%   (Leteo spoke on 11 prompts, Engram on 1)
  post-compaction                  1,136     1,692     -32.9%
  mem_context                      2,888     2,902     -0.5%
  mem_save (median of 3)             343       437     -21.5%
  mem_search (mean of 6)           1,049        90     +1069.5%  (Engram found nothing for 5 of the 6)
  tools/list (input schemas)       5,498     6,235     -11.8%    (20 tools against 23)
  server instructions                425       405     +4.9%
                                  ------    ------
  one session, schemas upfront    20,204    15,152     +33.3%
  one session, schemas deferred   14,281     8,512     +67.8%
```

**Of Leteo's 20,204, the fixed part is 5,923** — `tools/list` 5,498 and server instructions 425 — **or 7,910 with the 1,987 of session start.** The rest is the replies (10,211: one `mem_context`, three `mem_save` and six `mem_search`), the per-prompt hook (947) and the block after a compaction (1,136). The earlier, October table is the same sum over its own store and rows.

**Leteo is the more expensive engine in this session, by a third upfront and two thirds deferred, and one row is the whole of it.** Six searches cost Leteo 6,294 tokens and Engram 538, a difference of 5,756 against a gap of 5,052 between the upfront totals. Without that row Leteo is the cheaper of the two.

The row is not Engram being terse. Its replies to five of the six queries were between 43 and 47 tokens because they held no memory at all: the queries were phrases of three to six words, and Engram's default search wants every word in one memory. Leteo answered all six, between 254 and 1,780 tokens each. So the saving on that line is the price of an empty answer, and whether it is a saving depends on whether the agent needed what was not returned. I did not grade the answers here, so this page does not say which engine's six replies were worth more; [the comparison](comparison.md) measures that on a corpus with an answer key, where Engram returns nothing for 20 of 56 queries and Leteo for 1.

What does not depend on the queries: Leteo's session start is a fifth larger than Engram's, its per-prompt hook costs twice as much over twenty prompts while speaking on 11 of them, and its block after a compaction is a third smaller. A second run of the same session gave 20,207 and 15,190 upfront, so the totals repeat to within 0.3%.

### The tool list, corrected

An earlier version of this page put the `tools/list` array at 19 tools, 47,781 bytes and about 12,000 tokens, and called it 78% of the fixed cost. Two things were wrong. The count: the agent profile has **20** tools (the default profile has 23). And the measure: the array holds more than the model is sent. A client shows the model each tool's name, description and input schema. The **output schema** of each tool is in the array too, and the model is not sent it. Counted properly:

```
  tools/list, as the server sends it        65,621 B   (20 tools)
    of which outputSchema                   40,617 B   not model-visible
    name + description + inputSchema        21,801 B   5,195 tokens
      of which descriptions                  3,564 B
      of which input schemas                16,971 B
```

The 47.8 KB figure was mostly output schemas, which is why it was far too large to be the cost of the tools. Engram publishes no output schemas, and that is the whole of the difference between the two engines' `tools/list` bytes; in the tokens the model reads they are within 3% of each other. This count takes the name, description and input schema as the model-visible part and leaves out `annotations`; that is an assumption about the client, and the one to check first if the totals look off.

Each input schema starts with the JSON-Schema dialect declaration, 57 bytes times 20 tools, 1,140 bytes: the only pure ceremony in it. The rest is mine, the descriptions I wrote and the shape of my own tools. With the output schemas out, the tool list is 29% of a session's upfront total, not 78%, and under a client that defers schemas it is none of it.

What I take from this is narrower than what I wrote before. The cost I had been trimming, the memory block, is 1,586 tokens at the start and 1,266 after each compaction, and the tool list next to it is about three times that on an upfront client. Neither is the largest line. The replies to the tools in that mix are 7,923 tokens, 44% of the session upfront and 65% deferred, and they grow with use.

### Re-running it

```bash
cargo build --release
python3 tools/memory-bench/session_cost.py \
    --leteo-bin target/release/leteo --leteo-store copy-of-leteo.db \
    --engram-bin "$(command -v engram)" --engram-store copy-of-engram.db \
    --engram-plugin-root path/to/engram/plugin/claude-code \
    --project the-project --workdir /tmp
```

Run it with a Python that has `tiktoken` for the figures above; without it the output says `~tokens (bytes/4)` and must not be compared with them. The stores are copied with SQLite's backup API into a temporary directory and each engine runs there with its own `HOME`, data directory and port, so neither the originals nor a store in use are written. `--prompts`, `--queries` and `--saves` take your own inputs. To replay the per-prompt hook over a labelled sample instead, add `--labels labels.tsv`; the committed `session_cost_data/labels.example.tsv` shows the columns and is run against `--build-fixture`'s synthetic store. `tools/memory-bench/bench_cost.py` prints the same hook costs beside its byte counts.

## Why the saving is small here, and where it would not be

I sampled twelve of the fifty memories the block delivers and asked, one agent per memory, whether the repository could recover it — source, comments, tests, README, **and the full git history**.

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
