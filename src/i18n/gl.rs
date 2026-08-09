//! Galician.

use super::Screens;

pub const SCREENS: Screens = Screens {
    found_engram: "Atopouse unha instalación de Engram",
    engram_counts: "{observations} memorias, {sessions} sesións, {prompts} prompts, \
                    {relations} relacións",
    adopt_question: "Adoptar estas memorias en Leteo?",
    adopt_yes: "Si, tráeas",
    adopt_no: "Non, comezar de cero",
    choose_agents: "Que axentes debe configurar Leteo?",
    will_be_removed: "quitarase",
    will_be_installed: "instalarase",
    hooks_question: "Instalar os hooks de ciclo de vida que fan a memoria automática \
                     en {agents}?",
    yes: "Si",
    hooks_no: "Non, só as ferramentas MCP",
    voice_question: "Canto debe dicir {name} en voz alta?",
    voice_all: "saúdo, pistas, capturas e lembranzas",
    voice_reminders: "só a lembranza de gardar",
    voice_quiet: "nada, nin sequera a lembranza de gardar",
    interface_question: "En que lingua debe falarche Leteo?",
    interface_hint_first: "  As pantallas de Leteo: os paneis, os menús, a axuda e esta páxina.",
    interface_hint_second: "  O que di {name} e en que se escriben as memorias escóllense á parte.",
    voice_language_question: "En que idioma debe falar {name}?",
    voice_language_same: "igual que Leteo",
    voice_language_same_detail: "o idioma no que fale Leteo",
    voice_language_hint: "  {name} fala dentro da conversa do teu axente, non só aquí.",
    memory_language_question: "En que lingua se deben escribir as memorias?",
    language_auto: "auto",
    language_auto_detail: "a lingua na que escribes, sexa cal sexa",
    language_pinned_detail: "sempre, en calquera lingua na que che escriban",
    language_kept_warning: "  As memorias xa gardadas manteñen a lingua na que se escribiron.",
    language_split_warning_first: "  Cambiar isto deixa o almacén en dúas linguas, e unha \
                                   busca atopa",
    language_split_warning_second: "  a metade na que se fai.",
    language_other_hint: "  Calquera outra lingua: pon \"language\" en settings.json.",
    nothing_changed: "  Non se cambiou nada.",
    legend: "  espazo escoller   intro continuar   retroceso atrás   esc saír",

    options_question: "Que queres cambiar?",
    option_interface: "Idioma de Leteo",
    option_voice_language: "Idioma de {name}",
    option_memory_language: "Idioma das memorias",
    option_voice: "Voz de {name}",
    preferences_saved: "Preferencias gardadas",

    could_not_adopt: "  non se puido adoptar: {error}",
    could_not_save: "  non se puideron gardar as preferencias: {error}",
    could_not_configure: "  non se puido configurar {agent}: {error}",
    could_not_remove: "  non se puido quitar de {agent}: {error}",
    removed_from: "  quitado de {agent}",
    restart_them: "\n  reiníciaos para que o collan",

    empty_dashboard_what_happens: "As memorias aparecen aquí a medida que os axentes as gardan.",
    empty_dashboard_keys: "Preme Esc para o menú, ou ? para a axuda.",
    setup_cancelled: "Configuración cancelada. Non se cambiou nada.",
    setup_failed: "a configuración fallou: {error}",

    panel_setup: " Configuración ",
    panel_dashboard: " Panel ",
    panel_detail: " Detalle ",
    panel_content: " Contido ",
    panel_session: " Sesión ",
    panel_timeline: " Cronoloxía ",
    panel_context: " Contexto ",
    panel_session_timeline: " Cronoloxía da sesión ",
    panel_help: " Axuda ",
    panel_options: " Opcións ",
    panel_cloud: " Replicación na nube - só lectura ",
    panel_filters: " FILTROS ",
    panel_filters_count: " FILTROS ({count}) ",
    panel_recorded: " Rexistrado ({count}) ",
    list_observations: " Observacións",
    list_sessions: " Sesións",
    list_prompts: " Prompts",
    scope_one_project: " en {project} ",
    scope_many_projects: " en {count} proxectos ",
    list_matching: " que casan con \"{query}\"",
    list_position: " {position} de {total} ",
    search_placeholder: "buscar memorias",

    stat_observations: "OBSERVACIÓNS",
    stat_sessions: "SESIÓNS",
    stat_prompts: "PROMPTS",
    page_home: "INICIO",
    page_dashboard: "PANEL",
    page_detail: "DETALLE",
    page_session: "SESIÓN",
    page_timeline: "CRONOLOXÍA",
    page_setup: "CONFIGURACIÓN",
    page_cloud: "NUBE",
    page_help: "AXUDA",
    page_options: "OPCIÓNS",

    no_observations: "Non se atoparon observacións",
    no_sessions: "Non se atoparon sesións",
    no_prompts: "Non se atoparon prompts",
    no_projects: "Aínda non hai proxectos",
    no_observation_selected: "Ningunha observación escollida",
    no_session_selected: "Ningunha sesión escollida",
    no_timeline_loaded: "Ningunha cronoloxía cargada",
    no_summary: "Sen resumo",
    nothing_to_search: "Aínda non hai nada gardado — non hai nada que buscar",
    cancelled: "Cancelado",

    field_type: "Tipo",
    field_project: "Proxecto",
    field_scope: "Ámbito",
    field_session: "Sesión",
    field_topic: "Tema",
    field_started: "Comezo",
    field_ended: "Fin",
    field_summary: "Resumo",
    session_active: "activa",
    timeline_session: "Sesión: {session}",
    timeline_focus: "Foco: #{id} {title} | {total} observación(s) en total",
    timeline_focus_marker: "FOCO",

    cloud_server: "Servidor:    ",
    cloud_background: "En segundo plano: ",
    cloud_replicating: "Replicando:  ",
    cloud_enrolled: "Inscritos:   ",
    cloud_queued: "Na cola:     ",
    cloud_deferred: "Adiadas:     ",
    cloud_not_configured: "sen configurar",
    cloud_enabled: "activado",
    cloud_disabled: "desactivado",
    cloud_none: "ningún",
    cloud_unknown: "descoñecido",
    cloud_mutations: "{count} mutación(s)",
    cloud_deferred_dead: "{deferred} adiadas, {dead} mortas",
    cloud_unreadable: "Non se puido ler o almacén: {reason}",
    cloud_configure_hint: "Configura con: leteo cloud config set --server URL --token TOKEN \
                           --enable",
    cloud_state: "Estado:      ",
    cloud_failures: "{count} fallo(s) seguidos",
    cloud_backoff: "agardando ata {until}",

    menu_start_setup: "Comezar a configuración",
    menu_dashboard: "Panel",
    menu_cloud: "Replicación na nube",
    menu_options: "Opcións",
    menu_help: "Axuda",
    menu_quit: "Saír",
    menu_uninstall: "Desinstalar Leteo",
    uninstall_heading: "Quitar Leteo desta máquina?",
    uninstall_agents: "{count} axente(s) nos que está configurado",
    uninstall_warning: "Todo o de arriba vaise. Non se pode desfacer.",

    delete_memory: "Borrar a memoria #{id}?",
    delete_prompt: "Borrar o prompt #{id}?",
    delete_session: "Borrar a sesión {id}?",
    delete_project: "Borrar o proxecto {name}?",
    delete_permanent_warning: "Non se pode desfacer.",
    delete_prompts_warning: "As memorias recupéranse. Os prompts non.",
    delete_recoverable: "Isto pódese recuperar do almacén.",
    gone_permanently: "borrada para sempre",
    gone: "borrada",
    count_memories: "{count} memoria(s)",
    count_sessions: "{count} sesión(s)",
    count_prompts: "{count} prompt(s)",
    copied_to_clipboard: "Copiáronse {count} caracteres ao portapapeis",
    data_refreshed: "Datos recargados",
    deleted_memory: "Memoria #{id} {gone}",
    deleted_prompt: "Prompt #{id} borrado",
    deleted_session: "Sesión {id} {gone}, con {memories} memoria(s) e {prompts} prompt(s)",
    deleted_project: "Proxecto {name} {gone}, con {memories} memoria(s), \
                      {sessions} sesión(s) e {prompts} prompt(s)",
    sessions_kept: " — {count} sesión(s) mantidas, teñen filas doutros proxectos",
    refreshed_query: "\"{query}\": {observations} observación(s), {sessions} sesión(s), \
                      {prompts} prompt(s)",

    keys_confirm: "y confirmar  n/Esc cancelar",
    keys_confirm_footer: "y borrar    calquera outra tecla cancela",
    keys_confirm_window: "y  borrar            calquera outra tecla  cancelar",
    keys_home: "j/k moverse  Intro escoller  / buscar  ? axuda  q saír",
    keys_query: "escribe para buscar  Ctrl-U limpar  Intro/Esc volver á lista",
    keys_filters: "j/k proxecto  espazo marcar  f/Esc feito  q saír",
    keys_dashboard_searching: "j/k escoller  Intro abrir  Tab seguinte  f filtrar  / editar  \
                               Esc limpar",
    keys_dashboard_sessions: "j/k sesión  Intro abrir  Tab seguinte  f filtrar  / buscar  \
                              Esc atrás",
    keys_dashboard_prompts: "j/k prompt  Intro ler  Tab seguinte  f filtrar  / buscar  Esc atrás",
    keys_dashboard: "j/k mover  Intro detalle  Tab seguinte  f filtrar  / buscar  y copiar  \
                     d borrar",
    keys_detail: "j/k desprazar  Intro/t cronoloxía  y copiar  d borrar  Esc atrás  q saír",
    keys_session: "j/k escoller  PgDn/End avanzar  Intro detalle  y copiar  d borrar  Esc atrás",
    keys_timeline: "j/k escoller  Intro detalle  Esc atrás  / buscar  q saír",
    keys_setup: "j/k escoller  espazo marcar  Intro continuar  Retroceso atrás  Esc cancelar",
    keys_options: "j/k mover  Intro escoller  Esc atrás",
    keys_cloud: "R recargar  Esc atrás  q saír",
    keys_help: "Intro/Esc atrás  q saír",

    help_body: "\
Navegación
  j / Abaixo     mover a escolla ou baixar
  k / Arriba     mover a escolla ou subir
  PgDn / PgUp    unha pantalla de cada vez
  End / Home     o final da lista, e o comezo
  Tab            panel: amosar a lista seguinte —
                 observacións, sesións, prompts
  Intro          abrir a escolla / a cronoloxía
  Esc            cancelar ou volver atrás

Estreitar as listas
  f              filtrar por proxecto, onde o espazo marca e
                 desmarca. Sen marcas, todos os proxectos.
  /              buscar. Vai mentres escribes, e casa a palabra
                 que escribes polo seu comezo. Os dous filtros
                 aplícanse á vez, ás tres listas.
  Esc            saír dos filtros, logo deixar a busca, logo
                 saír da páxina — nesa orde.

Vistas
  g / r          panel, sobre as observacións
  s              panel, sobre as sesións
  t              cronoloxía desde o detalle
  S              configuración dos axentes
  c              replicación na nube
  ? / h          esta axuda

Accións
  y              copiar a memoria escollida ao portapapeis
  d              borrar o que hai baixo o cursor — unha memoria,
                 un prompt, unha sesión con todo o que rexistrou,
                 ou un proxecto enteiro. Pregunta antes.
  D              o mesmo, para sempre. Pregunta antes.
                 As memorias volven; os prompts non.

Xeral
  R              recargar os datos do panel
  Ctrl-U         limpar a busca
  q              saír (fóra da busca)",
};
