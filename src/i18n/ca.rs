//! Catalan.

use super::Screens;

pub const SCREENS: Screens = Screens {
    found_engram: "S'ha trobat una instal·lació d'Engram",
    engram_counts: "{observations} memòries, {sessions} sessions, {prompts} prompts, \
                    {relations} relacions",
    adopt_question: "Voleu adoptar aquestes memòries a Leteo?",
    adopt_yes: "Sí, porta-les",
    adopt_no: "No, comença de zero",
    choose_agents: "Quins agents ha de configurar Leteo?",
    will_be_removed: "es traurà",
    will_be_installed: "s'instal·larà",
    hooks_question: "Voleu instal·lar els hooks de cicle de vida que fan la memòria automàtica \
                     a {agents}?",
    yes: "Sí",
    hooks_no: "No, només les eines MCP",
    voice_question: "Quant ha de dir {name} en veu alta?",
    voice_all: "salutació, pistes, captures i recordatoris",
    voice_reminders: "només el recordatori de desar",
    voice_quiet: "res, ni tan sols el recordatori de desar",
    interface_question: "En quina llengua us ha de parlar Leteo?",
    interface_hint_first: "  Les pantalles de Leteo: els plafons, els menús, l'ajuda i aquesta pàgina.",
    interface_hint_second: "  Què diu {name} i en què s'escriuen les memòries es trien a part.",
    voice_language_question: "En quin idioma ha de parlar {name}?",
    voice_language_same: "igual que Leteo",
    voice_language_same_detail: "l'idioma en què parli Leteo",
    voice_language_hint: "  {name} parla dins la conversa del teu agent, no només aquí.",
    memory_language_question: "En quina llengua s'han d'escriure les memòries?",
    language_auto: "auto",
    language_auto_detail: "la llengua en què escriviu, sigui quina sigui",
    language_pinned_detail: "sempre, en qualsevol llengua que us escriguin",
    language_kept_warning: "  Les memòries ja desades mantenen la llengua en què es van escriure.",
    language_split_warning_first: "  Canviar-ho deixa el magatzem en dues llengües, i una \
                                   cerca troba",
    language_split_warning_second: "  la meitat en què es fa.",
    language_other_hint: "  Qualsevol altra llengua: poseu \"language\" a settings.json.",
    nothing_changed: "  No s'ha canviat res.",
    legend: "  espai triar   retorn continuar   retrocés enrere   esc sortir",

    options_question: "Què vols canviar?",
    option_interface: "Idioma de Leteo",
    option_voice_language: "Idioma d'en {name}",
    option_memory_language: "Idioma de les memòries",
    option_voice: "Veu de {name}",
    preferences_saved: "Preferències desades",

    could_not_adopt: "  no s'ha pogut adoptar: {error}",
    could_not_save: "  no s'han pogut desar les preferències: {error}",
    could_not_configure: "  no s'ha pogut configurar {agent}: {error}",
    could_not_remove: "  no s'ha pogut treure de {agent}: {error}",
    removed_from: "  tret de {agent}",
    restart_them: "\n  reinicieu-los perquè ho agafin",

    empty_dashboard_what_happens: "Les memòries apareixen aquí a mesura que els agents les desen.",
    empty_dashboard_keys: "Premeu Esc per al menú, o ? per a l'ajuda.",
    setup_cancelled: "Configuració cancel·lada. No s'ha canviat res.",
    setup_failed: "la configuració ha fallat: {error}",

    panel_setup: " Configuració ",
    panel_dashboard: " Tauler ",
    panel_detail: " Detall ",
    panel_content: " Contingut ",
    panel_session: " Sessió ",
    panel_timeline: " Cronologia ",
    panel_context: " Context ",
    panel_session_timeline: " Cronologia de la sessió ",
    panel_help: " Ajuda ",
    panel_options: " Opcions ",
    panel_cloud: " Replicació al núvol - només lectura ",
    panel_filters: " FILTRES ",
    panel_filters_count: " FILTRES ({count}) ",
    panel_recorded: " Registrat ({count}) ",
    list_observations: " Observacions",
    list_sessions: " Sessions",
    list_prompts: " Prompts",
    scope_one_project: " a {project} ",
    scope_many_projects: " a {count} projectes ",
    list_matching: " que casen amb \"{query}\"",
    list_position: " {position} de {total} ",
    search_placeholder: "cercar memòries",

    stat_observations: "OBSERVACIONS",
    stat_sessions: "SESSIONS",
    stat_prompts: "PROMPTS",
    page_home: "INICI",
    page_dashboard: "TAULER",
    page_detail: "DETALL",
    page_session: "SESSIÓ",
    page_timeline: "CRONOLOGIA",
    page_setup: "CONFIGURACIÓ",
    page_cloud: "NÚVOL",
    page_help: "AJUDA",
    page_options: "OPCIONS",

    no_observations: "No s'ha trobat cap observació",
    no_sessions: "No s'ha trobat cap sessió",
    no_prompts: "No s'ha trobat cap prompt",
    no_projects: "Encara no hi ha projectes",
    no_observation_selected: "Cap observació triada",
    no_session_selected: "Cap sessió triada",
    no_timeline_loaded: "Cap cronologia carregada",
    no_summary: "Sense resum",
    nothing_to_search: "Encara no hi ha res desat — no hi ha res a cercar",
    cancelled: "Cancel·lat",

    field_type: "Tipus",
    field_project: "Projecte",
    field_scope: "Abast",
    field_session: "Sessió",
    field_topic: "Tema",
    field_started: "Inici",
    field_ended: "Fi",
    field_summary: "Resum",
    session_active: "activa",
    timeline_session: "Sessió: {session}",
    timeline_focus: "Focus: #{id} {title} | {total} observació/ons en total",
    timeline_focus_marker: "FOCUS",

    cloud_server: "Servidor:    ",
    cloud_background: "En segon pla: ",
    cloud_replicating: "Replicant:   ",
    cloud_enrolled: "Inscrits:    ",
    cloud_queued: "A la cua:    ",
    cloud_deferred: "Ajornades:   ",
    cloud_not_configured: "no configurat",
    cloud_enabled: "activat",
    cloud_disabled: "desactivat",
    cloud_none: "cap",
    cloud_unknown: "desconegut",
    cloud_mutations: "{count} mutació/ons",
    cloud_deferred_dead: "{deferred} ajornades, {dead} mortes",
    cloud_unreadable: "No s'ha pogut llegir el magatzem: {reason}",
    cloud_configure_hint: "Configureu amb: leteo cloud config set --server URL --token TOKEN \
                           --enable",
    cloud_state: "Estat:       ",
    cloud_failures: "{count} fallada/es seguides",
    cloud_backoff: "esperant fins a {until}",

    menu_start_setup: "Comença la configuració",
    menu_dashboard: "Tauler",
    menu_cloud: "Replicació al núvol",
    menu_options: "Opcions",
    menu_help: "Ajuda",
    menu_quit: "Surt",
    menu_uninstall: "Desinstal·la Leteo",
    uninstall_heading: "Voleu treure Leteo d'aquesta màquina?",
    uninstall_agents: "{count} agent(s) on està configurat",
    uninstall_warning: "Tot el que hi ha a dalt se'n va. No es pot desfer.",

    delete_memory: "Voleu esborrar la memòria #{id}?",
    delete_prompt: "Voleu esborrar el prompt #{id}?",
    delete_session: "Voleu esborrar la sessió {id}?",
    delete_project: "Voleu esborrar el projecte {name}?",
    delete_permanent_warning: "No es pot desfer.",
    delete_prompts_warning: "Les memòries es recuperen. Els prompts no.",
    delete_recoverable: "Això es pot recuperar del magatzem.",
    gone_permanently: "esborrada per sempre",
    gone: "esborrada",
    count_memories: "{count} memòria/es",
    count_sessions: "{count} sessió/ons",
    count_prompts: "{count} prompt(s)",
    copied_to_clipboard: "S'han copiat {count} caràcters al porta-retalls",
    data_refreshed: "Dades recarregades",
    deleted_memory: "Memòria #{id} {gone}",
    deleted_prompt: "Prompt #{id} esborrat",
    deleted_session: "Sessió {id} {gone}, amb {memories} memòria/es i {prompts} prompt(s)",
    deleted_project: "Projecte {name} {gone}, amb {memories} memòria/es, \
                      {sessions} sessió/ons i {prompts} prompt(s)",
    sessions_kept: " — {count} sessió/ons mantingudes, tenen files d'altres projectes",
    refreshed_query: "\"{query}\": {observations} observació/ons, {sessions} sessió/ons, \
                      {prompts} prompt(s)",

    keys_confirm: "y confirmar  n/Esc cancel·lar",
    keys_confirm_footer: "y esborrar    qualsevol altra tecla cancel·la",
    keys_confirm_window: "y  esborrar          qualsevol altra tecla  cancel·lar",
    keys_home: "j/k moure  Retorn triar  / cercar  ? ajuda  q sortir",
    keys_query: "escriviu per cercar  Ctrl-U netejar  Retorn/Esc tornar a la llista",
    keys_filters: "j/k projecte  espai marcar  f/Esc fet  q sortir",
    keys_dashboard_searching: "j/k triar  Retorn obrir  Tab següent  f filtrar  / editar  \
                               Esc netejar",
    keys_dashboard_sessions: "j/k sessió  Retorn obrir  Tab següent  f filtrar  / cercar  \
                              Esc enrere",
    keys_dashboard_prompts: "j/k prompt  Retorn llegir  Tab següent  f filtrar  / cercar  \
                             Esc enrere",
    keys_dashboard: "j/k triar  Retorn detall  Tab següent  f filtrar  / cercar  y copiar  \
                     d esborrar",
    keys_detail: "j/k desplaçar  Retorn/t cronologia  y copiar  d esborrar  Esc enrere",
    keys_session: "j/k triar  PgDn/End avançar  Retorn detall  y copiar  d esborrar  Esc enrere",
    keys_timeline: "j/k triar  Retorn detall  Esc enrere  / cercar  q sortir",
    keys_setup: "j/k triar  espai marcar  Retorn continuar  Retrocés enrere  Esc cancel·lar",
    keys_options: "j/k moure  Retorn triar  Esc enrere",
    keys_cloud: "R recarregar  Esc enrere  q sortir",
    keys_help: "Retorn/Esc enrere  q sortir",

    help_body: "\
Navegació
  j / Avall      moure la tria o baixar
  k / Amunt      moure la tria o pujar
  PgDn / PgUp    una pantalla cada cop
  End / Home     el final de la llista, i el començament
  Tab            tauler: mostrar la llista següent —
                 observacions, sessions, prompts
  Retorn         obrir la tria / la cronologia
  Esc            cancel·lar o tornar enrere

Estrènyer les llistes
  f              filtrar per projecte, on l'espai marca i
                 desmarca. Sense marques, tots els projectes.
  /              cercar. Va mentre escriviu, i casa la paraula
                 que escriviu pel seu començament. Els dos
                 filtres s'apliquen alhora, a totes tres.
  Esc            sortir dels filtres, després deixar la cerca,
                 després sortir de la pàgina — en aquest ordre.

Vistes
  g / r          tauler, sobre les observacions
  s              tauler, sobre les sessions
  t              cronologia des del detall
  S              configuració dels agents
  c              replicació al núvol
  ? / h          aquesta ajuda

Accions
  y              copiar la memòria triada al porta-retalls
  d              esborrar el que hi ha sota el cursor — una
                 memòria, un prompt, una sessió amb tot el que
                 va registrar, o un projecte sencer. Pregunta.
  D              el mateix, per sempre. Pregunta abans.
                 Les memòries tornen; els prompts no.

General
  R              recarregar les dades del tauler
  Ctrl-U         netejar la cerca
  q              sortir (fora de la cerca)",
};
