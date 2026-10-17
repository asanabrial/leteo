# Development tools

Not part of the binary. Nothing here ships.

Two small crates, each its own workspace, and one Python harness. `mutate`
depends on nothing from `leteo` on purpose: it edits `src/` and then runs `cargo test` on it, and built
as part of the crate it mutates it would be rebuilt *from the mutated source*
before the suite could run. `retrieval` does depend on `leteo`, and that is
equally deliberate — it measures the ranking statement the product issues
rather than a copy of it.

## `mutate` — does the guard actually guard?

A test that passes is not evidence. A test that **fails when you break the
thing it is about** is. This breaks each invariant in `guards.json` on purpose,
runs the whole suite, and reports which tests noticed.

```bash
cargo run --release --manifest-path tools/mutate/Cargo.toml -- tools/guards.json
```

It exists because doing this by hand failed three times in one session, each
time looking like success:

- `cargo fmt` had reformatted the line the patch was matching on, so the edit
  silently did nothing and the *unmodified* code passed.
- A `cargo test "one two"` filter matched no tests at all. Zero ran; it said
  `ok`.
- A guard passed with its subject broken, because a fallback quietly supplied
  the right answer and the fast path had become dead code.

So it asserts the mutation landed, runs the full suite rather than a filter,
and treats a surviving mutation as a failure. A guard that survives its own
invariant being removed is not a guard.

Add a case when you add a guard:

```json
{"name": "what is being broken", "file": "src/…", "old": "exact text", "new": "replacement"}
```

`old` must match the file **after** `cargo fmt`, or the run reports
`NOT APPLIED` rather than pretending.

### Cost

One full suite run per case, so a sweep of 27 takes roughly ten minutes. This
is a pre-release check, not something to run per edit — narrow `guards.json` to
the cases near what you changed while iterating, and run the whole file before
shipping.

Progress prints as it goes, one line per case with its elapsed time.

### It breaks the working tree on purpose

While a case runs, the repository really is broken — that is the whole method.
Two things follow, and both cost real time before they were handled:

**Do not run anything else against the repo while a sweep is going.** A
`cargo test` racing a background sweep reported four failures that were not
real, and the obvious next move is to go and "fix" a bug that does not exist.

**A run that is killed leaves a mutation applied.** `tools/.mutate-in-flight.json`
holds the file being edited and its original bytes, so an interrupted run leaves
something the next one repairs on startup, saying so:

```
repaired src/store/search.rs: a previous run was interrupted mid-mutation
```

If that file exists, the tree is not what it looks like. Run the tool again — or
restore from the `original` it holds — before believing any test result.

### SURVIVED is not the same as unguarded

Some tests carry `#[ignore = "requires TEST_DATABASE_URL …"]`, including the
one that proves a tenant cannot reach another tenant's project. They do not run
locally; CI runs them against a real PostgreSQL service.

So a local sweep reports SURVIVED for anything only those tests cover. Five
isolation mutations did exactly that — allow every project grant, accept any
auth scheme — and the boundary was fine. The tool now prints the ignored count
before the verdicts for that reason. Read SURVIVED as **"nothing that ran
noticed"**, and check whether an ignored test covers it before concluding
anything.

### An ambiguous pattern is not a case

`old` must match exactly once. `.replace(..., 1)` takes the first occurrence,
so a pattern appearing twice mutates whichever copy comes first — silently, and
usually not the one meant. Five queries in `store/relations.rs` share a join
clause; a case aimed at the caveat query hit an unrelated one and reported
SURVIVED, which meant nothing.

The tool now refuses those as `AMBIGUOUS` and says how many matches it found.
Add surrounding lines until the pattern is unique.

### A mutation that hangs

Some mutations make a test loop instead of fail — removing the progress check
from the repository import leaves every round retrying the same unappliable
chunk. `cargo test` then never returns and the sweep stops dead, with no
verdict for that case or any after it.

Each run is bounded (`SUITE_TIMEOUT`, five minutes). A run that does not finish
counts as caught and says so: a hang is a test noticing, not a reason to wait.

## `retrieval` — can the store find what it holds?

