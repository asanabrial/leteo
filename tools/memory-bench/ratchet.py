import json, math, os, shutil, statistics, sys, tempfile
from collections import defaultdict

# A fresh store every run, and set before mcpclient is imported because it reads
# the directory at import: a ratchet that reused a store would be measuring
# whatever the last run left in it.
STATE = tempfile.mkdtemp(prefix="leteo-ratchet-")
os.environ["BENCH_STATE"] = STATE

from corpus import corpus, no_answer, queries
from mcpclient import MCP, PROJECTS, call_json, rank, save_corpus
from quality import PUBLISHED_LIMIT
import inflection

# The language the corpus's memories are written in, as the product reads it from
# settings. Without it a store indexes every memory as English and the Spanish
# stemmer never runs, so the Spanish half of the corpus would be measured without
# the stemmer it was meant to be measuring.
LETEO_HOME = os.path.join(STATE, "lhome")
os.makedirs(LETEO_HOME, exist_ok=True)
with open(os.path.join(LETEO_HOME, "settings.json"), "w") as settings:
    json.dump({"language": "Spanish"}, settings)

HERE = os.path.dirname(os.path.abspath(__file__))
FLOORS = os.path.join(HERE, "floors.json")
# The first two match many short bodies; the third is a disjunction over words
# that sit in the long ones, because the preview is the thing most likely to
# change a reply's size and short bodies never reach it.
BYTE_QUERIES = {
    "search_service": {"query": "service"},
    "search_latency": {"query": "latency"},
    "search_long_bodies": {"query": "regression test commit", "match_mode": "any"},
}


class CannotRun(Exception):
    pass


def measure():
    members = corpus()
    wanted = queries()
    servers = {p: MCP("leteo", p) for p in PROJECTS}
    try:
        ids = save_corpus(servers, members)

        ranks = defaultdict(list)
        # How many questions came back with nothing at all. MRR alone cannot
        # say it: a question answered wrongly and a question not answered both
        # score zero, and an agent is told very different things by the two.
        empty = defaultdict(int)
        millis = []
        for q in wanted:
            args = {"query": q["q"], "limit": PUBLISHED_LIMIT}
            if q.get("mode"):
                args["match_mode"] = q["mode"]
            reply, _, dt = call_json(servers[q["project"]], "mem_search", args)
            found = [r["id"] for r in reply.get("results") or []]
            place = rank(found, ids[q["target"]])
            ranks[q["kind"]].append(place)
            ranks["ALL"].append(place)
            if not found:
                empty[q["kind"]] += 1
                empty["ALL"] += 1
            millis.append(dt * 1000)
        if len(ranks["ALL"]) != len(wanted) or len(wanted) == 0:
            raise CannotRun(f"{len(ranks['ALL'])} queries were evaluated and the corpus defines {len(wanted)}")

        alpha = servers["alpha-api"]
        sizes = {}
        for name, extra in BYTE_QUERIES.items():
            reply, text, _ = call_json(alpha, "mem_search", dict(extra, limit=PUBLISHED_LIMIT))
            # Fewer than the limit would measure a smaller reply than the
            # ceiling is meant to bound, and pass.
            if len(reply.get("results") or []) != PUBLISHED_LIMIT:
                raise CannotRun(f"{name} returned {len(reply.get('results') or [])} results, not {PUBLISHED_LIMIT}")
            sizes[name] = len(text.encode())
        _, text, _ = call_json(alpha, "mem_context", {})
        sizes["context"] = len(text.encode())

        # The questions the corpus cannot answer. A stage with nothing to give
        # should say so: a confident list of unrelated memories is worse than an
        # empty answer, because the agent believes what memory returns. Three
        # things are gated: how many of these questions are answered at all, how
        # many of those replies carry no caveat, and how big they are. The first
        # is what catches a row that matched only a function word, which is a
        # wrong answer whether or not the reply says the match is weak.
        asked = no_answer()
        answered, confident, no_answer_bytes = 0, 0, []
        for q in asked:
            reply, text, _ = call_json(servers[q["project"]], "mem_search", {"query": q["q"], "limit": PUBLISHED_LIMIT})
            if not (reply.get("results") or []):
                continue
            answered += 1
            no_answer_bytes.append(len(text.encode()))
            if not reply.get("hint"):
                confident += 1
        no_answer_stats = dict(
            total=len(asked),
            answered=answered,
            confident=confident,
            bytes=round(statistics.mean(no_answer_bytes)) if no_answer_bytes else 0,
        )
    finally:
        for m in servers.values():
            try:
                m.close()
            except Exception:
                m.p.kill()

    mrr = {k: sum(1 / r for r in rs if r) / len(rs) for k, rs in ranks.items()}
    hit1 = {k: sum(r == 1 for r in rs) / len(rs) for k, rs in ranks.items()}
    # Their own stores, one per language, because the language a memory is stemmed
    # in is the store's setting; they are not part of `ALL`, whose kinds are
    # measured in one store written in Spanish.
    strict = inflection.measure(STATE)
    return dict(mrr=mrr, hit1=hit1, inflection=strict, counts={k: len(v) for k, v in ranks.items()}, bytes=sizes,
                empty={k: empty[k] for k in ranks}, median_ms=statistics.median(millis), no_answer=no_answer_stats)


