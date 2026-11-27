"""TARGETS carry the queries that must find them; DISTRACTORS share vocabulary
so a bag-of-words match is not enough. Deterministic (seeded)."""
import random

A, B = "alpha-api", "beta-web"

def T(key, project, typ, title, content, queries, topic=None):
    return dict(key=key, project=project, type=typ, title=title, content=content, topic=topic, queries=queries)

LONG_FILLER = (" We measured the before and after on the staging cluster, wrote the numbers into the runbook, "
    "and added a regression test so the next refactor cannot quietly undo it. The rollback plan is to revert "
    "the single commit; no data migration is involved. Follow-ups are tracked separately and none of them block release.")

TARGETS = [
 T("pgpool", A, "bugfix", "Fixed connection pool exhaustion under load",
   "Root cause: every HTTP handler opened a pgx pool instead of sharing one, so Postgres hit max_connections at ~400 rps. Now one pgxpool.Pool is built in main and injected." + LONG_FILLER,
   [("paraphrase","database ran out of connections when traffic spiked"),("partial","pgxpo"),("typo","conection pool exaustion"),("short","pgx")]),
 T("jwtrot", A, "decision", "Rotate JWT signing keys every 30 days with kid header",
   "Decision: signing keys live in KMS, tokens carry a kid header, verifier accepts current and previous key for a 48h overlap window.",
   [("paraphrase","how often do we change the token signing secret"),("multiword","jwt key rotation overlap policy kms"),("short","kid")], topic="architecture/auth-tokens"),
 T("ratelimit", A, "decision", "Use token bucket rate limiting per API key in Redis",
   "Sliding window was too expensive in Redis memory; token bucket with a Lua script gives atomic refill. 100 requests burst, 10/s sustained per API key.",
   [("longnl","what algorithm did we pick for rate limiting clients, and where is the bucket state stored?"),("partial","throttl"),("typo","rate limitting redis")], topic="architecture/rate-limit"),
 T("tzbug", A, "bugfix", "Invoices dated one day early for users in UTC-5",
   "Root cause: invoice date computed with time.Now().Truncate(24h) in UTC, then formatted in local time. Fixed by computing the date in the account's IANA timezone.",
   [("paraphrase","billing documents showing the wrong day for American customers"),("multiword","invoice timezone truncate bug")]),
 T("migr", A, "pattern", "Migrations are append-only and named with a UTC timestamp",
   "Never edit a migration that has shipped. New file per change, named 20240101T120000_description.sql, checked by CI against the applied list.",
   [("paraphrase","can I modify an old schema change file"),("partial","append-onl"),("short","sql")], topic="conventions/migrations"),
 T("grpcdl", A, "bugfix", "gRPC calls hung forever without a deadline",
   "Root cause: context.Background() passed to downstream gRPC clients, so a stuck inventory service blocked checkout goroutines indefinitely. Every outbound call now gets context.WithTimeout(ctx, 2s).",
   [("paraphrase","checkout stuck waiting on inventory service"),("typo","grpc deadlne timeout"),("longnl","why did checkout requests pile up when the inventory service stopped answering?")]),
 T("otel", A, "decision", "Adopt OpenTelemetry for tracing instead of Jaeger client",
   "The Jaeger client libraries are deprecated. OpenTelemetry SDK with OTLP exporter to the collector sidecar; sampling at 5% for prod, 100% for staging.",
   [("paraphrase","which tracing library replaced the deprecated one"),("partial","telemetr"),("short","otlp")], topic="architecture/observability"),
 T("cache", A, "discovery", "Product catalog cache stampede on cold start",
   "When the catalog cache expired, hundreds of requests recomputed it at once. singleflight.Group collapses concurrent misses into one load.",
   [("paraphrase","thundering herd when the cache expires"),("multiword","catalog singleflight stampede fix")]),
 T("s3up", A, "bugfix", "Large file uploads to S3 failed above 5 GB",
   "PutObject has a 5 GB limit. Switched to the multipart upload manager with 64 MB parts and a retry per part.",
   [("paraphrase","uploading huge files to object storage breaks"),("typo","multpart uplod s3"),("short","s3")]),
 T("nplus1", A, "bugfix", "N+1 query in order history endpoint",
   "Order history loaded line items per order in a loop: 1 + N queries. Replaced with a single query using ANY($1) and grouping in Go. p95 from 900ms to 60ms.",
   [("paraphrase","order history endpoint was slow because of repeated queries"),("partial","N+1"),("longnl","what made the order history page take almost a second and how did we fix it?")]),
 T("feature", A, "decision", "Feature flags evaluated server-side with Unleash",
   "Client-side flags leaked unreleased feature names in the bundle. Unleash runs as a sidecar, flags are evaluated in the API and only booleans reach the client.",
   [("paraphrase","where do toggles for unreleased features get decided"),("short","unleash")], topic="architecture/feature-flags"),
 T("idemp", A, "pattern", "Payment endpoints require an Idempotency-Key header",
   "Retries from mobile clients created duplicate charges. Payment POSTs now require Idempotency-Key; responses are stored for 24h keyed by it.",
   [("paraphrase","customers charged twice when the app retried"),("multiword","idempotency key payments duplicate")]),
 T("goroutine", A, "bugfix", "Goroutine leak in websocket hub",
   "Each disconnected client left a goroutine blocked on an unbuffered send channel. Fixed with a select on ctx.Done() and closing the channel on unregister.",
   [("typo","gorutine leek websocket"),("paraphrase","memory keeps growing because background workers never exit")]),
 T("es_mig", A, "decision", "Decisión: migrar la búsqueda de productos a Meilisearch",
   "Elasticsearch costaba demasiado para un índice de 200 mil productos. Meilisearch da tolerancia a errores tipográficos de serie y la latencia p95 bajó a 15 ms. La reindexación completa tarda cuatro minutos.",
   [("spanish","por qué cambiamos elasticsearch por meilisearch"),("paraphrase","why did we replace elasticsearch for product search"),("partial","meilisear"),("spanish","busqueda de productos migracion")], topic="architecture/search-engine"),
 T("es_bug", A, "bugfix", "Corregido: el cron de conciliación bancaria se ejecutaba dos veces",
   "Causa raíz: dos réplicas del worker tenían el planificador activo. Ahora el cron toma un advisory lock de Postgres antes de ejecutar; la réplica que no lo obtiene sale sin hacer nada.",
   [("spanish","conciliación bancaria duplicada"),("paraphrase","bank reconciliation job ran twice"),("partial","conciliac"),("spanish","conciliacion bancaria se ejecuta dos veces")]),
 T("es_disc", A, "discovery", "Descubrimiento: los índices parciales evitan escanear pedidos archivados",
   "El 90% de las filas de orders están archivadas. Un índice parcial WHERE archived = false redujo el tamaño del índice de 3 GB a 280 MB y las consultas de pedidos activos usan index-only scan.",
   [("spanish","índice parcial para pedidos activos"),("multiword","partial index archived orders size"),("typo","indice parcal pedidos")]),
 T("logs", A, "pattern", "Structured logging with slog and request IDs",
   "All logs are JSON via log/slog; middleware injects request_id and user_id. No fmt.Printf in handlers.",
   [("paraphrase","how should I write log lines in handlers"),("short","slog")], topic="conventions/logging"),
 T("healthz", A, "bugfix", "Kubernetes restarted pods during slow migrations",
   "Liveness probe hit /healthz which checked the database; a long migration made it fail and kubelet killed the pod mid-migration. Liveness now only checks the process; readiness checks dependencies.",
   [("paraphrase","pods getting killed while the schema upgrade ran"),("multiword","liveness readiness probe migration restart"),("longnl","why were our containers restarting in the middle of a database migration?")]),
 T("decimal", A, "bugfix", "Rounding errors in currency totals from float64",
   "Totals were summed as float64 and drifted by a cent. Money is now int64 minor units everywhere; formatting happens at the edge.",
   [("paraphrase","totals off by one cent"),("typo","rouding eror curency")]),
 T("cors", A, "bugfix", "CORS preflight failed for the admin dashboard",
   "OPTIONS requests were routed through auth middleware, which returned 401 before CORS headers were written. CORS middleware now runs first.",
   [("paraphrase","browser blocked admin panel requests with a preflight error"),("short","cors")]),
 T("hydr", B, "bugfix", "Hydration mismatch from Date rendering in SSR",
   "Server rendered dates in UTC, client in the browser locale, so React threw hydration mismatch warnings and re-rendered the whole tree. Dates now render client-only inside a useEffect gate.",
   [("paraphrase","react complains server html differs from client"),("typo","hidration missmatch"),("partial","hydrat")]),
 T("zustand", B, "decision", "Use Zustand instead of Redux for client state",
   "Redux boilerplate slowed every feature. Zustand stores per domain, server state stays in TanStack Query. Devtools via the zustand middleware.",
   [("paraphrase","which state management library did we choose for the frontend"),("short","redux"),("longnl","did we keep redux or move to something lighter for global state in the web app?")], topic="architecture/state-management"),
 T("bundle", B, "discovery", "Moment.js accounted for 280 KB of the main bundle",
   "Bundle analyzer showed moment with all locales in the main chunk. Replaced with date-fns and tree-shaken imports; main bundle down 31%.",
   [("paraphrase","what was making our javascript bundle so large"),("multiword","bundle size moment date-fns analyzer"),("partial","analyz")]),
 T("a11y", B, "pattern", "Every icon button needs an aria-label",
   "Screen readers announced icon-only buttons as 'button'. Lint rule jsx-a11y/control-has-associated-label enforces it.",
   [("paraphrase","screen reader says just button for icon buttons"),("short","aria")], topic="conventions/accessibility"),
 T("flutterweb", B, "discovery", "Flutter web CanvasKit adds 2 MB to first load",
   "CanvasKit renderer downloads a 2 MB wasm on first load. HTML renderer is lighter but text measurement differs; we keep CanvasKit and preload the wasm.",
   [("paraphrase","why is the flutter web first load heavy"),("partial","canvask"),("typo","canvaskit wasm prelaod")]),
 T("csp", B, "decision", "Strict Content Security Policy with nonces",
   "Inline scripts are allowed only with a per-request nonce; the CSP header is built per response. third-party analytics loaded via a proxied domain so the policy stays strict.",
   [("paraphrase","how do we allow inline scripts safely"),("short","csp"),("multiword","content security policy nonce analytics")], topic="architecture/security-headers"),
 T("infinite", B, "bugfix", "Infinite re-render loop in the search filters",
   "useEffect depended on an object literal recreated every render, so it set state on every render. Memoized the filter object with useMemo.",
   [("paraphrase","search page freezes because the component keeps rendering"),("typo","infinte rerender useefect"),("longnl","what caused the filters panel to keep re-rendering until the tab froze?")]),
 T("i18n", B, "decision", "Translations managed with ICU messages in Lokalise",
   "Hard-coded strings and string concatenation broke plurals in Spanish and Polish. ICU MessageFormat everywhere, synced from Lokalise in CI.",
   [("paraphrase","where do translated strings come from"),("short","icu")], topic="architecture/i18n"),
 T("sw", B, "bugfix", "Service worker served a stale app shell after deploy",
   "The service worker cached index.html with cache-first, so users kept the old bundle hashes and got 404s on chunks. index.html is now network-first and the SW calls skipWaiting on update.",
   [("paraphrase","users stuck on the old version after we deploy"),("multiword","service worker stale cache chunks 404")]),
 T("e2e", B, "pattern", "End-to-end tests use Playwright with fixtures per role",
   "Cypress was flaky with multiple tabs. Playwright fixtures log in as admin, editor or viewer once per worker and reuse storage state.",
   [("paraphrase","what do we use for browser tests"),("partial","playwr"),("typo","playwrite fixtures")], topic="conventions/testing"),
 T("img", B, "discovery", "Product images were served without width hints",
   "Lighthouse flagged CLS 0.31 on the product grid. Adding width and height attributes plus next/image sizes dropped CLS to 0.02.",
   [("paraphrase","layout jumping while product pictures load"),("short","cls"),("longnl","what caused cumulative layout shift on the product grid and how much did it improve?")]),
 T("es_web", B, "bugfix", "Corregido: el formulario de registro perdía los acentos",
   "Los nombres con tildes (José, Begoña) llegaban como JosÃ© porque el proxy reescribía el Content-Type sin charset. Se fuerza charset=utf-8 en el proxy y en el fetch.",
   [("spanish","nombres con tildes rotos en el registro"),("paraphrase","signup form mangles accented names"),("multiword","utf-8 charset proxy registration"),("spanish","acentos perdidos formulario")]),
 T("es_dec", B, "decision", "Decisión: modo oscuro con variables CSS y prefers-color-scheme",
   "Se descartó duplicar hojas de estilo. Los colores son tokens en :root, redefinidos bajo prefers-color-scheme: dark, con un interruptor que guarda la elección del usuario.",
   [("spanish","cómo implementamos el modo oscuro"),("paraphrase","dark theme implementation approach"),("short","dark")], topic="architecture/theming"),
 T("webvitals", B, "discovery", "INP regressions came from a synchronous analytics call",
   "Interaction to Next Paint jumped to 450 ms because every click ran a synchronous analytics serializer. Deferred it with requestIdleCallback; INP back to 120 ms.",
   [("paraphrase","clicks feel laggy because of tracking code"),("partial","requestIdle"),("typo","interaction next paint regresion")]),
 T("auth_web", B, "bugfix", "Refresh token race logged users out",
   "Two tabs refreshed the token at the same time; the second used an already-rotated refresh token and was rejected, logging the user out. A BroadcastChannel lock lets one tab refresh and share the result.",
   [("paraphrase","users randomly signed out with several tabs open"),("multiword","refresh token race tabs broadcastchannel"),("longnl","why were people getting logged out when they had more than one browser tab?")]),
 T("forms", B, "pattern", "Forms use react-hook-form with zod schemas",
   "Validation rules live in a zod schema shared with the API types; react-hook-form handles state. No controlled inputs for large forms.",
   [("paraphrase","how do we validate form fields"),("short","zod")], topic="conventions/forms"),
 T("fonts", B, "bugfix", "Flash of invisible text from self-hosted fonts",
   "Fonts were declared without font-display, so text stayed invisible for up to 3 s on slow networks. font-display: swap plus preload of the two main weights.",
   [("paraphrase","text not showing while fonts download"),("typo","font-dispaly swap"),("short","foit")]),
 T("webp", B, "decision", "Serve AVIF with WebP fallback from the image CDN",
   "AVIF saves 40% over JPEG for product photos; the CDN negotiates on the Accept header and falls back to WebP for older browsers.",
   [("paraphrase","which image format do we deliver"),("multiword","avif webp cdn accept negotiation")], topic="architecture/images"),
 T("storybook", B, "pattern", "Components are documented in Storybook with interaction tests",
   "Each atom and molecule has a story; play functions cover the interactive states and run in CI with the test runner.",
   [("paraphrase","where is the component documentation"),("partial","storyb")], topic="conventions/components"),
 T("memleak", B, "bugfix", "Memory leak from un-removed scroll listeners on the map page",
   "The map component added window scroll listeners on mount and never removed them; navigating back and forth piled up handlers. Cleanup in the effect's return.",
   [("paraphrase","browser tab memory grows on the map screen"),("multiword","scroll listener cleanup map leak")]),
 # The four memories behind the `stopword` kind. Each answers a question whose
 # words include function words a bag of stop words would match on its own, and
 # each has a query in both modes: the default pass a person types into
 # `mem_search`, and the `any` disjunction the issue found ranking a row that
 # shared only `de` above the memory that shared its content words.
 T("es_fuga", B, "bugfix", "Fuga de memoria en el servicio de imágenes",
   "El servicio de imágenes perdía memoria porque cada petición creaba un búfer de 4 MB que nadie liberaba. Ahora el búfer se reutiliza y el consumo bajó de 3 GB a 400 MB.",
   [("stopword","fuga de memoria"),("stopword","fuga de memoria","any")]),
 T("es_limite", B, "config", "Límite de subida de archivos grandes",
   "El límite de subida del servicio de informes es 5 GB; por encima se usa la carga multiparte con partes de 64 MB y un reintento por parte.",
   [("stopword","límite de subida"),("stopword","límite de subida","any")]),
 T("en_requests", A, "decision", "Webhook deliveries are capped at 120 requests per minute per tenant",
   "The webhook dispatcher drops anything past 120 requests per minute for one tenant and reports the overflow in the delivery log.",
   [("stopword","how many requests per minute"),("stopword","how many requests per minute","any")]),
 T("en_email", A, "decision", "Sign-in emails are delivered through Postmark",
   "Postmark is the email provider for sign-in and password reset messages; the old SMTP relay was retired.",
   [("stopword","which email provider"),("stopword","which email provider","any")]),
]

