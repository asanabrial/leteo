import json, math, os, shutil, statistics, sys, tempfile
from collections import defaultdict

# A fresh store every run, and set before mcpclient is imported because it reads
# the directory at import: a ratchet that reused a store would be measuring
# whatever the last run left in it.
STATE = tempfile.mkdtemp(prefix="leteo-ratchet-")
os.environ["BENCH_STATE"] = STATE

from corpus import corpus, no_answer, queries
from mcpclient import MCP, call_json
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
PROJECTS = ("alpha-api", "beta-web")
SEARCH_LIMIT = 20
# The two configurations the answers are scored in. `None` sends no limit at
# all, so the number is whatever the binary under test gives an agent that asks
# for none: writing 10 here would measure the harness's copy of the default and
# keep passing after a change to the real one. `SEARCH_LIMIT` stays because the
# byte ceilings and the empty-answer ceiling were measured at it, and a stage
# that only adds rows past the default is a change this catches.
LIMITS = {"default": None, "wide": SEARCH_LIMIT}
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
        ids = {}
        for mem in members:
            args = dict(title=mem["title"], content=mem["content"], type=mem["type"])
            if mem["topic"]:
                args["topic_key"] = mem["topic"]
            reply, _, _ = call_json(servers[mem["project"]], "mem_save", args)
            # A save that merged into an earlier memory would leave a target
            # without an id of its own, and every query for it would then be
            # scored against the wrong row.
            if reply.get("status") != "inserted" or reply.get("project") != mem["project"]:
                raise CannotRun(f"{mem['key']} was not inserted into {mem['project']}: {json.dumps(reply)[:300]}")
            ids[mem["key"]] = reply["observation"]["id"]
        if len(set(ids.values())) != len(members):
            raise CannotRun("two corpus memories were given the same id")

        ranks = {name: defaultdict(list) for name in LIMITS}
        # How many questions came back with nothing at all, at the wide limit.
        # MRR alone cannot say it: a question answered wrongly and a question
        # not answered both score zero, and an agent is told very different
        # things by the two.
        empty = defaultdict(int)
        millis = []
        for q in wanted:
            for name, limit in LIMITS.items():
                args = {"query": q["q"]}
                if limit is not None:
                    args["limit"] = limit
                reply, _, dt = call_json(servers[q["project"]], "mem_search", args)
                found = [r["id"] for r in reply.get("results") or []]
                target = ids[q["target"]]
                rank = found.index(target) + 1 if target in found else None
                ranks[name][q["kind"]].append(rank)
                ranks[name]["ALL"].append(rank)
                if limit == SEARCH_LIMIT:
                    if not found:
                        empty[q["kind"]] += 1
                        empty["ALL"] += 1
                    millis.append(dt * 1000)
        for name in LIMITS:
            if len(ranks[name]["ALL"]) != len(wanted) or len(wanted) == 0:
                raise CannotRun(f"{len(ranks[name]['ALL'])} queries were evaluated at {name} "
                                f"and the corpus defines {len(wanted)}")

        alpha = servers["alpha-api"]
        sizes = {}
        for name, extra in BYTE_QUERIES.items():
            reply, text, _ = call_json(alpha, "mem_search", dict(extra, limit=SEARCH_LIMIT))
            # Fewer than the limit would measure a smaller reply than the
            # ceiling is meant to bound, and pass.
            if len(reply.get("results") or []) != SEARCH_LIMIT:
                raise CannotRun(f"{name} returned {len(reply.get('results') or [])} results, not {SEARCH_LIMIT}")
            sizes[name] = len(text.encode())
        _, text, _ = call_json(alpha, "mem_context", {})
        sizes["context"] = len(text.encode())

        # The questions the corpus cannot answer. A stage with nothing to give
        # should say so: a confident list of unrelated memories is worse than an
        # empty answer, because the agent believes what memory returns. What is
        # gated is the replies carrying no caveat at all -- an empty answer and
        # one that says the match is weak both tell the agent not to rely on it.
        # The words-only stages are a separate question the issue leaves alone.
        asked = no_answer()
        answered, confident, no_answer_bytes = 0, 0, []
        for q in asked:
            reply, text, _ = call_json(servers[q["project"]], "mem_search", {"query": q["q"], "limit": SEARCH_LIMIT})
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

    def at(name):
        return {
            "mrr": {k: sum(1 / r for r in rs if r) / len(rs) for k, rs in ranks[name].items()},
            "hit1": {k: sum(r == 1 for r in rs) / len(rs) for k, rs in ranks[name].items()},
            "counts": {k: len(v) for k, v in ranks[name].items()},
        }

    wide, default = at("wide"), at("default")
    # Their own stores, one per language, because the language a memory is stemmed
    # in is the store's setting; they are not part of `ALL`, whose kinds are
    # measured in one store written in Spanish.
    strict = inflection.measure(STATE)
    return dict(mrr=wide["mrr"], hit1=wide["hit1"], counts=wide["counts"], default=default,
                inflection=strict, bytes=sizes,
                empty={k: empty[k] for k in ranks["wide"]}, median_ms=statistics.median(millis),
                no_answer=no_answer_stats)


