"""A labelled set of saves: which pairs really contradict, and which only share words.

`conflict_candidates.py` saves every memory here into a throwaway store through
`mem_save` and scores the candidates each save proposed. The label it scores
against is `contradicts`: the key of the memory whose claim this one overturns,
or `None` for a save that contradicts nothing.

Two halves, and both are needed. The probes labelled `contradicts` are what
recall is measured on — same subject, different claim, which is the pair
`find_candidates` exists to propose. The unlabelled probes are what precision is
measured on: they are ordinary notes about the same project that deliberately
share words with the seeds and with each other, because a word-only matcher is
supposed to be fooled by exactly that. Half of each half is Spanish, and the
same-title pair is `queue`/`queue-pg`: two memories whose titles are identical
and whose claims are not.

Seeds are saved before the probes, so a probe's contradiction is always in the
store when it is saved. The order is fixed and identical for every binary
measured, and a probe saved later cannot propose an earlier probe whose pair was
already filed — that is the product's own behaviour on every save after the
first, not something this corpus arranges.
"""

PROJECT = "alpha-api"


def seed(key, lang, title, content):
    return dict(key=key, lang=lang, title=title, content=content, contradicts=None)


def probe(key, lang, title, content, contradicts=None):
    return dict(key=key, lang=lang, title=title, content=content, contradicts=contradicts)


# The older half of each contradiction. Nothing is measured on these saves; they
# are in the store so that the probes that contradict them have something to
# contradict.
SEEDS = [
    seed(
        "queue",
        "en",
        "Queue backend for the ingestion worker",
        "Decision: the ingestion worker reads jobs from Redis lists. The ack lives "
        "in the same key and the worker deletes it once the handler returns.",
    ),
    seed(
        "retry",
        "en",
        "The retry budget is per host",
        "Each host keeps its own counter, so a burst against one box does not "
        "consume another host's budget.",
    ),
    seed(
        "deploy",
        "en",
        "Deploys run from the trunk branch",
        "Everyone merges to trunk and the deploy job builds trunk; there is no "
        "long-lived release branch anywhere in the pipeline.",
    ),
    seed(
        "tokens",
        "en",
        "Session tokens live for 24 hours",
        "The auth service issues access tokens with a 24 hour lifetime and there "
        "is no refresh token beside them.",
    ),
    seed(
        "backup",
        "en",
        "Nightly backups run at 03:15 UTC",
        "The snapshot job is scheduled by cron at 03:15 UTC against the primary "
        "replica.",
    ),
    seed(
        "index",
        "en",
        "The search index rebuilds on every deploy",
        "Deploy jobs run a full reindex after the release step; nothing else "
        "rebuilds it.",
    ),
    seed(
        "migrations",
        "en",
        "Migrations are applied by hand in production",
        "An operator runs the SQL after the review; CI never touches production.",
    ),
    seed(
        "indice",
        "es",
        "El índice de texto completo se reconstruye en cada despliegue",
        "El trabajo de despliegue lanza una reindexación completa al terminar la "
        "publicación.",
    ),
    seed(
        "cache",
        "es",
        "La caché de sesiones vive en la memoria del proceso",
        "Cada instancia guarda sus sesiones en un mapa local; no hay almacén "
        "compartido.",
    ),
    seed(
        "reintentos",
        "es",
        "Los reintentos se cuentan por petición",
        "Cada petición lleva su propio contador en la puerta de entrada.",
    ),
    seed(
        "respaldo",
        "es",
        "El respaldo nocturno corre a las 03:15 UTC",
        "El cron de la copia se dispara a las 03:15 UTC contra la réplica "
        "primaria.",
    ),
    seed(
        "eventos",
        "es",
        "El registro de eventos se guarda en SQLite",
        "La tabla de eventos vive en la misma base que el resto de los datos.",
    ),
    seed(
        "limite",
        "es",
        "El límite de peticiones es de 100 por segundo",
        "La puerta de entrada permite 100 peticiones por segundo y rechaza el "
        "resto con un 429.",
    ),
    seed(
        "sesion",
        "es",
        "La sesión del panel dura 24 horas",
        "El panel firma una cookie con 24 horas de vida y no pide volver a "
        "entrar.",
    ),
]


