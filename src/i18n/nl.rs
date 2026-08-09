//! Dutch.

use super::Screens;

pub const SCREENS: Screens = Screens {
    found_engram: "Er is een Engram-installatie gevonden",
    engram_counts: "{observations} herinneringen, {sessions} sessies, {prompts} prompts, \
                    {relations} relaties",
    adopt_question: "Deze herinneringen overnemen in Leteo?",
    adopt_yes: "Ja, haal ze over",
    adopt_no: "Nee, leeg beginnen",
    choose_agents: "Welke agents moet Leteo instellen?",
    will_be_removed: "wordt verwijderd",
    will_be_installed: "wordt geïnstalleerd",
    hooks_question: "De lifecycle-hooks installeren die het onthouden in {agents} \
                     automatisch maken?",
    yes: "Ja",
    hooks_no: "Nee, alleen de MCP-gereedschappen",
    voice_question: "Hoeveel moet {name} hardop zeggen?",
    voice_all: "begroeting, tips, vangsten en herinneringen",
    voice_reminders: "alleen de herinnering om op te slaan",
    voice_quiet: "niets, zelfs niet de herinnering om op te slaan",
    interface_question: "In welke taal moet Leteo tegen je praten?",
    interface_hint_first: "  Leteo's eigen schermen: de panelen, de menu's, de hulp en deze pagina.",
    interface_hint_second: "  Wat {name} zegt en waarin herinneringen staan, kies je apart.",
    voice_language_question: "In welke taal moet {name} spreken?",
    voice_language_same: "als Leteo",
    voice_language_same_detail: "de taal die Leteo zelf spreekt",
    voice_language_hint: "  {name} praat ook in het gesprek met je agent, niet alleen hier.",
    memory_language_question: "In welke taal moeten herinneringen geschreven worden?",
    language_auto: "auto",
    language_auto_detail: "de taal waarin je schrijft, welke dat ook is",
    language_pinned_detail: "altijd, in welke taal men je ook schrijft",
    language_kept_warning: "  Al opgeslagen herinneringen houden de taal waarin ze geschreven zijn.",
    language_split_warning_first: "  Dit wijzigen laat de opslag in twee talen achter, en een \
                                   zoekopdracht vindt",
    language_split_warning_second: "  de helft waarin ze gesteld wordt.",
    language_other_hint: "  Elke andere taal: zet \"language\" in settings.json.",
    nothing_changed: "  Er is niets gewijzigd.",
    legend: "  spatie kiezen   enter verder   backspace terug   esc stoppen",

    options_question: "Wat wil je wijzigen?",
    option_interface: "Taal van Leteo",
    option_voice_language: "Taal van {name}",
    option_memory_language: "Taal van de herinneringen",
    option_voice: "Stem van {name}",
    preferences_saved: "Voorkeuren opgeslagen",

    could_not_adopt: "  overnemen mislukt: {error}",
    could_not_save: "  voorkeuren niet opgeslagen: {error}",
    could_not_configure: "  {agent} instellen mislukt: {error}",
    could_not_remove: "  verwijderen uit {agent} mislukt: {error}",
    removed_from: "  verwijderd uit {agent}",
    restart_them: "\n  start ze opnieuw op zodat het aankomt",

    empty_dashboard_what_happens: "Herinneringen verschijnen hier zodra je agents ze opslaan.",
    empty_dashboard_keys: "Druk op Esc voor het menu, of ? voor hulp.",
    setup_cancelled: "Instellen afgebroken. Er is niets gewijzigd.",
    setup_failed: "instellen mislukt: {error}",

    panel_setup: " Instellen ",
    panel_dashboard: " Overzicht ",
    panel_detail: " Detail ",
    panel_content: " Inhoud ",
    panel_session: " Sessie ",
    panel_timeline: " Tijdlijn ",
    panel_context: " Context ",
    panel_session_timeline: " Tijdlijn van de sessie ",
    panel_help: " Hulp ",
    panel_options: " Opties ",
    panel_cloud: " Cloudreplicatie - alleen lezen ",
    panel_filters: " FILTERS ",
    panel_filters_count: " FILTERS ({count}) ",
    panel_recorded: " Vastgelegd ({count}) ",
    list_observations: " Waarnemingen",
    list_sessions: " Sessies",
    list_prompts: " Prompts",
    scope_one_project: " in {project} ",
    scope_many_projects: " in {count} projecten ",
    list_matching: " die passen bij \"{query}\"",
    list_position: " {position} van {total} ",
    search_placeholder: "herinneringen zoeken",

    stat_observations: "WAARNEMINGEN",
    stat_sessions: "SESSIES",
    stat_prompts: "PROMPTS",
    page_home: "BEGIN",
    page_dashboard: "OVERZICHT",
    page_detail: "DETAIL",
    page_session: "SESSIE",
    page_timeline: "TIJDLIJN",
    page_setup: "INSTELLEN",
    page_cloud: "CLOUD",
    page_help: "HULP",
    page_options: "OPTIES",

    no_observations: "Geen waarnemingen gevonden",
    no_sessions: "Geen sessies gevonden",
    no_prompts: "Geen prompts gevonden",
    no_projects: "Nog geen projecten",
    no_observation_selected: "Geen waarneming gekozen",
    no_session_selected: "Geen sessie gekozen",
    no_timeline_loaded: "Geen tijdlijn geladen",
    no_summary: "Geen samenvatting",
    nothing_to_search: "Nog niets opgeslagen — er valt niets te zoeken",
    cancelled: "Afgebroken",

    field_type: "Soort",
    field_project: "Project",
    field_scope: "Bereik",
    field_session: "Sessie",
    field_topic: "Onderwerp",
    field_started: "Begonnen",
    field_ended: "Geëindigd",
    field_summary: "Samenvatting",
    session_active: "actief",
    timeline_session: "Sessie: {session}",
    timeline_focus: "Focus: #{id} {title} | {total} waarneming(en) in totaal",
    timeline_focus_marker: "FOCUS",

    cloud_server: "Server:      ",
    cloud_background: "Achtergrond: ",
    cloud_replicating: "Repliceren:  ",
    cloud_enrolled: "Aangemeld:   ",
    cloud_queued: "In wachtrij: ",
    cloud_deferred: "Uitgesteld:  ",
    cloud_not_configured: "niet ingesteld",
    cloud_enabled: "aan",
    cloud_disabled: "uit",
    cloud_none: "geen",
    cloud_unknown: "onbekend",
    cloud_mutations: "{count} wijziging(en)",
    cloud_deferred_dead: "{deferred} uitgesteld, {dead} dood",
    cloud_unreadable: "De opslag kon niet gelezen worden: {reason}",
    cloud_configure_hint: "Instellen met: leteo cloud config set --server URL --token TOKEN \
                           --enable",
    cloud_state: "Toestand:    ",
    cloud_failures: "{count} mislukking(en) op rij",
    cloud_backoff: "wacht tot {until}",

    menu_start_setup: "Instellen starten",
    menu_dashboard: "Overzicht",
    menu_cloud: "Cloudreplicatie",
    menu_options: "Opties",
    menu_help: "Hulp",
    menu_quit: "Stoppen",
    menu_uninstall: "Leteo verwijderen",
    uninstall_heading: "Leteo van deze machine halen?",
    uninstall_agents: "{count} agent(s) waarin het is ingesteld",
    uninstall_warning: "Alles hierboven gaat weg. Dit kan niet ongedaan gemaakt worden.",

    delete_memory: "Herinnering #{id} wissen?",
    delete_prompt: "Prompt #{id} wissen?",
    delete_session: "Sessie {id} wissen?",
    delete_project: "Project {name} wissen?",
    delete_permanent_warning: "Dit kan niet ongedaan gemaakt worden.",
    delete_prompts_warning: "Herinneringen zijn terug te halen. Prompts niet.",
    delete_recoverable: "Dit is uit de opslag terug te halen.",
    gone_permanently: "voorgoed gewist",
    gone: "gewist",
    count_memories: "{count} herinnering(en)",
    count_sessions: "{count} sessie(s)",
    count_prompts: "{count} prompt(s)",
    copied_to_clipboard: "{count} tekens naar het klembord gekopieerd",
    data_refreshed: "Gegevens herladen",
    deleted_memory: "Herinnering #{id} {gone}",
    deleted_prompt: "Prompt #{id} gewist",
    deleted_session: "Sessie {id} {gone}, met {memories} herinnering(en) en {prompts} prompt(s)",
    deleted_project: "Project {name} {gone}, met {memories} herinnering(en), \
                      {sessions} sessie(s) en {prompts} prompt(s)",
    sessions_kept: " — {count} sessie(s) behouden, die dragen rijen van andere projecten",
    refreshed_query: "\"{query}\": {observations} waarneming(en), {sessions} sessie(s), \
                      {prompts} prompt(s)",

    keys_confirm: "y bevestigen  n/Esc afbreken",
    keys_confirm_footer: "y wissen    elke andere toets breekt af",
    keys_confirm_window: "y  wissen            elke andere toets  afbreken",
    keys_home: "j/k bewegen  Enter kiezen  / zoeken  ? hulp  q stoppen",
    keys_query: "typ om te zoeken  Ctrl-U wissen  Enter/Esc terug naar de lijst",
    keys_filters: "j/k project  spatie merken  f/Esc klaar  q stoppen",
    keys_dashboard_searching: "j/k kiezen  Enter openen  Tab volgende  f filteren  / wijzigen  \
                               Esc wissen",
    keys_dashboard_sessions: "j/k sessie  Enter openen  Tab volgende  f filteren  / zoeken  \
                              Esc terug",
    keys_dashboard_prompts: "j/k prompt  Enter lezen  Tab volgende  f filteren  / zoeken  \
                             Esc terug",
    keys_dashboard: "j/k kiezen  Enter detail  Tab volgende  f filteren  / zoeken  y kopie  \
                     d wissen",
    keys_detail: "j/k rollen  Enter/t tijdlijn  y kopiëren  d wissen  Esc terug  q stoppen",
    keys_session: "j/k kiezen  PgDn/End verder  Enter detail  y kopiëren  d wissen  Esc terug",
    keys_timeline: "j/k kiezen  Enter detail  Esc terug  / zoeken  q stoppen",
    keys_setup: "j/k kiezen  spatie merken  Enter verder  Backspace terug  Esc afbreken",
    keys_options: "j/k kiezen  Enter wijzigen  Esc terug",
    keys_cloud: "R herladen  Esc terug  q stoppen",
    keys_help: "Enter/Esc terug  q stoppen",

    help_body: "\
Bewegen
  j / Omlaag     de keuze bewegen of omlaag rollen
  k / Omhoog     de keuze bewegen of omhoog rollen
  PgDn / PgUp    een scherm tegelijk
  End / Home     het einde van de lijst, en het begin
  Tab            overzicht: de volgende lijst tonen —
                 waarnemingen, sessies, prompts
  Enter          de keuze openen / de tijdlijn
  Esc            afbreken of terug

De lijsten smaller maken
  f              op project filteren, waar spatie merkt en
                 ontmerkt. Zonder merk geldt elk project.
  /              zoeken. Het loopt terwijl je typt, en het
                 getypte woord wordt op zijn begin gepast.
                 Beide filters gelden tegelijk, voor alle drie.
  Esc            de filters verlaten, dan de zoekopdracht laten
                 vallen, dan de pagina — in die volgorde.

Weergaven
  g / r          overzicht, op de waarnemingen
  s              overzicht, op de sessies
  t              tijdlijn vanuit het detail
  S              agents instellen
  c              cloudreplicatie
  ? / h          deze hulp

Handelingen
  y              de gekozen herinnering naar het klembord
  d              wissen wat onder de cursor staat — een
                 herinnering, een prompt, een sessie met alles
                 wat ze vastlegde, of een heel project.
                 Vraagt eerst.
  D              hetzelfde, voorgoed. Vraagt eerst.
                 Herinneringen komen terug; prompts niet.

Algemeen
  R              de gegevens van het overzicht herladen
  Ctrl-U         de zoekopdracht wissen
  q              stoppen (buiten de zoekopdracht)",
};