Every question is built from a memory the store already has, so the answer that
should come back is known without anybody labelling anything.

```bash
cargo run --release --manifest-path tools/retrieval/Cargo.toml -- ~/.leteo/leteo.db
cargo run --release --manifest-path tools/retrieval/Cargo.toml -- copy.db \n  --weights "20.0, 0.3, 0.0, 0.0, 0.0, 6.0"
cargo run --release --manifest-path tools/retrieval/Cargo.toml -- copy.db --rerank
```

Read-only, against the database you name. It reports title-shaped and
body-shaped questions separately, each against two independent draws, because
the gap between those four numbers is the only thing that makes a weight change
believable.

`--rerank` prints the shipped ordering beside the same statement with the Engram
pin/recency/stability factor as its sort key, over the same draws. The factor is
not adopted.

### The trap, which has now caught two attempts

Questions drawn from titles reward weighting titles. Sweeping the bm25 vector
against a title-drawn set found `(title 20, content 0.3, topic 6)` worth +3.3
MRR points — **entirely fake**, and only visible by asking the same vector
questions drawn from bodies, where it lost.

Then it caught the fix for it. Body questions taken from the *top* of a body are
title questions wearing another hat: memories are written lead-sentence first
and that sentence restates the title. Measured on a real store, the same vector:

| | shipped | title-tuned |
|---|---|---|
| bodies, from the top | 0.9557 | 0.9673 (+0.012) |
| bodies, past the lead | **0.9778** | 0.9718 (−0.006) |
| bodies, past the lead, held out | **0.9708** | 0.9659 (−0.005) |

The first row says change the weights. The other two say the shipped vector is
the Pareto point, which is what the earlier sweep concluded. `BODY_SKIP` steps
past the lead for that reason; a harness without it produces the first row and
sounds convincing.

### What it cannot tell you

Whether recall surfaces the memory somebody *actually wanted*. Self-retrieval
measures the index, not the judgement — a question nobody asked, answered by the
memory it was copied from. The ranking gap it does show is real and bounded:
title-shaped questions sit lower than body-shaped ones, and no reweighting has
closed it, because the same idea in different words is a semantic problem rather
than a lexical one.

### It cannot compare ways of choosing query words

A prompt longer than twelve words is cut to its first twelve, so nearly half of
real prompts are searched by their opening — the courtesy — and not by what
they ask. Choosing the twelve *rarest* words instead measures better here, and
keeps measuring better with the recall gate applied:

| | speaks | right when it speaks | hits overall |
|---|---|---|---|
| first twelve (shipped) | 83.3% | 79.2% | 66.0% |
| twelve rarest | 84.0% | 82.5% | 69.3% |

Do not believe it. Every question this harness asks is copied out of the memory
it expects back, so the rare words in that question are by construction the
words that identify that memory and no other. Ranking by rarity is then close
to sorting by "how nearly is this an id" — the instrument rewards the variable
under test, whatever the gate does afterwards.

The shipped choice was made against a hundred and twenty hand-labelled
prompt-to-memory pairs, where rarest-six raised top-three hits from 28% to 34%
and *lost* on precision, 20% against 23%. That is the right instrument for this
question and this is not it. Settling it needs questions somebody actually
asked, which is why `mem_save` now records the prompt a memory answers.

## `memory-bench` — a floor under search quality, and a comparison on held-out data

The ratchet's corpus is 178 synthetic memories in two projects and 117 queries of
seven kinds — paraphrase, short, Spanish, partial word, typo, several words, a
long natural-language question — each with the one memory it should find. Eight of
the Spanish questions target Spanish-written counterparts of English memories; the
ratchet names Spanish as the store's memory language in its settings, because a
store that does not indexes every memory as English. Two things
use it.