def ceiling(measured):
    """The next hundred above a measurement, plus one hundred.

    Rounding to the next hundred alone leaves between one and ninety-nine bytes
    of room, and a ceiling that tight breaches on a change that is not a
    regression — a word added to the hint, a field reordered in the reply.
    Between a hundred and two hundred bytes of room is smaller than any change
    worth catching and larger than any change that is not.
    """
    return (measured // 100 + 2) * 100


def propose(got):
    return {
        "raise_margin": 0.02,
        "mrr": {k: math.floor(got["mrr"][k] * 1000 + 1e-9) / 1000 for k in sorted(got["mrr"])},
        "bytes": {k: ceiling(v) for k, v in sorted(got["bytes"].items())},
        "empty": {"ALL": got["empty"]["ALL"]},
        "inflection": {k: math.floor(v["mrr"] * 1000 + 1e-9) / 1000 for k, v in sorted(got["inflection"].items())},
        # A ceiling, like `empty` and `bytes`: the count of no-answer questions
        # answered at all, the count of those with no caveat, and the size of
        # those replies, can only be allowed to fall. The first is what says a
        # question the store cannot answer is not given a partial row for
        # sharing a stop word.
        "no_answer": {"answered": got["no_answer"]["answered"],
                      "confident": got["no_answer"]["confident"],
                      "bytes": ceiling(got["no_answer"]["bytes"])},
    }


def judge(got, floors):
    breaches, raisable = [], []
    if set(floors["mrr"]) != set(got["mrr"]):
        breaches.append(f"kinds differ: floors name {sorted(floors['mrr'])}, the corpus has {sorted(got['mrr'])}")
    if set(floors["bytes"]) != set(got["bytes"]):
        breaches.append(f"byte measurements differ: floors name {sorted(floors['bytes'])}, measured {sorted(got['bytes'])}")
    if set(floors["inflection"]) != set(got["inflection"]):
        breaches.append(f"inflection languages differ: floors name {sorted(floors['inflection'])}, "
                        f"the sets are {sorted(got['inflection'])}")
    for code, floor in floors["inflection"].items():
        value = got["inflection"].get(code, {}).get("mrr")
        if value is not None and value < floor:
            breaches.append(f"strict-pass MRR {code}: measured {value:.3f}, floor {floor:.3f}")
        elif value is not None and value - floor >= floors["raise_margin"]:
            raisable.append(f"strict-pass MRR {code}: measured {value:.3f} is {value - floor:.3f} above its floor {floor:.3f}")
    for kind, floor in floors["mrr"].items():
        value = got["mrr"].get(kind)
        if value is None:
            continue
        if value < floor:
            breaches.append(f"MRR {kind}: measured {value:.3f}, floor {floor:.3f}")
        elif value - floor >= floors["raise_margin"]:
            raisable.append(f"MRR {kind}: measured {value:.3f} is {value - floor:.3f} above its floor {floor:.3f}")
    # A ceiling and not a floor: the count of questions answered with nothing
    # can only be allowed to fall. Only the total is gated; the per-kind
    # counts are printed, because a kind of eight questions moves by one.
    for kind, ceiling in floors.get("empty", {}).items():
        value = got["empty"].get(kind)
        if value is None:
            # A ceiling on something that was not measured would pass for ever.
            raise CannotRun(f"floors.json bounds empty answers for {kind!r}, which was not measured "
                            f"(measured: {sorted(got['empty'])})")
        if value > ceiling:
            breaches.append(f"empty answers {kind}: measured {value}, ceiling {ceiling}")
        elif value < ceiling:
            raisable.append(f"empty answers {kind}: measured {value} is below its ceiling {ceiling}")
    for name, ceiling in floors["bytes"].items():
        value = got["bytes"].get(name)
        if value is not None and value > ceiling:
            breaches.append(f"bytes {name}: measured {value}, ceiling {ceiling}")
    # The no-answer questions: how many come back with a reply that carries no
    # caveat, and how big those replies are. Both are ceilings, because a stage
    # with nothing to give should say less, not more.
    for name, ceiling in floors.get("no_answer", {}).items():
        value = got["no_answer"].get(name)
        if value is None:
            raise CannotRun(f"floors.json bounds no-answer {name!r}, which was not measured "
                            f"(measured: {sorted(got['no_answer'])})")
        if value > ceiling:
            breaches.append(f"no-answer {name}: measured {value}, ceiling {ceiling}")
        elif value < ceiling:
            raisable.append(f"no-answer {name}: measured {value} is below its ceiling {ceiling}")
    return breaches, raisable


def main():
    # mcpclient falls back to `leteo` on PATH, which on a developer machine is
    # an installed release, and a ratchet that quietly measured that binary
    # instead of the build under test passed against the wrong code once.
    binary = os.environ.get("LETEO_BIN")
    if not binary:
        raise CannotRun("LETEO_BIN is not set; name the binary under test")
    print(f"binary under test: {binary}", file=sys.stderr)
    if "--propose" in sys.argv:
        print(json.dumps(propose(measure()), indent=2))
        return 0
    floors = json.load(open(FLOORS))
    got = measure()
    n = got["counts"]["ALL"]
    print(f"{n} queries evaluated, median warm search {got['median_ms']:.1f} ms (printed, not gated)")
    for k in sorted(got["mrr"]):
        print(f"  {k:10} n={got['counts'][k]:3}  hit@1 {got['hit1'][k]:.3f}  MRR {got['mrr'][k]:.3f}  floor {floors['mrr'].get(k, float('nan')):.3f}  empty {got['empty'][k]:3}")
    for k, v in sorted(got["inflection"].items()):
        print(f"  strict {k:3}  n={v['n']:3}  hit@1 {v['hit1']:.3f}  MRR {v['mrr']:.3f}  floor {floors['inflection'].get(k, float('nan')):.3f}")
    for k, v in sorted(got["bytes"].items()):
        print(f"  {k:15} {v:6} bytes  ceiling {floors['bytes'].get(k, 0)}")
    na = got["no_answer"]
    print(f"  no-answer {na['answered']}/{na['total']} answered, {na['confident']} with no caveat, "
          f"mean {na['bytes']} bytes  ceiling {floors.get('no_answer', {})}")
    breaches, raisable = judge(got, floors)
    for line in raisable:
        print("RAISE?", line)
    for line in breaches:
        print("BREACH", line)
    if breaches:
        print(f"ratchet FAILED: {len(breaches)} breach(es)")
        return 1
    print("ratchet passed")
    return 0


if __name__ == "__main__":
    try:
        code = main()
    except (CannotRun, OSError, RuntimeError, KeyError, ValueError) as e:
        print(f"ratchet could not run: {type(e).__name__}: {e}")
        code = 2
    finally:
        shutil.rmtree(STATE, ignore_errors=True)
    sys.exit(code)
