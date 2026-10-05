"""Do the paraphrases and translations really share no content word with their target?

The English paraphrases are the part of the hard set that claims to be free of the
target's own words, so the claim is checked rather than trusted. Words are split the
way Leteo's `prompt_terms` splits (alphanumeric or `_`, three characters or more,
lowercased); function words of the thirteen languages are ignored. Two words are
shared when equal after folding accents, or when both have five characters or more and
begin with the same five (so `connection` and `connections` count).

Before anything else it checks that the data files are the ones `checksums.json`
records, byte for byte: the data is 237 KB of generated questions that no reader
reviews line by line, so what stands in for the review is that it is the set that
was measured, and that the invariant above holds of it.

usage: check_sets.py        exits 1 and lists every violation, 2 on a changed file
"""

import hashlib
import json
import os
import sys
import unicodedata

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "..", "..", "engram-bench"))
import corpus as C  # noqa: E402

STOP = set(
    """the and for with that this from into over under when what which who whom whose why how where was were are is be been being
has have had does did not but all any can could should would will may might must our ours their them they then than there these those
its it's you your yours we us he she his her him one two three also only just very more most some such each other about after before
again against because between both down during few further here off once out own same too until while above below through very per via
el la los las un una unos unas de del al y o u que en por para con sin sobre entre se su sus lo le les es son fue era muy mas más como
cuando donde qué por porqué porque este esta estos estas ese esa eso nos nuestro nuestra no si sí ya hay ha han
els les una uns del dels amb per que com quan on aquest aquesta això són més
der die das den dem des ein eine einen einem einer und oder mit für von bei aus nach ist sind war wurde wird nicht auch wie wenn wir unser
le les des une du et ou avec pour par dans sur sont est pas plus que qui quand comment pourquoi nous notre
il lo gli una uno del della dei delle con per che non come quando perché sono noi nostro
het een van met voor door bij uit naar niet ook hoe wanneer waarom wij onze zijn werd
na nie się jest są jak dla przez kiedy dlaczego czy nasz
um uma uns umas com por para que não como quando porque são nós nosso foi
și cu pentru din care nu este sunt cum când nostru
och med för att som inte hur när varför vår vara blev
eta ez da dira bat zer nola zergatik noiz gure""".split()
)


def words(text: str) -> set[str]:
    found, current = [], []
    for char in text + " ":
        if char.isalnum() or char == "_":
            current.append(char)
        elif current:
            found.append("".join(current))
            current = []
    return {w.lower() for w in found if len(w) >= 3 and w.lower() not in STOP}


def fold(word: str) -> str:
    return "".join(c for c in unicodedata.normalize("NFKD", word) if not unicodedata.combining(c))


def shared(query: str, document: str) -> set[str]:
    asked = {fold(w) for w in words(query)}
    held = {fold(w) for w in words(document)}
    return {
        a for a in asked for b in held
        if a == b or (len(a) >= 5 and len(b) >= 5 and a[:5] == b[:5])
    }


def verify_data() -> bool:
    """Every file under data/ is exactly the one recorded, and no other is there."""
    recorded = json.load(open(os.path.join(HERE, "checksums.json")))["files"]
    folder = os.path.join(HERE, "data")
    actual = {
        name: hashlib.sha256(open(os.path.join(folder, name), "rb").read()).hexdigest()
        for name in sorted(os.listdir(folder))
    }
    ok = True
    for name in sorted(set(recorded) | set(actual)):
        if recorded.get(name) != actual.get(name):
            ok = False
            print(f"data/{name}: recorded {recorded.get(name)}, found {actual.get(name)}")
    return ok


def main() -> int:
    if not verify_data():
        print("the hard-set data is not the data checksums.json records")
        return 2
    targets = {t["key"]: t for t in C.TARGETS}
    english = [(q, targets[q["key"]]) for q in json.load(open(os.path.join(HERE, "data", "para_en.json")))]
    bad = 0
    for q, target in english:
        hits = shared(q["q"], target["title"] + " " + target["content"])
        if hits:
            bad += 1
            print(f"para_en: {q['key']}: {q['q']!r} shares {sorted(hits)}")
    print(f"{len(english)} English paraphrases checked against their English target, {bad} share a content word")

    # The translated stores keep the English paraphrases and replace the memory
    # with its translation. A cognate or a technical term the translator left
    # alone (`permission`/`permiso`, `FOIT`) is a shared word there, and is not
    # a defect of the set: the claim is made of the English store, and these are
    # counted so nobody has to take that on trust.
    cognates = 0
    for lang in "de es eu gl".split():
        translated = {t["key"]: t for t in json.load(open(os.path.join(HERE, "data", f"mem_{lang}.json")))}
        for q, _ in english:
            target = translated[q["key"]]
            if shared(q["q"], target["title"] + " " + target["content"]):
                cognates += 1
    print(f"{cognates} of {4 * len(english)} paraphrase/translation pairs share a word through a cognate or a kept term (informational)")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