Those Spanish questions share whole words with their memories, so `porter` and the
exact index answer them and the kind cannot show the stemmer working. That is
what `inflection_sets.py` is for: for every language with a Snowball stemmer, eight
questions that reach their memory only through another inflection of a word it
holds, at least three edits from it. `inflection.py` scores each set on the strict
pass alone, in a store of its own with that language named in its settings, and the
ratchet holds a floor per language under `inflection` in `floors.json`.
`python3 tools/memory-bench/inflection.py --porter-only` runs the same stores
with English named, which is the control: a set earns its place only if it scores
far lower there. `inflection_check.py` verifies the edit distances against the typo
budget in `src/store/search.rs`, and needs `pip install snowballstemmer` to say
which words a stemmer folds together.

`ratchet.py` needs nothing but Leteo and stock `python3`. It saves the corpus
into a fresh store in a temporary directory through `mem_save`, runs every query
through `mem_search` at limit 20, and compares mean reciprocal rank per kind and
overall, and the bytes of twenty-result searches and of `mem_context`,
against `floors.json`. CI runs it as the `search-quality` job.

```bash
cargo build --release
LETEO_BIN=target/release/leteo python3 tools/memory-bench/ratchet.py
```

`LETEO_BIN` has no default here, because `leteo` on `PATH` is usually an
installed release and not the build under test.

It exits 1 and names every breach with its measured value and its floor. It
exits 2 if it could not run at all — a missing binary, a tool error, a save that
did not insert, fewer queries evaluated than the corpus defines — because a
suite that can pass having executed nothing guards nothing. Time is printed and
not gated: it does not transfer between SQLite builds, ranking does.

`floors.json` is the only place the numbers live. Floors are the measured MRR
rounded down to three decimals; ceilings are the measured bytes rounded up to
the next hundred and then by one more hundred, so a ceiling leaves between one
and two hundred bytes of room — smaller than any change worth catching, larger
than a word added to a hint or a field reordered, which are not regressions. The
one ceiling on empty answers is the measured count,
which can only fall. Since the semantic stage the kinds it can help — `spanish`,
`paraphrase`, `multiword` — are measured with it on. When a run prints `RAISE?` a kind is two points or more above
its floor. To raise floors after an improvement, or to move them on purpose,
run `python3 tools/memory-bench/ratchet.py --propose`, which prints a
replacement file under those two rules, and commit it with the change that
moved the numbers. A floor is never lowered to make a red run green without a
sentence in the commit saying what was given up and why.

A guard is checked by breaking what it protects. Disabling a relaxed search
stage failed the ratchet on the kinds that depend on it, and raising
`PREVIEW_BYTES` failed it on the byte ceilings; both were put back and the diff
of the file came back empty.

`--propose` also prints the empty-answer ceiling. A run prints how many questions
each kind came back empty for, beside its MRR: a question answered wrongly and a
question not answered both score zero, and an agent is told different things by the
two.

The second use is the comparison with other memory engines, on a **held-out**
corpus: `tools/memory-bench/comparison_corpus.py` is a different corpus from the
one the ratchet gates on, so the ratchet cannot tune against the corpus that
measures it. Each engine is an adapter in `adapters.py` behind one interface: it
starts isolated under `BENCH_STATE`, so no run reaches a real store, writes the
corpus through the engine's own write path and is asked the same queries at the
same limit through its own search path. A hit maps to a corpus memory by the id,
path or name we set where the engine keeps one, otherwise by a content rule that
is the same for every engine, and a hit that maps to nothing is counted as
unmappable. Engram keeps its `engram save` loader and Leteo adopts a copy of that
store with `leteo import --from-engram`, so ids survive and Engram's and Leteo's
numbers stay comparable with earlier runs. The deterministic tier adds
basic-memory (run through `uvx`, as an external process only: it is AGPL-3.0) and
`@modelcontextprotocol/server-memory` (through `npx`), both pinned to an exact
version in `adapters.py`. An engine whose binary, package runner or service is
missing is skipped with a named reason and has no row; an engine that starts and
then faults is `failed` with the exception named, and has no row either, so a
crash after starting is never read as a prerequisite that was absent. A fault
raised while an engine is being closed is named the same way and does not take
the rest of the run with it.