def ceiling(measured):
    """The next hundred above a measurement, plus one hundred.

    Rounding to the next hundred alone leaves between one and ninety-nine bytes
    of room, and a ceiling that tight breaches on a change that is not a
    regression — a word added to the hint, a field reordered in the reply.
    Between a hundred and two hundred bytes of room is smaller than any change
    worth catching and larger than any change that is not.
    """
    return (measured // 100 + 2) * 100


def floors_of(measured):
    return {k: math.floor(measured[k] * 1000 + 1e-9) / 1000 for k in sorted(measured)}


def propose(got):
    return {
        "raise_margin": 0.02,
        "mrr": floors_of(got["mrr"]),
        "hit1": floors_of(got["hit1"]),
        # The limit an agent gets when it names none. Its own floors, because a
        # change to the default can move what the first ten rows contain while
        # the twenty-row figures stay where they were.
        "default": {"mrr": floors_of(got["default"]["mrr"]), "hit1": floors_of(got["default"]["hit1"])},
        "bytes": {k: ceiling(v) for k, v in sorted(got["bytes"].items())},
        "empty": {"ALL": got["empty"]["ALL"]},
        "inflection": {k: math.floor(v["mrr"] * 1000 + 1e-9) / 1000 for k, v in sorted(got["inflection"].items())},
        # A ceiling, like `empty` and `bytes`: the count of no-answer questions
        # answered without a caveat, and the size of those replies, can only be
        # allowed to fall.
        "no_answer": {"confident": got["no_answer"]["confident"],
                      "bytes": ceiling(got["no_answer"]["bytes"])},
    }


def check_floors(label, measured, floors, margin, breaches, raisable):
    """Compare one section, and refuse a floor set that names other questions.

    A floor on a kind the run did not measure would pass for ever, and so would
    a kind the run measures that nobody wrote a floor for: the second is a
    corpus change that quietly stopped being gated.
    """
    if set(floors) != set(measured):
        breaches.append(f"{label} kinds differ: floors name {sorted(floors)}, measured {sorted(measured)}")
        return
    for kind, floor in floors.items():
        value = measured[kind]
        if value < floor:
            breaches.append(f"{label} {kind}: measured {value:.3f}, floor {floor:.3f}")
        elif value - floor >= margin:
            raisable.append(f"{label} {kind}: measured {value:.3f} is {value - floor:.3f} above its floor {floor:.3f}")


def judge(got, floors):
    breaches, raisable = [], []
    margin = floors["raise_margin"]
    if set(floors["bytes"]) != set(got["bytes"]):
        breaches.append(f"byte measurements differ: floors name {sorted(floors['bytes'])}, measured {sorted(got['bytes'])}")
    if set(floors["inflection"]) != set(got["inflection"]):
        breaches.append(f"inflection languages differ: floors name {sorted(floors['inflection'])}, "
                        f"the sets are {sorted(got['inflection'])}")
    for code, floor in floors["inflection"].items():
        value = got["inflection"].get(code, {}).get("mrr")
        if value is not None and value < floor:
            breaches.append(f"strict-pass MRR {code}: measured {value:.3f}, floor {floor:.3f}")
        elif value is not None and value - floor >= margin:
            raisable.append(f"strict-pass MRR {code}: measured {value:.3f} is {value - floor:.3f} above its floor {floor:.3f}")
    check_floors("MRR", got["mrr"], floors["mrr"], margin, breaches, raisable)
    # hit@1 beside MRR: a change that trades first place for a slightly better
    # average is exactly what MRR alone lets through.
    check_floors("hit@1", got["hit1"], floors["hit1"], margin, breaches, raisable)
    if "default" not in floors:
        raise CannotRun("floors.json has no default-limit section; the limit an agent gets is not gated")
    check_floors("default-limit MRR", got["default"]["mrr"], floors["default"]["mrr"],
                 margin, breaches, raisable)
    check_floors("default-limit hit@1", got["default"]["hit1"], floors["default"]["hit1"],
                 margin, breaches, raisable)
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
        print(f"  {k:10} n={got['counts'][k]:3}  hit@1 {got['hit1'][k]:.3f}  MRR {got['mrr'][k]:.3f}  "
              f"floors h@1 {floors['hit1'].get(k, float('nan')):.3f} MRR {floors['mrr'].get(k, float('nan')):.3f}  "
              f"empty {got['empty'][k]:3}")
    for k in sorted(got["default"]["mrr"]):
        print(f"  default {k:10}  hit@1 {got['default']['hit1'][k]:.3f}  MRR {got['default']['mrr'][k]:.3f}  "
              f"floors h@1 {floors['default']['hit1'].get(k, float('nan')):.3f} "
              f"MRR {floors['default']['mrr'].get(k, float('nan')):.3f}")
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