# The same eight memories written in Spanish, which is what the `spanish` queries
# for them are about. Before these existed, eight of the sixteen Spanish
# questions asked in Spanish for a memory written in English: no stemmer can
# bridge that, so the kind's ceiling was set by the corpus and not by the search.
# The English originals and their English questions are unchanged.
#
# Written as a Spanish-speaking developer would write the note rather than
# fitted to the question: some questions share no whole word with their target.
ES_COUNTERPARTS = [
 T("es_tzbug", A, "bugfix", "Corregido: facturas con la fecha de un día antes para usuarios en UTC-5",
   "Causa raíz: la fecha de la factura se calculaba con time.Now().Truncate(24h) en UTC y luego se formateaba en hora local. Se corrige calculando la fecha en la zona horaria IANA de la cuenta.",
   [("spanish","facturas con fecha equivocada por zona horaria")]),
 T("es_cache", A, "discovery", "Descubrimiento: estampida de caché del catálogo de productos en el arranque en frío",
   "Cuando expiraba la caché del catálogo, cientos de peticiones la recalculaban a la vez. singleflight.Group agrupa los fallos concurrentes en una sola carga.",
   [("spanish","estampida de caché al arrancar")]),
 T("es_idemp", A, "pattern", "Los endpoints de pago exigen la cabecera Idempotency-Key",
   "Los reintentos de los clientes móviles creaban cobros duplicados. Los POST de pago ahora requieren Idempotency-Key; las respuestas se guardan durante 24 h asociadas a ella.",
   [("spanish","cobros duplicados al reintentar el pago")]),
 T("es_decimal", A, "bugfix", "Errores de redondeo en los totales de moneda por usar float64",
   "Los totales se sumaban como float64 y se desviaban un céntimo. Ahora el dinero son enteros int64 en unidades menores en todas partes; el formato se aplica en el borde.",
   [("spanish","errores de redondeo en importes")]),
 T("es_a11y", B, "pattern", "Todo botón de icono necesita un aria-label",
   "Los lectores de pantalla anunciaban los botones con solo un icono como «botón». Es una regla de accesibilidad que hace cumplir jsx-a11y/control-has-associated-label.",
   [("spanish","accesibilidad botones con icono")]),
 T("es_i18n", B, "decision", "Traducciones gestionadas con mensajes ICU en Lokalise",
   "Las cadenas escritas a mano y la concatenación rompían los plurales en español y polaco. ICU MessageFormat en todas partes, sincronizado desde Lokalise en CI.",
   [("spanish","traducciones y plurales en la web")]),
 T("es_sw", B, "bugfix", "El service worker servía un app shell obsoleto tras el despliegue",
   "El service worker guardaba index.html con cache-first, así que los usuarios conservaban los hashes antiguos del bundle y recibían 404 en los chunks. Ahora index.html es network-first y el SW llama a skipWaiting al actualizar.",
   [("spanish","usuarios con la versión vieja después del despliegue")]),
 T("es_memleak", B, "bugfix", "Fuga de memoria por listeners de scroll sin eliminar en la página del mapa",
   "El componente del mapa añadía listeners de scroll en window al montarse y nunca los eliminaba; al navegar adelante y atrás se acumulaban manejadores. La limpieza va en el return del efecto.",
   [("spanish","fuga de memoria en el mapa")]),
]

