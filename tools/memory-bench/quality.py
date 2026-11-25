"""The two row counts every quality instrument scores, named in one place.

The harness asks at twenty because the comparison page states that limit as
method and the floors in `floors.json` were set there. An agent does not: a
`mem_search` that names no limit gets `DEFAULT_SEARCH_LIMIT` from the product,
and until this module existed the harness had its own ten written out where it
needed one. A ranking change that landed in rows eleven to twenty, or in the
default itself, then showed as a token saving and no change in quality, because
no instrument asked at the configuration an agent uses.

Reading the default out of `src/store/search.rs` rather than restating it is
what keeps the instrument on that configuration when somebody changes it: the
next run measures the new number instead of the harness disagreeing in silence.
`test_precision.py` holds the number to the binary's own behaviour — a query
with no `limit` returns that many rows — because a constant read from source is
only a claim until something observes it.
"""
import os
import re

HERE = os.path.dirname(os.path.abspath(__file__))
SEARCH_RS = os.path.normpath(os.path.join(HERE, "..", "..", "src", "store", "search.rs"))

# How many results a search returns when the caller does not say, as the
# product declares it. One constant in Rust; this reads it rather than copying
# it, so a change there is measured rather than discovered.
def default_limit():
    try:
        with open(SEARCH_RS, encoding="utf-8") as source:
            text = source.read()
    except OSError as error:
        raise RuntimeError(
            f"cannot read the product's default search limit from {SEARCH_RS}: {error}"
        )
    found = re.search(r"DEFAULT_SEARCH_LIMIT:\s*usize\s*=\s*(\d+)", text)
    if not found:
        raise RuntimeError(
            f"{SEARCH_RS} no longer declares `DEFAULT_SEARCH_LIMIT: usize = <n>`, so the "
            "harness cannot say how many results an agent gets"
        )
    return int(found.group(1))


# The limit the comparison and the floor gate ask at. It is the harness's own
# twenty, not the product's default: the floors were set here and the page says
# so, and a floor moved with the default would be a different gate.
PUBLISHED_LIMIT = 20


def scored_limits():
    """The limits a quality run scores, the agent's configuration first.

    One query's worth of difference at each is what the reports print, and a
    regression is judged at every limit here rather than at whichever one the
    person running it remembered.
    """
    return tuple(dict.fromkeys((default_limit(), PUBLISHED_LIMIT)))