# Every one of these is a save `find_candidates` is asked about. The labelled
# ones overturn the memory named by `contradicts`; the rest are ordinary project
# notes that share the vocabulary of the seeds and of each other.
PROBES = [
    # --- contradictions, English ------------------------------------------
    probe(
        "queue-pg",
        "en",
        # The identical title. A same-title memory that claims the opposite is
        # the case the issue found missed.
        "Queue backend for the ingestion worker",
        "Decision: the ingestion worker reads jobs from Postgres with SELECT ... "
        "FOR UPDATE SKIP LOCKED. Redis only carries the progress counters.",
        contradicts="queue",
    ),
    probe(
        "retry-request",
        "en",
        "The retry budget is per request",
        "Every request carries its own budget, counted by the gateway, so one "
        "slow caller cannot exhaust another's.",
        contradicts="retry",
    ),
    probe(
        "deploy-release",
        "en",
        "Deploys run from a release branch",
        "Each service cuts a release branch and deploys that; trunk is not "
        "deployed directly any more.",
        contradicts="deploy",
    ),
    probe(
        "tokens-15",
        "en",
        "Session tokens live for 15 minutes",
        "Access tokens expire in 15 minutes and are refreshed silently in the "
        "background.",
        contradicts="tokens",
    ),
    probe(
        "backup-local",
        "en",
        "Nightly backups run at 01:00 local time",
        "The snapshot follows the local clock of each region, 01:00 where the "
        "primary lives.",
        contradicts="backup",
    ),
    probe(
        "index-sunday",
        "en",
        "The search index rebuilds every Sunday",
        "A weekly cron rebuilds the index on Sunday morning; deploys leave it "
        "alone.",
        contradicts="index",
    ),
    probe(
        "migrations-auto",
        "en",
        "Migrations are applied automatically at deploy",
        "The deploy job runs the migration list before the new pods start, and "
        "nobody runs SQL by hand.",
        contradicts="migrations",
    ),
    # --- contradictions, Spanish ------------------------------------------
    probe(
        "indice-domingo",
        "es",
        "El índice de texto completo se reconstruye cada domingo",
        "Un cron semanal reconstruye el índice el domingo; los despliegues no lo "
        "tocan.",
        contradicts="indice",
    ),
    probe(
        "cache-redis",
        "es",
        "La caché de sesiones vive en Redis",
        "Todas las instancias leen y escriben las sesiones en un Redis "
        "compartido.",
        contradicts="cache",
    ),
    probe(
        "reintentos-host",
        "es",
        "Los reintentos se cuentan por host",
        "Cada host mantiene su contador y no comparte el presupuesto con los "
        "demás.",
        contradicts="reintentos",
    ),
    probe(
        "respaldo-local",
        "es",
        "El respaldo nocturno corre a la 01:00 local",
        "La copia sigue el reloj local de cada región, a la 01:00.",
        contradicts="respaldo",
    ),
    probe(
        "eventos-s3",
        "es",
        "El registro de eventos se guarda en S3",
        "Cada evento se escribe como un objeto en el bucket de auditoría.",
        contradicts="eventos",
    ),
    probe(
        "limite-1000",
        "es",
        "El límite de peticiones es de 1000 por segundo",
        "Subimos el límite a 1000 peticiones por segundo después de la prueba de "
        "carga.",
        contradicts="limite",
    ),
    probe(
        "sesion-15",
        "es",
        "La sesión del panel dura 15 minutos",
        "El panel expira la cookie a los 15 minutos y renueva en segundo plano.",
        contradicts="sesion",
    ),
    # --- unrelated saves, English -----------------------------------------
    probe(
        "oom",
        "en",
        "Worker OOM killed during the nightly batch",
        "The cgroup limit was 512 MB and the batch loaded a whole day of rows "
        "into memory before writing anything.",
    ),
    probe(
        "nplus1",
        "en",
        "N+1 query in the orders endpoint",
        "The endpoint loaded line items per order in a loop; it is one query "
        "with ANY now.",
    ),
    probe(
        "partial-index",
        "en",
        "The orders query ignores the partial index",
        "The predicate was written as a function of the column, so the planner "
        "could not use it.",
    ),
    probe(
        "backfill",
        "en",
        "Backfill job holds a table lock for hours",
        "The backfill ran in one transaction; it is batched by id now.",
    ),
    probe(
        "stale-pods",
        "en",
        "Stale pods survive the rollout",
        "The readiness gate passed before the new image was pulled, so the old "
        "replica kept serving.",
    ),
    probe(
        "replica-lag",
        "en",
        "The replica lags after the migration window",
        "A long DDL statement held the apply worker; the lag cleared on its own "
        "twenty minutes later.",
    ),
    probe(
        "timeout",
        "en",
        "Timeout set to two seconds on the payment call",
        "The upstream takes 1.4 seconds at p95, so anything slower is a genuine "
        "failure and worth retrying.",
    ),
    probe(
        "webhooks",
        "en",
        "Payment webhooks are retried with jitter",
        "The sender retries with exponential backoff and full jitter, so the "
        "receiver sees a spread rather than a wave.",
    ),
    probe(
        "login-form",
        "en",
        "The login form posts the password in clear text over http",
        "The form action was an absolute http URL on a staging host that "
        "somebody left in the template.",
    ),
    probe(
        "cookie-tokens",
        "en",
        "Auth tokens are stored in the cookie jar",
        "The test client keeps them in memory; nothing is written to disk.",
    ),
    probe(
        "build-times",
        "en",
        "Build times doubled after the compiler upgrade",
        "The new version inlines more aggressively; disabling it brought the "
        "build back to where it was.",
    ),
    probe(
        "cache-warmer",
        "en",
        "The config flag disables the cache warmer",
        "The flag is read at boot, so changing it needs a restart.",
    ),
    probe(
        "metrics",
        "en",
        "Metrics are scraped every fifteen seconds",
        "The scrape interval is set in the collector config, not in the "
        "application.",
    ),
    probe(
        "email-twice",
        "en",
        "The email notification goes out twice on a retry",
        "The send call was not idempotent and the queue redelivered after a "
        "timeout.",
    ),
    # --- unrelated saves, Spanish -----------------------------------------
    probe(
        "worker-memoria",
        "es",
        "El worker de ingesta se reinició por falta de memoria",
        "El límite del contenedor era de 512 MB y el lote cargaba el día entero "
        "antes de escribir.",
    ),
    probe(
        "consulta-pedidos",
        "es",
        "La consulta de pedidos ignora el índice parcial",
        "El predicado estaba escrito como una función de la columna, así que el "
        "planificador no podía usarlo.",
    ),
    probe(
        "pods-antiguos",
        "es",
        "El despliegue deja pods antiguos con vida",
        "La sonda de disponibilidad pasó antes de descargar la imagen nueva y la "
        "réplica vieja siguió sirviendo.",
    ),
    probe(
        "cron-respaldo",
        "es",
        "El cron de respaldo se solapa con la copia de la semana",
        "Las dos tareas caían en la misma ventana y competían por el mismo "
        "volumen.",
    ),
    probe(
        "tabla-eventos",
        "es",
        "La tabla de eventos crece sin límite",
        "Nadie purga las filas viejas; el disco llegó al 90 por ciento.",
    ),
    probe(
        "pedidos-archivados",
        "es",
        "Los pedidos archivados se leen con un escaneo completo",
        "La consulta no filtraba por la columna de archivo y recorría la tabla "
        "entera.",
    ),
    probe(
        "reintento-pago",
        "es",
        "El reintento del pago usa un temporizador de dos segundos",
        "El proveedor tarda 1,4 segundos en el percentil 95, así que lo más lento "
        "es un fallo real.",
    ),
    probe(
        "formulario-acceso",
        "es",
        "El formulario de acceso envía la contraseña sin cifrar",
        "La acción del formulario apuntaba a un host de pruebas por http que "
        "alguien dejó en la plantilla.",
    ),
    probe(
        "metricas",
        "es",
        "Las métricas se recogen cada quince segundos",
        "El intervalo se configura en el colector, no en la aplicación.",
    ),
    probe(
        "aviso-correo",
        "es",
        "El aviso por correo sale dos veces",
        "El envío no era idempotente y la cola reentregó tras un tiempo de "
        "espera agotado.",
    ),
    probe(
        "cache-catalogo",
        "es",
        "La caché del catálogo se calienta en el arranque",
        "El calentamiento recorre las categorías más visitadas y tarda unos "
        "segundos.",
    ),
    probe(
        "flag-config",
        "es",
        "El flag de configuración desactiva el calentador",
        "El flag se lee al arrancar, así que cambiarlo exige reiniciar.",
    ),
    probe(
        "migracion-pedidos",
        "es",
        "La migración de pedidos tarda seis horas",
        "La tabla tiene cuatrocientos millones de filas y se copia por lotes.",
    ),
    probe(
        "token-caduca",
        "es",
        "El token de acceso caduca sin aviso",
        "El cliente no mira la fecha de expiración y recibe un 401 a mitad de la "
        "pantalla.",
    ),
]


def memories():
    """Every save in the order the script makes it: seeds, then probes."""
    return [dict(seed, phase="seed") for seed in SEEDS] + [
        dict(memory, phase="probe") for memory in PROBES
    ]


def same_title_pairs():
    """The probes whose title is exactly a seed's — the case a word matcher misses."""
    seeds = {seed["title"] for seed in SEEDS}
    return [probe["key"] for probe in PROBES if probe["title"] in seeds]