SERVICES = ["billing","inventory","checkout","notifications","search","catalog","shipping","accounts","reports","admin"]
TECH = ["Postgres","Redis","Kafka","gRPC","S3","Kubernetes","Nginx","React","Next.js","Flutter","TypeScript","Go"]
VERBS = ["Refactored","Documented","Investigated","Tuned","Cleaned up","Split","Renamed","Upgraded","Audited","Benchmarked"]
OBJS = ["the retry policy","the config loader","error wrapping","the health endpoint","the batch job","the metrics labels",
        "the deploy pipeline","test fixtures","the API client","pagination","the CSV export","the cron schedule",
        "the cache keys","the webhook handler","the session store","the queue consumer"]
ES_TITLES = ["Revisada la política de reintentos del servicio de {s}","Actualizado {t} en el servicio de {s}",
             "Documentada la configuración de {t} para {s}","Limpieza de código muerto en {s}",
             "Investigación: latencia alta en {s} con {t}"]
TYPES = ["decision","bugfix","discovery","pattern","config"]

def distractors(n=130, seed=7):
    r = random.Random(seed)
    out = []
    for i in range(n):
        proj = A if i % 2 == 0 else B
        s, t, v, o = r.choice(SERVICES), r.choice(TECH), r.choice(VERBS), r.choice(OBJS)
        typ = r.choice(TYPES)
        if i % 6 == 5:
            title = r.choice(ES_TITLES).format(s=s, t=t)
            body = (f"Cambios en {s} relacionados con {t}. Se revisaron los tiempos de respuesta, los errores en los registros "
                    f"y la configuración de despliegue. No hubo cambios de esquema. Número de referencia {1000+i}.")
        else:
            title = f"{v} {o} in {s}"
            body = (f"{v} {o} in the {s} service that talks to {t}. Context: the previous approach was hard to test and "
                    f"produced noisy logs during deploys. Outcome: simpler code, same behaviour, request latency and error "
                    f"rate unchanged. Ticket {1000+i}.")
            if i % 9 == 0:
                body += LONG_FILLER * 3
        out.append(dict(key=f"d{i}", project=proj, type=typ, title=title, content=body, topic=None, queries=[]))
    return out

