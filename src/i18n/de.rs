//! German.

use super::Screens;

pub const SCREENS: Screens = Screens {
    found_engram: "Eine Engram-Installation wurde gefunden",
    engram_counts: "{observations} Erinnerungen, {sessions} Sitzungen, {prompts} Prompts, \
                    {relations} Beziehungen",
    adopt_question: "Diese Erinnerungen in Leteo übernehmen?",
    adopt_yes: "Ja, alle übernehmen",
    adopt_no: "Nein, leer anfangen",
    choose_agents: "Welche Agenten soll Leteo einrichten?",
    will_be_removed: "wird entfernt",
    will_be_installed: "wird eingerichtet",
    hooks_question: "Die Lifecycle-Hooks einrichten, die das Erinnern in {agents} \
                     automatisch machen?",
    yes: "Ja",
    hooks_no: "Nein, nur die MCP-Werkzeuge",
    voice_question: "Wie viel soll {name} laut sagen?",
    voice_all: "Begrüßung, Hinweise, Mitschnitte und Erinnerungen",
    voice_reminders: "nur die Erinnerung ans Speichern",
    voice_quiet: "nichts, nicht einmal die Erinnerung ans Speichern",
    interface_question: "In welcher Sprache soll Leteo mit dir sprechen?",
    interface_hint_first: "  Leteos eigene Schirme: die Panels, die Menüs, die Hilfe, diese Seite.",
    interface_hint_second: "  Was {name} sagt und worin Erinnerungen stehen, wird getrennt gewählt.",
    voice_language_question: "In welcher Sprache soll {name} sprechen?",
    voice_language_same: "wie Leteo",
    voice_language_same_detail: "die Sprache, die Leteo selbst spricht",
    voice_language_hint: "  {name} spricht auch in der Unterhaltung deines Agenten, nicht nur hier.",
    memory_language_question: "In welcher Sprache sollen Erinnerungen geschrieben werden?",
    language_auto: "auto",
    language_auto_detail: "die Sprache, in der du schreibst, welche auch immer",
    language_pinned_detail: "immer, egal in welcher man dir schreibt",
    language_kept_warning: "  Schon gespeicherte Erinnerungen behalten ihre Sprache.",
    language_split_warning_first: "  Das lässt den Speicher in zwei Sprachen zurück, und eine \
                                   Suche findet",
    language_split_warning_second: "  die Hälfte, in der sie gestellt wird.",
    language_other_hint: "  Jede andere Sprache: \"language\" in settings.json setzen.",
    nothing_changed: "  Es wurde nichts geändert.",
    legend: "  Leer wählen   Enter weiter   Rück zurück   Esc beenden",

    options_question: "Was möchtest du ändern?",
    option_interface: "Sprache von Leteo",
    option_voice_language: "{name}s Sprache",
    option_memory_language: "Sprache der Erinnerungen",
    option_voice: "{name}s Stimme",
    preferences_saved: "Einstellungen gespeichert",

    could_not_adopt: "  Übernahme fehlgeschlagen: {error}",
    could_not_save: "  Einstellungen nicht gespeichert: {error}",
    could_not_configure: "  {agent} nicht einrichtbar: {error}",
    could_not_remove: "  aus {agent} nicht entfernbar: {error}",
    removed_from: "  aus {agent} entfernt",
    restart_them: "\n  starte sie neu, damit es greift",

    empty_dashboard_what_happens: "Erinnerungen erscheinen hier, sobald deine Agenten sie \
                                   speichern.",
    empty_dashboard_keys: "Esc für das Menü, oder ? für die Hilfe.",
    setup_cancelled: "Einrichtung abgebrochen. Es wurde nichts geändert.",
    setup_failed: "Einrichtung fehlgeschlagen: {error}",

    panel_setup: " Einrichtung ",
    panel_dashboard: " Übersicht ",
    panel_detail: " Detail ",
    panel_content: " Inhalt ",
    panel_session: " Sitzung ",
    panel_timeline: " Verlauf ",
    panel_context: " Kontext ",
    panel_session_timeline: " Verlauf der Sitzung ",
    panel_help: " Hilfe ",
    panel_options: " Optionen ",
    panel_cloud: " Cloud-Replikation - nur lesen ",
    panel_filters: " FILTER ",
    panel_filters_count: " FILTER ({count}) ",
    panel_recorded: " Aufgezeichnet ({count}) ",
    list_observations: " Beobachtungen",
    list_sessions: " Sitzungen",
    list_prompts: " Prompts",
    scope_one_project: " in {project} ",
    scope_many_projects: " in {count} Projekten ",
    list_matching: " passend zu \"{query}\"",
    list_position: " {position} von {total} ",
    search_placeholder: "Erinnerungen suchen",

    stat_observations: "BEOBACHTUNGEN",
    stat_sessions: "SITZUNGEN",
    stat_prompts: "PROMPTS",
    page_home: "START",
    page_dashboard: "ÜBERSICHT",
    page_detail: "DETAIL",
    page_session: "SITZUNG",
    page_timeline: "VERLAUF",
    page_setup: "EINRICHTUNG",
    page_cloud: "CLOUD",
    page_help: "HILFE",
    page_options: "OPTIONEN",

    no_observations: "Keine Beobachtungen gefunden",
    no_sessions: "Keine Sitzungen gefunden",
    no_prompts: "Keine Prompts gefunden",
    no_projects: "Noch keine Projekte",
    no_observation_selected: "Keine Beobachtung gewählt",
    no_session_selected: "Keine Sitzung gewählt",
    no_timeline_loaded: "Kein Verlauf geladen",
    no_summary: "Keine Zusammenfassung",
    nothing_to_search: "Noch nichts gespeichert — es gibt nichts zu suchen",
    cancelled: "Abgebrochen",

    field_type: "Art",
    field_project: "Projekt",
    field_scope: "Geltung",
    field_session: "Sitzung",
    field_topic: "Thema",
    field_started: "Beginn",
    field_ended: "Ende",
    field_summary: "Zusammenfassung",
    session_active: "aktiv",
    timeline_session: "Sitzung: {session}",
    timeline_focus: "Fokus: #{id} {title} | {total} Beobachtung(en) gesamt",
    timeline_focus_marker: "FOKUS",

    cloud_server: "Server:      ",
    cloud_background: "Im Hintergrund: ",
    cloud_replicating: "Repliziert:  ",
    cloud_enrolled: "Angemeldet:  ",
    cloud_queued: "In Warte:    ",
    cloud_deferred: "Aufgeschoben: ",
    cloud_not_configured: "nicht eingerichtet",
    cloud_enabled: "aktiv",
    cloud_disabled: "abgeschaltet",
    cloud_none: "keine",
    cloud_unknown: "unbekannt",
    cloud_mutations: "{count} Änderung(en)",
    cloud_deferred_dead: "{deferred} aufgeschoben, {dead} tot",
    cloud_unreadable: "Der Speicher war nicht lesbar: {reason}",
    cloud_configure_hint: "Einrichten mit: leteo cloud config set --server URL --token TOKEN \
                           --enable",
    cloud_state: "Zustand:     ",
    cloud_failures: "{count} Fehlschlag/Fehlschläge in Folge",
    cloud_backoff: "wartet bis {until}",

    menu_start_setup: "Einrichtung starten",
    menu_dashboard: "Übersicht",
    menu_cloud: "Cloud-Replikation",
    menu_options: "Optionen",
    menu_help: "Hilfe",
    menu_quit: "Beenden",
    menu_uninstall: "Leteo deinstallieren",
    uninstall_heading: "Leteo von diesem Rechner entfernen?",
    uninstall_agents: "{count} Agent(en), in denen es eingerichtet ist",
    uninstall_warning: "Alles darüber ist weg. Das lässt sich nicht rückgängig machen.",

    delete_memory: "Erinnerung #{id} löschen?",
    delete_prompt: "Prompt #{id} löschen?",
    delete_session: "Sitzung {id} löschen?",
    delete_project: "Projekt {name} löschen?",
    delete_permanent_warning: "Das lässt sich nicht rückgängig machen.",
    delete_prompts_warning: "Erinnerungen lassen sich zurückholen. Prompts nicht.",
    delete_recoverable: "Das lässt sich aus dem Speicher zurückholen.",
    gone_permanently: "endgültig gelöscht",
    gone: "gelöscht",
    count_memories: "{count} Erinnerung(en)",
    count_sessions: "{count} Sitzung(en)",
    count_prompts: "{count} Prompt(s)",
    copied_to_clipboard: "{count} Zeichen in die Zwischenablage kopiert",
    data_refreshed: "Daten neu geladen",
    deleted_memory: "Erinnerung #{id} {gone}",
    deleted_prompt: "Prompt #{id} gelöscht",
    deleted_session: "Sitzung {id} {gone}, mit {memories} Erinnerung(en) und \
                      {prompts} Prompt(s)",
    deleted_project: "Projekt {name} {gone}, mit {memories} Erinnerung(en), \
                      {sessions} Sitzung(en) und {prompts} Prompt(s)",
    sessions_kept: " — {count} Sitzung(en) behalten, sie tragen fremde Projekte",
    refreshed_query: "\"{query}\": {observations} Beobachtung(en), {sessions} Sitzung(en), \
                      {prompts} Prompt(s)",

    keys_confirm: "y bestätigen  n/Esc abbrechen",
    keys_confirm_footer: "y löschen    jede andere Taste bricht ab",
    keys_confirm_window: "y  löschen           jede andere Taste  abbrechen",
    keys_home: "j/k bewegen  Enter wählen  / suchen  ? Hilfe  q beenden",
    keys_query: "tippen zum Suchen  Ctrl-U leeren  Enter/Esc zurück zur Liste",
    keys_filters: "j/k Projekt  Leer setzen  f/Esc fertig  q beenden",
    keys_dashboard_searching: "j/k wählen  Enter öffnen  Tab nächste  f filtern  / ändern  \
                               Esc leeren",
    keys_dashboard_sessions: "j/k Sitzung  Enter öffnen  Tab nächste  f filtern  / suchen  \
                              Esc zurück",
    keys_dashboard_prompts: "j/k Prompt  Enter lesen  Tab nächste  f filtern  / suchen  \
                             Esc zurück",
    keys_dashboard: "j/k wählen  Enter Detail  Tab nächste  f filtern  / suchen  y kopieren  \
                     d weg",
    keys_detail: "j/k rollen  Enter/t Verlauf  y kopieren  d löschen  Esc zurück  q beenden",
    keys_session: "j/k wählen  PgDn/End weiter  Enter Detail  y kopieren  d löschen  Esc zurück",
    keys_timeline: "j/k wählen  Enter Detail  Esc zurück  / suchen  q beenden",
    keys_setup: "j/k wählen  Leer setzen  Enter weiter  Rück zurück  Esc abbrechen",
    keys_options: "j/k wählen  Enter ändern  Esc zurück",
    keys_cloud: "R neu laden  Esc zurück  q beenden",
    keys_help: "Enter/Esc zurück  q beenden",

    help_body: "\
Bewegen
  j / Ab         Auswahl bewegen oder nach unten rollen
  k / Auf        Auswahl bewegen oder nach oben rollen
  PgDn / PgUp    ein Schirm auf einmal
  End / Home     das Ende der Liste, und der Anfang
  Tab            Übersicht: die nächste Liste zeigen —
                 Beobachtungen, Sitzungen, Prompts
  Enter          Auswahl öffnen / Verlauf
  Esc            abbrechen oder zurück

Die Listen einengen
  f              nach Projekt filtern, wo Leer setzt und
                 löst. Ohne Marke gilt jedes Projekt.
  /              suchen. Läuft beim Tippen, und trifft das
                 getippte Wort an seinem Anfang. Beide
                 Filter gelten zugleich, für alle drei Listen.
  Esc            die Filter verlassen, dann die Suche fallen
                 lassen, dann die Seite — in dieser Reihenfolge.

Ansichten
  g / r          Übersicht, auf den Beobachtungen
  s              Übersicht, auf den Sitzungen
  t              Verlauf aus dem Detail
  S              Einrichtung der Agenten
  c              Cloud-Replikation
  ? / h          diese Hilfe

Aktionen
  y              die gewählte Erinnerung in die Zwischenablage
  d              löschen, was unter dem Zeiger steht — eine
                 Erinnerung, ein Prompt, eine Sitzung mit allem
                 Aufgezeichneten, oder ein ganzes Projekt.
                 Fragt vorher.
  D              dasselbe, endgültig. Fragt vorher.
                 Erinnerungen kommen zurück; Prompts nicht.

Allgemein
  R              die Daten der Übersicht neu laden
  Ctrl-U         die Suche leeren
  q              beenden (außerhalb der Suche)",
};
