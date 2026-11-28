"""The store the per-prompt hint is labelled against.

A third corpus, deliberately: `corpus.py` is the ratchet's tuning set and
`comparison_corpus.py` is held out from it, so a hint threshold tuned on either
would be tuned on something that is also scoring it. This one is small, mixed
English and Spanish, and every prompt in `labels.hint.tsv` was written against
it by hand.

Sixteen memories, eight per language, numbered by the order `--build-fixture`
writes them: id N is the Nth memory here. The labels name those ids and
`test_session_cost.py` holds the two together, so an edit that reorders this
list fails rather than scoring a hit against the wrong memory.

The related prompts share a distinctive word with the memory they name, because
that is what a hint is for: the store holds the answer, and the person asks
without using the tool. The neutral ones are ordinary things an agent is asked
that this store does not hold at all, in both languages, including the paella
prompt the hint was observed answering with database memories.

Both the memories and the prompts are synthetic, and the prompts were written
with the rule they measure in view -- it is a measured set, not an independent
one. A set somebody labelled without knowing what was changed would be stronger
evidence, and this does not pretend to be that.
"""

PROJECT = "hint-lab"


def M(kind, title, content):
    return dict(key=title, project=PROJECT, type=kind, title=title, content=content, topic=None)


MEMORIES = [
    M("decision", "Keyset pagination for the orders list",
      "Offset pagination re-read the whole table for deep pages. Orders now page by a (created_at, id) keyset cursor."),
    M("pattern", "Retries use exponential backoff with jitter",
      "Clients retry a failed call with exponential backoff, a 200 ms base, a 30 s cap and full jitter."),
    M("decision", "JWT signing keys rotate every thirty days",
      "The signing key is rotated on the first Monday of the month. The previous key stays valid for one overlap window."),
    M("bugfix", "Postgres ran out of connections under load",
      "Root cause: every handler opened its own pgx pool. One pool is now built at startup and injected."),
    M("pattern", "The cache is invalidated on deploy",
      "Every deploy writes a new asset version and clears the tagged cache entries, so a rollback serves the old version again."),
    M("pattern", "Structured logs carry a request id",
      "Handlers log JSON through slog. Middleware injects request_id and user_id into every line."),
    M("decision", "The deploy freeze is the last Friday of the quarter",
      "No production deploy goes out on the last Friday of a quarter. Emergency fixes need the on-call lead."),
    M("decision", "Feature flags are evaluated server-side",
      "Client-side flags leaked unreleased names into the bundle, so flags are evaluated in the API and only booleans reach the client."),
    M("discovery", "Índice parcial para los pedidos activos",
      "El 90% de las filas están archivadas. Un índice parcial WHERE archived = false bajó el índice de 3 GB a 280 MB."),
    M("bugfix", "Corregido: las facturas llevaban la fecha de un día antes",
      "La fecha se calculaba en UTC y se formateaba en hora local. Ahora se calcula en la zona horaria de la cuenta."),
    M("pattern", "El despliegue nocturno lo revisa la guardia",
      "El despliegue de las dos de la mañana lo revisa quien está de guardia, con el runbook al lado."),
    M("decision", "La nómina se calcula en la zona horaria de la cuenta",
      "El cierre de nómina usa la zona horaria configurada en la cuenta, no la del servidor."),
    M("pattern", "El webhook reintenta cada cinco minutos",
      "Un webhook fallido se reintenta cada cinco minutos, hasta seis veces, y luego queda en la cola muerta."),
    M("discovery", "El buscador ignora las tildes",
      "La búsqueda pliega las tildes al indexar, así que «índice» e «indice» encuentran lo mismo."),
    M("bugfix", "Corregido: la caché servía la versión vieja tras desplegar",
      "El despliegue no invalidaba la clave de la portada, así que se servía el HTML anterior durante horas."),
    M("decision", "Los informes se guardan en Parquet",
      "Los informes mensuales se escriben en Parquet particionado por mes; el CSV se genera sólo cuando alguien lo pide."),
]


def corpus():
    return [dict(memory) for memory in MEMORIES]