An engine whose answer is not the same twice declares how many passes it needs
(`Adapter.passes`, five for basic-memory, which fills its full-text index
asynchronously and exposes no completion signal to wait on). It is loaded into a
state of its own per pass; its head-to-head row reports the mean with the lowest
and the highest pass, the engines table the pass count, the per-kind table the
mean alone, and an engine measured once keeps its plain number. A run that faults before its
passes are done is not measured at all: the engines table names what stopped it.

One command runs the load and the benchmarks and rewrites the comparison page
from what it measured:

```bash
cargo build --release
LETEO_BIN=target/release/leteo python3 tools/memory-bench/compare.py
```

It regenerates [`docs/comparison.md`](../docs/comparison.md), which carries each
engine's measured version, the run date, the Leteo commit and the per-kind
numbers. The numbers live there rather than here, where they were dated prose and
drifted; the individual scripts (`bench_engines.py` for the head-to-head,
`load_engram.py`, `load_leteo.sh`, `bench_search.py`, `bench_any.py`,
`bench_cost.py`) still run on their own, and `compare.py` skips a step it cannot
run and says so. `python3 -m unittest discover -s tools/memory-bench -p
'test_*.py'` checks the hit mapping, the isolation and the skip handling.

`compare.py --tier llm` is the one opt-in flag for the engines that need a local
model, and it is never run by CI. It measures mem0 (two rows, `infer=False` and
`infer=True`), claude-mem and Graphiti MCP on a local Ollama and nothing else,
loads each engine from an empty store `--passes` times (three by default) and
rewrites only the LLM section of the page, with every figure as a mean and the
lowest and highest pass, the models and their digests, and the pass count. It
needs Ollama with `qwen2.5:7b` and `nomic-embed-text` (or the models named in
`BENCH_LLM_MODEL` and `BENCH_EMBED_MODEL`), `uv`, `npm`, `node`, `bun` and Docker;
whatever is missing skips that engine with the reason. Two engines are too
heavy for a default run and are listed as not measured, with the reason measured
on the machine that produced the page, unless opted in: `BENCH_MEM0_INFER=1` for
mem0 with `infer=True` and `BENCH_GRAPHITI=1` for Graphiti. Each pass first checks
for 8 GiB of free disk and a run that would leave less stops. Letta is listed and skipped
with its reason. Graphiti's FalkorDB runs in a throwaway container named
`leteo-bench-<pid>-…` on a random loopback port, with no volume, removed when the
run ends. Adding an engine is a class in `llm_tier.py` and an entry in `LLM`; the
pure logic is checked by `test_llm_tier.py` and needs none of the services.

`bench_cost.py` needs an Engram source checkout because Engram's
session-start hook is a shell script in its tree, not part of its binary.
`bench_cost.py` also prints what the per-prompt and post-compaction hooks put in
the model's context, in tokens where `tiktoken` is installed.

`session_cost.py` is the session model: what each engine puts in the model's
context over a 20-prompt session (session start, the per-prompt hook, the
post-compaction hook, the replies of `mem_context`, `mem_save` and `mem_search`,
the `tools/list` input schemas and the server instructions), totalled with the
schemas paid upfront and deferred. It is the source of the figures in
`docs/does-memory-save-tokens.md`.

```bash
python3 tools/memory-bench/session_cost.py \
    --leteo-bin target/release/leteo --leteo-store copy.db \
    --engram-bin "$(command -v engram)" --engram-store engram-copy.db \
    --engram-plugin-root path/to/engram/plugin/claude-code --project NAME --workdir /tmp
```

Both stores are copied with the backup API, read-only, into a temporary
directory, and each engine runs there with its own `HOME`, data directory and
port, so neither original is written. Use a Python with `tiktoken` for tokens;
without it the output says `bytes/4` and cannot be compared with a tiktoken run.
`--prompts`, `--queries` and `--saves` replace the synthetic inputs in
`session_cost_data/`. The model-visible part of a tool is its name, description
and input schema: `tools/list` also carries an `outputSchema` per tool, which is
most of its bytes and is not sent to the model.

