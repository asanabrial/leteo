//! Romanian.

use super::Screens;

pub const SCREENS: Screens = Screens {
    found_engram: "S-a găsit o instalare Engram",
    engram_counts: "{observations} memorii, {sessions} sesiuni, {prompts} prompturi, \
                    {relations} relații",
    adopt_question: "Preiei aceste memorii în Leteo?",
    adopt_yes: "Da, preia-le",
    adopt_no: "Nu, pornește gol",
    choose_agents: "Ce agenți ar trebui să configureze Leteo?",
    will_be_removed: "va fi eliminat",
    will_be_installed: "va fi instalat",
    hooks_question: "Instalez hook-urile de ciclu de viață care automatizează memoria \
                     în {agents}?",
    yes: "Da",
    hooks_no: "Nu, doar instrumentele MCP",
    voice_question: "Cât ar trebui să spună {name} cu voce tare?",
    voice_all: "salut, sugestii, capturi și mementouri",
    voice_reminders: "doar mementoul de salvare",
    voice_quiet: "nimic, nici măcar mementoul de salvare",
    interface_question: "În ce limbă ar trebui să-ți vorbească Leteo?",
    interface_hint_first: "  Ecranele Leteo: panourile, meniurile, ajutorul și pagina aceasta.",
    interface_hint_second: "  Ce spune {name} și în ce limbă sunt memoriile se aleg separat.",
    voice_language_question: "În ce limbă ar trebui să vorbească {name}?",
    voice_language_same: "ca Leteo",
    voice_language_same_detail: "limba pe care o vorbește Leteo însuși",
    voice_language_hint: "  {name} vorbește în conversația agentului tău, nu doar aici.",
    memory_language_question: "În ce limbă ar trebui scrise memoriile?",
    language_auto: "auto",
    language_auto_detail: "limba în care scrii, oricare ar fi ea",
    language_pinned_detail: "mereu, indiferent cum ți se scrie",
    language_kept_warning: "  Memoriile deja salvate își păstrează limba în care au fost scrise.",
    language_split_warning_first: "  Schimbarea lasă arhiva în două limbi, iar o căutare \
                                   găsește",
    language_split_warning_second: "  jumătatea în care este întrebată.",
    language_other_hint: "  Orice altă limbă: setează \"language\" în settings.json.",
    nothing_changed: "  Nimic nu a fost schimbat.",
    legend: "  spațiu alegere   enter continuare   backspace înapoi   esc ieșire",

    options_question: "Ce vrei să schimbi?",
    option_interface: "Limba Leteo",
    option_voice_language: "Limba lui {name}",
    option_memory_language: "Limba memoriilor",
    option_voice: "Vocea lui {name}",
    preferences_saved: "Preferințe salvate",

    could_not_adopt: "  nu s-a putut prelua: {error}",
    could_not_save: "  nu s-au putut salva preferințele: {error}",
    could_not_configure: "  nu s-a putut configura {agent}: {error}",
    could_not_remove: "  nu s-a putut elimina din {agent}: {error}",
    removed_from: "  eliminat din {agent}",
    restart_them: "\n  repornește-i pentru a prelua schimbarea",

    empty_dashboard_what_happens: "Memoriile apar aici pe măsură ce agenții tăi le salvează.",
    empty_dashboard_keys: "Apasă Esc pentru meniu sau ? pentru ajutor.",
    setup_cancelled: "Configurare anulată. Nimic nu a fost schimbat.",
    setup_failed: "configurare eșuată: {error}",

    panel_setup: " Configurare ",
    panel_dashboard: " Panou ",
    panel_detail: " Detaliu ",
    panel_content: " Conținut ",
    panel_session: " Sesiune ",
    panel_timeline: " Cronologie ",
    panel_context: " Context ",
    panel_session_timeline: " Cronologia sesiunii ",
    panel_help: " Ajutor ",
    panel_options: " Opțiuni ",
    panel_cloud: " Replicare în cloud - doar citire ",
    panel_filters: " FILTRE ",
    panel_filters_count: " FILTRE ({count}) ",
    panel_recorded: " Înregistrat ({count}) ",
    list_observations: " Observații",
    list_sessions: " Sesiuni",
    list_prompts: " Prompturi",
    scope_one_project: " în {project} ",
    scope_many_projects: " în {count} proiecte ",
    list_matching: " care corespund cu \"{query}\"",
    list_position: " {position} din {total} ",
    search_placeholder: "caută memorii",

    stat_observations: "OBSERVAȚII",
    stat_sessions: "SESIUNI",
    stat_prompts: "PROMPTURI",
    page_home: "ACASĂ",
    page_dashboard: "PANOU",
    page_detail: "DETALIU",
    page_session: "SESIUNE",
    page_timeline: "CRONOLOGIE",
    page_setup: "CONFIGURARE",
    page_cloud: "CLOUD",
    page_help: "AJUTOR",
    page_options: "OPȚIUNI",

    no_observations: "Nicio observație găsită",
    no_sessions: "Nicio sesiune găsită",
    no_prompts: "Niciun prompt găsit",
    no_projects: "Încă niciun proiect",
    no_observation_selected: "Nicio observație selectată",
    no_session_selected: "Nicio sesiune selectată",
    no_timeline_loaded: "Nicio cronologie încărcată",
    no_summary: "Niciun rezumat",
    nothing_to_search: "Încă nimic salvat — nu e nimic de căutat",
    cancelled: "Anulat",

    field_type: "Tip",
    field_project: "Proiect",
    field_scope: "Domeniu",
    field_session: "Sesiune",
    field_topic: "Subiect",
    field_started: "Început",
    field_ended: "Sfârșit",
    field_summary: "Rezumat",
    session_active: "activă",
    timeline_session: "Sesiune: {session}",
    timeline_focus: "Focalizare: #{id} {title} | {total} observații în total",
    timeline_focus_marker: "FOCALIZARE",

    cloud_server: "Server:      ",
    cloud_background: "Fundal:      ",
    cloud_replicating: "Replicare:   ",
    cloud_enrolled: "Înrolat:     ",
    cloud_queued: "În coadă:    ",
    cloud_deferred: "Amânate:     ",
    cloud_not_configured: "neconfigurat",
    cloud_enabled: "activat",
    cloud_disabled: "dezactivat",
    cloud_none: "niciunul",
    cloud_unknown: "necunoscut",
    cloud_mutations: "{count} mutații",
    cloud_deferred_dead: "{deferred} amânate, {dead} moarte",
    cloud_unreadable: "Arhiva nu a putut fi citită: {reason}",
    cloud_configure_hint: "Configurează cu: leteo cloud config set --server URL --token TOKEN \
                           --enable",
    cloud_state: "Stare:       ",
    cloud_failures: "{count} eșecuri la rând",
    cloud_backoff: "se așteaptă până la {until}",

    menu_start_setup: "Pornește configurarea",
    menu_dashboard: "Panou",
    menu_cloud: "Replicare în cloud",
    menu_options: "Opțiuni",
    menu_help: "Ajutor",
    menu_quit: "Ieșire",
    menu_uninstall: "Dezinstalează Leteo",
    uninstall_heading: "Elimin Leteo de pe această mașină?",
    uninstall_agents: "{count} agenți în care este configurat",
    uninstall_warning: "Totul de mai sus dispare. Nu se poate anula.",

    delete_memory: "Șterg memoria #{id}?",
    delete_prompt: "Șterg promptul #{id}?",
    delete_session: "Șterg sesiunea {id}?",
    delete_project: "Șterg proiectul {name}?",
    delete_permanent_warning: "Nu se poate anula.",
    delete_prompts_warning: "Memoriile se pot recupera. Prompturile nu.",
    delete_recoverable: "Se poate recupera din arhivă.",
    gone_permanently: "ștearsă definitiv",
    gone: "ștearsă",
    count_memories: "{count} memorii",
    count_sessions: "{count} sesiuni",
    count_prompts: "{count} prompturi",
    copied_to_clipboard: "S-au copiat {count} caractere în clipboard",
    data_refreshed: "Date reîmprospătate",
    deleted_memory: "Memoria #{id} {gone}",
    deleted_prompt: "Promptul #{id} șters",
    deleted_session: "Sesiunea {id} {gone}, cu {memories} memorii și {prompts} prompturi",
    deleted_project: "Proiectul {name} {gone}, cu {memories} memorii, \
                      {sessions} sesiuni și {prompts} prompturi",
    sessions_kept: " — {count} sesiuni păstrate, conțin rânduri ale altor proiecte",
    refreshed_query: "\"{query}\": {observations} observații, {sessions} sesiuni, \
                      {prompts} prompturi",

    keys_confirm: "y confirmare  n/Esc anulare",
    keys_confirm_footer: "y ștergere    orice altă tastă anulează",
    keys_confirm_window: "y  ștergere          orice altă tastă  anulare",
    keys_home: "j/k navigare  Enter alegere  / căutare  ? ajutor  q ieșire",
    keys_query: "tastează pentru căutare  Ctrl-U șterge  Enter/Esc înapoi la listă",
    keys_filters: "j/k proiect  spațiu bifează  f/Esc gata  q ieșire",
    keys_dashboard_searching: "j/k alegere  Enter deschide  Tab următoarea  f filtru  \
                               / editează  Esc curăță",
    keys_dashboard_sessions: "j/k sesiune  Enter deschide  Tab următoarea  f filtru  \
                              / căutare  Esc înapoi",
    keys_dashboard_prompts: "j/k prompt  Enter citește  Tab următoarea  f filtru  \
                             / căutare  Esc înapoi",
    keys_dashboard: "j/k alegere  Enter detaliu  Tab alta  f filtru  / căutare  \
                     y copiere  d ștergere",
    keys_detail: "j/k derulare  Enter/t cronologie  y copiere  d ștergere  Esc înapoi",
    keys_session: "j/k alegere  PgDn/End avansare  Enter detaliu  y copiere  d ștergere",
    keys_timeline: "j/k alegere  Enter detaliu  Esc înapoi  / căutare  q ieșire",
    keys_setup: "j/k alegere  spațiu bifează  Enter continuare  Backspace înapoi  Esc ieșire",
    keys_options: "j/k mișcare  Enter alegere  Esc înapoi",
    keys_cloud: "R reîmprospătare  Esc înapoi  q ieșire",
    keys_help: "Enter/Esc înapoi  q ieșire",

    help_body: "\
Navigare
  j / Jos        mută alegerea sau derulează în jos
  k / Sus        mută alegerea sau derulează în sus
  PgDn / PgUp    câte un ecran odată
  End / Home     sfârșitul listei și începutul ei
  Tab            panou: arată lista următoare —
                 observații, sesiuni, prompturi
  Enter          deschide alegerea / cronologia
  Esc            anulează sau mergi înapoi

Îngustarea listelor
  f              filtrează după proiect, unde spațiul
                 bifează și debifează. Fără bife,
                 toate proiectele.
  /              căutare. Rulează pe măsură ce tastezi
                 și potrivește cuvântul tastat după
                 început. Ambele îngustări se aplică
                 odată, tuturor celor trei liste.
  Esc            ieși din filtre, apoi renunță la căutare,
                 apoi părăsește pagina — în ordinea aceasta.

Vederi
  g / r          panou, pe observații
  s              panou, pe sesiuni
  t              cronologie din detaliu
  S              configurarea agenților
  c              replicare în cloud
  ? / h          acest ajutor

Acțiuni
  y              copiază memoria aleasă în clipboard
  d              șterge ce e sub cursor — o memorie, un
                 prompt, o sesiune cu tot ce a înregistrat
                 sau un proiect cu tot ce conține. Întreabă
                 mai întâi.
  D              la fel, definitiv. Întreabă mai întâi.
                 Memoriile revin; prompturile nu.

General
  R              reîmprospătează datele panoului
  Ctrl-U         curăță căutarea
  q              ieșire (în afara căutării)",
};
