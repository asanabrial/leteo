//! Italian.

use super::Screens;

pub const SCREENS: Screens = Screens {
    found_engram: "È stata trovata un'installazione di Engram",
    engram_counts: "{observations} memorie, {sessions} sessioni, {prompts} prompt, \
                    {relations} relazioni",
    adopt_question: "Adottare queste memorie in Leteo?",
    adopt_yes: "Sì, portarle qui",
    adopt_no: "No, ricominciare da zero",
    choose_agents: "Quali agenti deve configurare Leteo?",
    will_be_removed: "verrà rimosso",
    will_be_installed: "verrà installato",
    hooks_question: "Installare gli hook di ciclo di vita che rendono la memoria automatica \
                     in {agents}?",
    yes: "Sì",
    hooks_no: "No, solo gli strumenti MCP",
    voice_question: "Quanto deve dire {name} ad alta voce?",
    voice_all: "saluto, suggerimenti, catture e promemoria",
    voice_reminders: "solo il promemoria per salvare",
    voice_quiet: "niente, nemmeno il promemoria per salvare",
    interface_question: "In quale lingua deve parlarti Leteo?",
    interface_hint_first: "  Le schermate di Leteo: i pannelli, i menu, la guida e questa pagina.",
    interface_hint_second: "  Cosa dice {name} e in che lingua sono i ricordi si scelgono a parte.",
    voice_language_question: "In che lingua deve parlare {name}?",
    voice_language_same: "come Leteo",
    voice_language_same_detail: "la lingua che parla Leteo stesso",
    voice_language_hint: "  {name} parla dentro la conversazione del tuo agente, non solo qui.",
    memory_language_question: "In quale lingua vanno scritte le memorie?",
    language_auto: "auto",
    language_auto_detail: "la lingua in cui scrivi, qualunque sia",
    language_pinned_detail: "sempre, in qualunque lingua ti si scriva",
    language_kept_warning: "  Le memorie già salvate mantengono la lingua in cui furono scritte.",
    language_split_warning_first: "  Cambiarlo lascia l'archivio in due lingue, e una ricerca \
                                   trova",
    language_split_warning_second: "  la metà in cui viene fatta.",
    language_other_hint: "  Qualunque altra lingua: imposta \"language\" in settings.json.",
    nothing_changed: "  Non è stato cambiato nulla.",
    legend: "  spazio scegliere   invio continuare   backspace indietro   esc uscire",

    options_question: "Cosa vuoi cambiare?",
    option_interface: "Lingua di Leteo",
    option_voice_language: "Lingua di {name}",
    option_memory_language: "Lingua dei ricordi",
    option_voice: "Voce di {name}",
    preferences_saved: "Preferenze salvate",

    could_not_adopt: "  adozione non riuscita: {error}",
    could_not_save: "  preferenze non salvate: {error}",
    could_not_configure: "  impossibile configurare {agent}: {error}",
    could_not_remove: "  impossibile rimuovere da {agent}: {error}",
    removed_from: "  rimosso da {agent}",
    restart_them: "\n  riavviali perché lo recepiscano",

    empty_dashboard_what_happens: "Le memorie appaiono qui man mano che i tuoi agenti le salvano.",
    empty_dashboard_keys: "Premi Esc per il menu, o ? per l'aiuto.",
    setup_cancelled: "Configurazione annullata. Non è stato cambiato nulla.",
    setup_failed: "configurazione fallita: {error}",

    panel_setup: " Configurazione ",
    panel_dashboard: " Pannello ",
    panel_detail: " Dettaglio ",
    panel_content: " Contenuto ",
    panel_session: " Sessione ",
    panel_timeline: " Cronologia ",
    panel_context: " Contesto ",
    panel_session_timeline: " Cronologia della sessione ",
    panel_help: " Aiuto ",
    panel_options: " Opzioni ",
    panel_cloud: " Replica nel cloud - sola lettura ",
    panel_filters: " FILTRI ",
    panel_filters_count: " FILTRI ({count}) ",
    panel_recorded: " Registrato ({count}) ",
    list_observations: " Osservazioni",
    list_sessions: " Sessioni",
    list_prompts: " Prompt",
    scope_one_project: " in {project} ",
    scope_many_projects: " in {count} progetti ",
    list_matching: " che corrispondono a \"{query}\"",
    list_position: " {position} di {total} ",
    search_placeholder: "cercare memorie",

    stat_observations: "OSSERVAZIONI",
    stat_sessions: "SESSIONI",
    stat_prompts: "PROMPT",
    page_home: "INIZIO",
    page_dashboard: "PANNELLO",
    page_detail: "DETTAGLIO",
    page_session: "SESSIONE",
    page_timeline: "CRONOLOGIA",
    page_setup: "CONFIGURAZIONE",
    page_cloud: "CLOUD",
    page_help: "AIUTO",
    page_options: "OPZIONI",

    no_observations: "Nessuna osservazione trovata",
    no_sessions: "Nessuna sessione trovata",
    no_prompts: "Nessun prompt trovato",
    no_projects: "Ancora nessun progetto",
    no_observation_selected: "Nessuna osservazione scelta",
    no_session_selected: "Nessuna sessione scelta",
    no_timeline_loaded: "Nessuna cronologia caricata",
    no_summary: "Nessun riassunto",
    nothing_to_search: "Non c'è ancora nulla di salvato — non c'è nulla da cercare",
    cancelled: "Annullato",

    field_type: "Tipo",
    field_project: "Progetto",
    field_scope: "Ambito",
    field_session: "Sessione",
    field_topic: "Argomento",
    field_started: "Inizio",
    field_ended: "Fine",
    field_summary: "Riassunto",
    session_active: "attiva",
    timeline_session: "Sessione: {session}",
    timeline_focus: "Fuoco: #{id} {title} | {total} osservazione/i in tutto",
    timeline_focus_marker: "FUOCO",

    cloud_server: "Server:      ",
    cloud_background: "In sottofondo: ",
    cloud_replicating: "Replica:     ",
    cloud_enrolled: "Iscritti:    ",
    cloud_queued: "In coda:     ",
    cloud_deferred: "Rinviate:    ",
    cloud_not_configured: "non configurato",
    cloud_enabled: "attivo",
    cloud_disabled: "disattivato",
    cloud_none: "nessuno",
    cloud_unknown: "sconosciuto",
    cloud_mutations: "{count} mutazione/i",
    cloud_deferred_dead: "{deferred} rinviate, {dead} morte",
    cloud_unreadable: "Non è stato possibile leggere l'archivio: {reason}",
    cloud_configure_hint: "Configura con: leteo cloud config set --server URL --token TOKEN \
                           --enable",
    cloud_state: "Stato:       ",
    cloud_failures: "{count} fallimento/i di fila",
    cloud_backoff: "in attesa fino a {until}",

    menu_start_setup: "Avviare la configurazione",
    menu_dashboard: "Pannello",
    menu_cloud: "Replica nel cloud",
    menu_options: "Opzioni",
    menu_help: "Aiuto",
    menu_quit: "Uscire",
    menu_uninstall: "Disinstallare Leteo",
    uninstall_heading: "Rimuovere Leteo da questa macchina?",
    uninstall_agents: "{count} agente/i in cui è configurato",
    uninstall_warning: "Tutto quanto sopra sparisce. Non si può annullare.",

    delete_memory: "Cancellare la memoria #{id}?",
    delete_prompt: "Cancellare il prompt #{id}?",
    delete_session: "Cancellare la sessione {id}?",
    delete_project: "Cancellare il progetto {name}?",
    delete_permanent_warning: "Non si può annullare.",
    delete_prompts_warning: "Le memorie si recuperano. I prompt no.",
    delete_recoverable: "Questo si può recuperare dall'archivio.",
    gone_permanently: "cancellata per sempre",
    gone: "cancellata",
    count_memories: "{count} memoria/e",
    count_sessions: "{count} sessione/i",
    count_prompts: "{count} prompt",
    copied_to_clipboard: "Copiati {count} caratteri negli appunti",
    data_refreshed: "Dati ricaricati",
    deleted_memory: "Memoria #{id} {gone}",
    deleted_prompt: "Prompt #{id} cancellato",
    deleted_session: "Sessione {id} {gone}, con {memories} memoria/e e {prompts} prompt",
    deleted_project: "Progetto {name} {gone}, con {memories} memoria/e, \
                      {sessions} sessione/i e {prompts} prompt",
    sessions_kept: " — {count} sessione/i tenute, contengono righe di altri progetti",
    refreshed_query: "\"{query}\": {observations} osservazione/i, {sessions} sessione/i, \
                      {prompts} prompt",

    keys_confirm: "y confermare  n/Esc annullare",
    keys_confirm_footer: "y cancellare    qualsiasi altro tasto annulla",
    keys_confirm_window: "y  cancellare        qualsiasi altro tasto  annullare",
    keys_home: "j/k muoversi  Invio scegliere  / cercare  ? aiuto  q uscire",
    keys_query: "scrivi per cercare  Ctrl-U pulire  Invio/Esc tornare alla lista",
    keys_filters: "j/k progetto  spazio segnare  f/Esc fatto  q uscire",
    keys_dashboard_searching: "j/k scegliere  Invio aprire  Tab prossima  f filtrare  \
                               / modificare  Esc pulire",
    keys_dashboard_sessions: "j/k sessione  Invio aprire  Tab prossima  f filtrare  / cercare  \
                              Esc indietro",
    keys_dashboard_prompts: "j/k prompt  Invio leggere  Tab prossima  f filtrare  / cercare  \
                             Esc indietro",
    keys_dashboard: "j/k muovi  Invio dettaglio  Tab altra  f filtro  / cerca  \
                     y copia  d togli",
    keys_detail: "j/k scorrere  Invio/t cronologia  y copiare  d cancellare  Esc indietro",
    keys_session: "j/k scegliere  PgDn/End avanzare  Invio dettaglio  y copiare  d cancellare",
    keys_timeline: "j/k scegliere  Invio dettaglio  Esc indietro  / cercare  q uscire",
    keys_setup: "j/k scegliere  spazio segnare  Invio continuare  Backspace indietro  Esc uscire",
    keys_options: "j/k spostare  Invio scegliere  Esc indietro",
    keys_cloud: "R ricaricare  Esc indietro  q uscire",
    keys_help: "Invio/Esc indietro  q uscire",

    help_body: "\
Navigazione
  j / Giù        muovere la scelta o scendere
  k / Su         muovere la scelta o salire
  PgDn / PgUp    una schermata alla volta
  End / Home     la fine della lista, e l'inizio
  Tab            pannello: mostrare la lista dopo —
                 osservazioni, sessioni, prompt
  Invio          aprire la scelta / la cronologia
  Esc            annullare o tornare indietro

Restringere le liste
  f              filtrare per progetto, dove lo spazio segna
                 e toglie. Senza segni, ogni progetto.
  /              cercare. Corre mentre scrivi, e combacia con
                 la parola in corso dal suo inizio. I due
                 filtri si applicano insieme, a tutte e tre.
  Esc            uscire dai filtri, poi lasciare la ricerca,
                 poi uscire dalla pagina — in quest'ordine.

Viste
  g / r          pannello, sulle osservazioni
  s              pannello, sulle sessioni
  t              cronologia dal dettaglio
  S              configurazione degli agenti
  c              replica nel cloud
  ? / h          questo aiuto

Azioni
  y              copiare la memoria scelta negli appunti
  d              cancellare ciò che sta sotto il cursore — una
                 memoria, un prompt, una sessione con tutto
                 quanto ha registrato, o un progetto intero.
                 Chiede prima.
  D              lo stesso, per sempre. Chiede prima.
                 Le memorie tornano; i prompt no.

Generale
  R              ricaricare i dati del pannello
  Ctrl-U         pulire la ricerca
  q              uscire (fuori dalla ricerca)",
};
