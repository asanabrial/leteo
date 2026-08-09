//! Spanish.

use super::Screens;

pub const SCREENS: Screens = Screens {
    found_engram: "Se encontró una instalación de Engram",
    engram_counts: "{observations} memorias, {sessions} sesiones, {prompts} prompts, \
                    {relations} relaciones",
    adopt_question: "¿Adoptar estas memorias en Leteo?",
    adopt_yes: "Sí, tráelas",
    adopt_no: "No, empezar de cero",
    choose_agents: "¿Qué agentes debe configurar Leteo?",
    will_be_removed: "se quitará",
    will_be_installed: "se instalará",
    hooks_question: "¿Instalar los hooks de ciclo de vida que hacen la memoria automática \
                     en {agents}?",
    yes: "Sí",
    hooks_no: "No, solo las herramientas MCP",
    voice_question: "¿Cuánto debe decir {name} en voz alta?",
    voice_all: "saludo, pistas, capturas y recordatorios",
    voice_reminders: "solo el recordatorio para guardar",
    voice_quiet: "nada, ni siquiera el recordatorio para guardar",
    interface_question: "¿En qué idioma debe hablarte Leteo?",
    interface_hint_first: "  Las pantallas de Leteo: los paneles, los menús, la ayuda y esta página.",
    interface_hint_second: "  Lo que dice {name} y en qué se escriben las memorias se eligen aparte.",
    voice_language_question: "¿En qué idioma debe hablar {name}?",
    voice_language_same: "igual que Leteo",
    voice_language_same_detail: "el idioma en el que hable Leteo",
    voice_language_hint: "  {name} habla dentro de la conversación de tu agente, no solo aquí.",
    memory_language_question: "¿En qué idioma se deben escribir las memorias?",
    language_auto: "auto",
    language_auto_detail: "el idioma en el que escribas, sea cual sea",
    language_pinned_detail: "siempre, te escriban como te escriban",
    language_kept_warning: "  Las memorias ya guardadas conservan el idioma en que se escribieron.",
    language_split_warning_first: "  Cambiarlo deja el almacén en dos idiomas, y una búsqueda \
                                   encuentra",
    language_split_warning_second: "  la mitad en la que se pregunta.",
    language_other_hint: "  Cualquier otro idioma: pon \"language\" en settings.json.",
    nothing_changed: "  No se cambió nada.",
    legend: "  espacio elegir   intro continuar   retroceso atrás   esc salir",

    options_question: "¿Qué quieres cambiar?",
    option_interface: "Idioma de Leteo",
    option_voice_language: "Idioma de {name}",
    option_memory_language: "Idioma de las memorias",
    option_voice: "Voz de {name}",
    preferences_saved: "Preferencias guardadas",

    could_not_adopt: "  no se pudo adoptar: {error}",
    could_not_save: "  no se pudieron guardar las preferencias: {error}",
    could_not_configure: "  no se pudo configurar {agent}: {error}",
    could_not_remove: "  no se pudo quitar de {agent}: {error}",
    removed_from: "  se quitó de {agent}",
    restart_them: "\n  reinícialos para que lo cojan",

    empty_dashboard_what_happens: "Las memorias aparecen aquí a medida que tus agentes las guardan.",
    empty_dashboard_keys: "Pulsa Esc para el menú, o ? para la ayuda.",
    setup_cancelled: "Configuración cancelada. No se cambió nada.",
    setup_failed: "la configuración falló: {error}",

    panel_setup: " Configuración ",
    panel_dashboard: " Panel ",
    panel_detail: " Detalle ",
    panel_content: " Contenido ",
    panel_session: " Sesión ",
    panel_timeline: " Cronología ",
    panel_context: " Contexto ",
    panel_session_timeline: " Cronología de la sesión ",
    panel_help: " Ayuda ",
    panel_options: " Opciones ",
    panel_cloud: " Replicación en la nube - solo lectura ",
    panel_filters: " FILTROS ",
    panel_filters_count: " FILTROS ({count}) ",
    panel_recorded: " Registrado ({count}) ",
    list_observations: " Memorias",
    list_sessions: " Sesiones",
    list_prompts: " Prompts",
    scope_one_project: " en {project} ",
    scope_many_projects: " en {count} proyectos ",
    list_matching: " que coinciden con \"{query}\"",
    list_position: " {position} de {total} ",
    search_placeholder: "buscar memorias",

    stat_observations: "MEMORIAS",
    stat_sessions: "SESIONES",
    stat_prompts: "PROMPTS",
    page_home: "INICIO",
    page_dashboard: "PANEL",
    page_detail: "DETALLE",
    page_session: "SESIÓN",
    page_timeline: "CRONOLOGÍA",
    page_setup: "CONFIGURACIÓN",
    page_cloud: "NUBE",
    page_help: "AYUDA",
    page_options: "OPCIONES",

    no_observations: "No se encontraron memorias",
    no_sessions: "No se encontraron sesiones",
    no_prompts: "No se encontraron prompts",
    no_projects: "Todavía no hay proyectos",
    no_observation_selected: "Ninguna memoria seleccionada",
    no_session_selected: "Ninguna sesión seleccionada",
    no_timeline_loaded: "No hay cronología cargada",
    no_summary: "Sin resumen",
    nothing_to_search: "Todavía no hay nada guardado — no hay dónde buscar",
    cancelled: "Cancelado",

    field_type: "Tipo",
    field_project: "Proyecto",
    field_scope: "Ámbito",
    field_session: "Sesión",
    field_topic: "Tema",
    field_started: "Empezó",
    field_ended: "Terminó",
    field_summary: "Resumen",
    session_active: "activa",
    timeline_session: "Sesión: {session}",
    timeline_focus: "Foco: #{id} {title} | {total} memoria(s) en total",
    timeline_focus_marker: " FOCO",

    cloud_server: "Servidor:    ",
    cloud_background: "En segundo plano: ",
    cloud_replicating: "Replicando:  ",
    cloud_enrolled: "Inscritos:   ",
    cloud_queued: "En cola:     ",
    cloud_deferred: "Aplazados:   ",
    cloud_not_configured: "sin configurar",
    cloud_enabled: "activada",
    cloud_disabled: "desactivada",
    cloud_none: "ninguno",
    cloud_unknown: "desconocido",
    cloud_mutations: "{count} cambio(s)",
    cloud_deferred_dead: "{deferred} aplazados, {dead} descartados",
    cloud_unreadable: "No se pudo leer el almacén: {reason}",
    cloud_configure_hint: "Configúralo con: leteo cloud config set --server URL --token TOKEN \
                           --enable",
    cloud_state: "Estado:      ",
    cloud_failures: "{count} fallo(s) seguidos",
    cloud_backoff: "esperando hasta {until}",

    menu_start_setup: "Empezar configuración",
    menu_dashboard: "Panel",
    menu_cloud: "Replicación en la nube",
    menu_options: "Opciones",
    menu_help: "Ayuda",
    menu_quit: "Salir",
    menu_uninstall: "Desinstalar Leteo",
    uninstall_heading: "¿Quitar Leteo de esta máquina?",
    uninstall_agents: "{count} agente(s) en los que está configurado",
    uninstall_warning: "Se va todo lo de arriba. Esto no se puede deshacer.",

    delete_memory: "¿Borrar la memoria #{id}?",
    delete_prompt: "¿Borrar el prompt #{id}?",
    delete_session: "¿Borrar la sesión {id}?",
    delete_project: "¿Borrar el proyecto {name}?",
    delete_permanent_warning: "Esto no se puede deshacer.",
    delete_prompts_warning: "Las memorias se pueden recuperar. Los prompts no.",
    delete_recoverable: "Esto se puede recuperar del almacén.",
    gone_permanently: "borrada para siempre",
    gone: "borrada",
    count_memories: "{count} memoria(s)",
    count_sessions: "{count} sesión(es)",
    count_prompts: "{count} prompt(s)",
    copied_to_clipboard: "Copiados {count} caracteres al portapapeles",
    data_refreshed: "Datos actualizados",
    deleted_memory: "Memoria #{id} {gone}",
    deleted_prompt: "Prompt #{id} borrado",
    deleted_session: "Sesión {id} {gone}, con {memories} memoria(s) y {prompts} prompt(s)",
    deleted_project: "Proyecto {name} {gone}, con {memories} memoria(s), \
                      {sessions} sesión(es) y {prompts} prompt(s)",
    sessions_kept: " — se conservaron {count} sesión(es): tienen filas de otros proyectos",
    refreshed_query: "\"{query}\": {observations} memoria(s), {sessions} sesión(es), \
                      {prompts} prompt(s)",

    keys_confirm: "y confirmar  n/Esc cancelar",
    keys_confirm_footer: "y borrar    cualquier otra tecla cancela",
    keys_confirm_window: "y  borrar            cualquier otra tecla  cancela",
    keys_home: "j/k moverse  Intro elegir  / buscar  ? ayuda  q salir",
    keys_query: "escribe para buscar sobre la marcha  Ctrl-U limpiar  Intro/Esc volver a la lista",
    keys_filters: "j/k proyecto  espacio marcar  f/Esc listo  q salir",
    keys_dashboard_searching: "j/k elegir  Intro abrir  Tab lista  f filtrar  / editar  \
                               Esc quitar búsqueda",
    keys_dashboard_sessions: "j/k sesión  Intro abrir  Tab lista  f filtrar  / buscar  Esc volver",
    keys_dashboard_prompts: "j/k prompt  Intro leer  Tab lista  f filtrar  / buscar  Esc volver",
    keys_dashboard: "j/k elegir  Intro detalle  Tab lista  f filtrar  / buscar  y copiar  d borrar",
    keys_detail: "j/k desplazar  Intro/t cronología  y copiar  d borrar  Esc volver  q salir",
    keys_session: "j/k elegir  RePág/Fin moverse  Intro detalle  y copiar  d borrar  Esc volver",
    keys_timeline: "j/k elegir  Intro detalle  Esc volver  / buscar  q salir",
    keys_setup: "j/k elegir  espacio marcar  Intro continuar  Retroceso atrás  Esc cancelar",
    keys_options: "j/k mover  Intro elegir  Esc volver",
    keys_cloud: "R actualizar  Esc volver  q salir",
    keys_help: "Intro/Esc volver  q salir",

    help_body: "\
Moverse
  j / Abajo      bajar la selección o desplazar
  k / Arriba     subir la selección o desplazar
  RePág / AvPág  una pantalla entera
  Fin / Inicio   el final de la lista, y el principio
  Tab            panel: mostrar la lista siguiente —
                 memorias, sesiones, prompts
  Intro          abrir la selección / cronología
  Esc            cancelar o volver

Acotar las listas
  f              filtrar por proyecto; espacio marca y
                 desmarca. Sin marcas salen todos.
  /              buscar. Va sobre la marcha, y casa la
                 palabra que escribes por su principio.
                 Los dos filtros se aplican a la vez.
  Esc            salir de los filtros, luego quitar la
                 búsqueda, luego salir — en ese orden.

Vistas
  g / r          panel, sobre las memorias
  s              panel, sobre las sesiones
  t              cronología desde el detalle
  S              configuración de agentes
  c              replicación en la nube
  ? / h          esta ayuda

Acciones
  y              copiar la memoria elegida al portapapeles
  d              borrar lo que hay bajo el cursor — una
                 memoria, un prompt, una sesión con todo lo
                 que registró, o un proyecto entero. Pregunta.
  D              lo mismo, para siempre. Pregunta.
                 Las memorias vuelven; los prompts no.

General
  R              recargar los datos del panel
  Ctrl-U         limpiar la búsqueda
  q              salir (fuera de la búsqueda)",
};