`--labels FILE.tsv` replays the per-prompt hint instead: once per labelled
prompt, in a fresh session, over the store as it is (`source` other than `real`)
or cut back to how it stood at the prompt's `created_at` (`source` `real`). It
reports how often the hook is silent on prompts labelled `neutral` and how often
it names a memory from `best_memory_ids` on prompts labelled `related`, with the
tokens it cost. `--only real|synthetic` replays one source. The columns are
`id, created_at, project, source, prompt, label, best_memory_ids`, and later
columns are ignored. `session_cost_data/labels.example.tsv` is a synthetic
example; `--build-fixture OUT.db` writes the store it was labelled against, the
comparison corpus in write order, so the ids line up (a test holds them to it).
`bench_any.py` reruns Engram with its opt-in `match_mode: "any"`, which agents
only get if they ask for it.

Two limits to keep in view before quoting any of it. The comparison corpus is
held out from the ratchet, so the ratchet does not tune against it, but its
queries are still synthetic and were written knowing how both engines search;
questions real agents asked would be the stronger test. And precision is measured
only on unanswerable questions: the no-answer precision counts the replies to
questions the store cannot answer that came back empty or carried a caveat, not
whether the results an engine returns are the right ones.

## `semantic` — where the shipped model comes from, and whether it earns its keep

`tools/semantic/` rebuilds the embedding model under `assets/model/` from the model
it is derived from (`build_model.py`, with `fetch_corpus.py` for the text the
vocabulary is pruned against), checks the result against `checksums.json`, and
carries the two things that keep the stage honest:

```bash
# The model is a file the binary loads, so the ones that exercise the stage say where.
export LETEO_MODEL_DIR=assets/model
python3 tools/semantic/hardset/evaluate.py target/release/leteo
python3 tools/semantic/hardset/check_sets.py
python3 tools/semantic/check_binary_size.py target/release/leteo
cargo run --release --manifest-path tools/retrieval/Cargo.toml -- copy.db --pipeline
```

- **`hardset/evaluate.py`** asks 2,028 questions that do not use their target's words,
  once with the `semantic_search` setting off and once with it on, through one binary,
  and reports the paired difference with a bootstrap 95% interval. It fails when the
  whole set's interval does not exclude zero, or any kind falls by more than 0.02. The
  questions are LLM-generated; `hardset/README.md` says by what and how they were
  checked. The `search-quality` CI job runs it.
- **`check_binary_size.py`** fails above its byte budget (the binary is about 21 MB on Linux, and the model is a file beside it). Prove it by lowering the
  constant under the binary's size, running it, and restoring the file.
- **`check_install.sh`** builds an archive laid out as `release.yml` packs one, installs it
  with `scripts/install.sh` over `file://` into temporary directories, asks the installed
  binary's `doctor` whether the model is verified, and removes Leteo three ways
  (`leteo uninstall --yes`, `uninstall.sh` with and without the binary), failing if any
  model file or directory is left or `share/` is taken. It then runs the npm wrapper against the
  same archive over a local HTTPS endpoint: a cold install, a cache whose binary was deleted, and
  two runs at once of each. Every command runs under `env -i`, so no variable of the caller's
  (`LETEO_DATABASE` included) reaches the binary. It needs `node`, `openssl` and `python3`.
  The `search-quality` CI job runs it. Prove it by pointing `install.sh` at a different model
  directory and running it, or by taking the heal out of `npm/bin/leteo.js`.
- **`retrieval --pipeline`** asks the whole search over a *copy* of a store, with and
  without the stage. It cannot show the stage helping — those questions are built from
  their targets' words — only that it does no harm where it has nothing to do.

See [`semantic/README.md`](semantic/README.md).

## `winget` — the manifests every release submits

`manifests/` holds the three winget manifests for `asanabrial.leteo` with the
version, tag, SHA-256 and release date left as `@NAME@` placeholders, and
`render.sh <tag> <directory>` fills them for one published release, reading the
SHA-256 from that release's `SHA256SUMS`. The `winget` job of
`publish-registries.yml` renders them and hands the directory to `komac submit`.

```bash
sh tools/winget/render.sh v0.4.2 /tmp/winget-manifests
```

The package description lives here, so a change to it reaches winget with the
next release rather than by hand.