def corpus():
    return TARGETS + ES_COUNTERPARTS + distractors()

def queries():
    qs = []
    for t in TARGETS + ES_COUNTERPARTS:
        for entry in t["queries"]:
            # A third item names the match mode when the query is about that
            # mode; the default is the one a person types into `mem_search`.
            kind, q = entry[0], entry[1]
            mode = entry[2] if len(entry) > 2 else None
            qs.append(dict(kind=kind, q=q, target=t["key"], project=t["project"], mode=mode))
    return qs

# Questions the corpus cannot answer. A stage that has nothing to give should say
# so, because the agent believes what memory returns and a confident list of
# unrelated memories is worse than an empty answer.
#
# They are about people, offices, money and the law, none of which any memory
# here is about, and each names its project so the question is asked of a store
# that really holds nothing for it. They are not in `queries()`: their kind is
# their own and they have no target to rank, so they never enter the answerable
# MRR. They are the `no_answer` half of the corpus the harness charges for.
NO_ANSWER = [
    (A, "what is our parental leave policy"),
    (A, "who approves travel expenses"),
    (A, "how many vacation days do we get"),
    (A, "what is the guest wifi password"),
    (A, "when is the next company all-hands"),
    (A, "how do I request a new laptop"),
    (A, "what is the referral bonus for new hires"),
    (A, "which health insurance provider do we use"),
    (A, "what is the dress code for the offsite"),
    (A, "how do I book a meeting room"),
    (A, "what is the coffee machine brand in the kitchen"),
    (A, "when does the office close for the holidays"),
    (B, "cuál es la política de vacaciones"),
    (B, "cómo pido un portátil nuevo"),
    (B, "cuándo es la cena de empresa"),
    (B, "qué proveedor de nóminas usamos"),
    (B, "cómo se solicita una baja médica"),
    (B, "cuál es el menú de la cafetería"),
    (B, "cómo se reserva una sala de reuniones"),
    (B, "qué gimnasio tiene descuento para empleados"),
    (B, "cuántos días de teletrabajo hay"),
    (B, "quién aprueba los gastos de viaje"),
    (B, "dónde está la primera planta"),
    (B, "cuál es el horario de verano"),
    # A stop word and words no memory carries. On the words alone there is no
    # answer at all, and a partial row reached through `de` answers a question
    # nobody asked.
    (B, "receta de paella valenciana"),
    (B, "de la"),
    (A, "the"),
]

def no_answer():
    return [dict(kind="no_answer", q=q, project=p) for p, q in NO_ANSWER]

# Single absent words: correctly spelt words the corpus does not hold, of the
# shape somebody types when the store has never been told the thing they are
# asking about. They are not questions, because the correction stage reads words
# rather than a sentence, and one word is the whole label: a rewrite of any of
# them is an answer to a question nobody asked.
#
# The first seven are what a real store was measured doing — `insurance` came
# back as eight rows about `instance`, `paella` as eight about `falla`, neither
# flagged. The last three are absent here and near nothing the corpus holds, so
# they say whether the rules refuse everything or only the awkward cases.
ABSENT_WORDS = [
    (B, "paella"),
    (A, "saml"),
    (A, "insurance"),
    (B, "seguro"),
    (B, "nómina"),
    (A, "guitar"),
    (B, "muestreo"),
    (A, "photosynthesis"),
    (B, "ballena"),
    (A, "kayak"),
]

def absent_words():
    return [dict(kind="absent", q=q, project=p) for p, q in ABSENT_WORDS]

if __name__ == "__main__":
    c = corpus(); q = queries()
    from collections import Counter
    print(len(c), "memories;", len(q), "queries", Counter(x["kind"] for x in q))
