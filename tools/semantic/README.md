# The semantic model

Not part of the binary's code, but the origin of the one binary asset: the
embedding model under `assets/model/`. A release packs it beside the binary and
publishes it as release assets; the semantic search stage loads it from a file
and only after checking every file against the hashes the binary was built with.
Nothing ships whose origin cannot be traced to a command in this directory.

## What the model is

`sentence-transformers/static-similarity-mrl-multilingual-v1` (Apache-2.0), a
static embedding model: one vector per WordPiece token, mean-pooled, no neural
compute at query time. Three changes, all made by `build_model.py`:

1. **Truncated to 256 dimensions.** The model is trained Matryoshka-style, so
   its first columns are themselves an embedding. Truncation replaces the PCA
   step a distilled Model2Vec model would need.
2. **Quantised to int8** with `model2vec`'s own routine. It measured the same as
   f16 and half the size.
3. **Pruned to the pieces Leteo's languages use.** 49,203 of 105,879 pieces,
   chosen from text in Leteo's thirteen interface languages plus Leteo's own
   technical English. Every special token and every single-character piece in
   the Latin and punctuation ranges is kept, so a word can always be spelled and
   never falls to `[UNK]` for want of a piece. Pruning cost no measurable
   quality.

Why this model and not a distilled one, and what it does and does not cover —
including Basque, which it was not trained on — is argued with its numbers in
[`openspec/specs/search.md`](../../openspec/specs/search.md) §15.

## Reproducing the weights

```sh
python3 -m venv .venv && .venv/bin/pip install -r tools/semantic/requirements.txt
.venv/bin/python tools/semantic/fetch_corpus.py corpus
.venv/bin/python tools/semantic/build_model.py corpus out --check
```

`--check` compares the SHA-256 of the corpus and of the output files with
[`checksums.json`](checksums.json) and exits non-zero on any difference. The
inputs that could drift are pinned: the source model at a revision, the corpus
at a dataset revision, and the technical text at a commit of this repository
(`git show 058b2e3:…`), not at the working tree — a build that read the working
tree would change its weights after every edit to a document.

`fetch_corpus.py` writes each language to a temporary name and renames it when the
language is complete, so a file that exists is a whole one. After the last language
the `datasets` streaming iterator can keep the process from exiting; the files are
done and the process can be interrupted.

The shipped files were produced by this script from a corpus fetched before the
dataset revision was pinned in `fetch_corpus.py`; `checksums.json` records the
hash of every corpus file that build read, and `--check` refuses a corpus that
differs from them.

## What is here

| file | what it is |
| --- | --- |
| `fetch_corpus.py` | the first 4,000 articles of each language from Wikipedia (`20231101`), in dataset order |
| `build_model.py` | convert, truncate, quantise, prune; prints and checks the SHA-256 of the output |
| `checksums.json` | what each input and output hashes to, and what they were built from |
| `requirements.txt` | the versions it was built with |
| `hardset/` | the hard evaluation set, and `check_sets.py`, which verifies it |

The tokenizer is stored as `assets/model/tokenizer.json.gz`, deterministic gzip made by
[`pack_gz.py`](pack_gz.py) (level 9, no name, zero mtime), because 843 KB of one-line JSON
is neither reviewable nor small. `checksums.json` records the stored bytes and, under
`decompressed`, the JSON they decompress to; the second is the claim that matters, since
another zlib may write other valid bytes for the same vocabulary. To read it:
`gzip -dc assets/model/tokenizer.json.gz`.

The weights are not a Cargo input and not in the crate: the binary loads them from a
file, so the crate's size does not depend on the model's. Changing them is changing
three things together, and a test holds the first two to each other: this directory's
`checksums.json`, the hashes and `MODEL_ID` in `src/semantic/mod.rs` that the binary
pins and stores beside every vector, and the release that packs `assets/model/`. A
binary that finds files that do not match its pins does not load them. Nothing in the
build runs Python. The model's licence text is `LICENSES/Apache-2.0.txt`, named in
`NOTICE`.
