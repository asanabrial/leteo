//! Polish.

use super::Screens;

pub const SCREENS: Screens = Screens {
    found_engram: "Znaleziono instalację Engrama",
    engram_counts: "{observations} wspomnień, {sessions} sesji, {prompts} promptów, \
                    {relations} relacji",
    adopt_question: "Przenieść te wspomnienia do Leteo?",
    adopt_yes: "Tak, przenieś je",
    adopt_no: "Nie, zacznij od zera",
    choose_agents: "Które agenty ma skonfigurować Leteo?",
    will_be_removed: "zostanie usunięty",
    will_be_installed: "zostanie zainstalowany",
    hooks_question: "Zainstalować haki cyklu życia, dzięki którym pamięć w {agents} \
                     działa sama?",
    yes: "Tak",
    hooks_no: "Nie, tylko narzędzia MCP",
    voice_question: "Ile {name} ma mówić na głos?",
    voice_all: "powitanie, podpowiedzi, przechwyty i przypomnienia",
    voice_reminders: "tylko przypomnienie o zapisaniu",
    voice_quiet: "nic, nawet przypomnienia o zapisaniu",
    interface_question: "W jakim języku Leteo ma do ciebie mówić?",
    interface_hint_first: "  Własne ekrany Leteo: panele, menu, pomoc i ta strona.",
    interface_hint_second: "  To, co mówi {name}, i język wspomnień wybiera się osobno.",
    voice_language_question: "W jakim języku ma mówić {name}?",
    voice_language_same: "jak Leteo",
    voice_language_same_detail: "język, którym mówi samo Leteo",
    voice_language_hint: "  {name} odzywa się w rozmowie z twoim agentem, nie tylko tutaj.",
    memory_language_question: "W jakim języku mają być pisane wspomnienia?",
    language_auto: "auto",
    language_auto_detail: "język, w którym piszesz, jakikolwiek by był",
    language_pinned_detail: "zawsze, niezależnie od tego, jak się do ciebie pisze",
    language_kept_warning: "  Zapisane już wspomnienia zachowują język, w którym powstały.",
    language_split_warning_first: "  Zmiana zostawia zbiór w dwóch językach, a wyszukiwanie \
                                   znajdzie",
    language_split_warning_second: "  tę połowę, w której je zadano.",
    language_other_hint: "  Dowolny inny język: ustaw \"language\" w settings.json.",
    nothing_changed: "  Nic nie zostało zmienione.",
    legend: "  spacja wybór   enter dalej   backspace wstecz   esc wyjście",

    options_question: "Co chcesz zmienić?",
    option_interface: "Język Leteo",
    option_voice_language: "Język: {name}",
    option_memory_language: "Język wspomnień",
    option_voice: "Głos: {name}",
    preferences_saved: "Zapisano ustawienia",

    could_not_adopt: "  nie udało się przenieść: {error}",
    could_not_save: "  nie udało się zapisać ustawień: {error}",
    could_not_configure: "  nie udało się skonfigurować {agent}: {error}",
    could_not_remove: "  nie udało się usunąć z {agent}: {error}",
    removed_from: "  usunięto z {agent}",
    restart_them: "\n  uruchom je ponownie, żeby to przyjęły",

    empty_dashboard_what_happens: "Wspomnienia pojawiają się tutaj, gdy agenty je zapisują.",
    empty_dashboard_keys: "Naciśnij Esc, żeby wrócić do menu, albo ? po pomoc.",
    setup_cancelled: "Konfiguracja przerwana. Nic nie zostało zmienione.",
    setup_failed: "konfiguracja nie powiodła się: {error}",

    panel_setup: " Konfiguracja ",
    panel_dashboard: " Pulpit ",
    panel_detail: " Szczegóły ",
    panel_content: " Treść ",
    panel_session: " Sesja ",
    panel_timeline: " Oś czasu ",
    panel_context: " Kontekst ",
    panel_session_timeline: " Oś czasu sesji ",
    panel_help: " Pomoc ",
    panel_options: " Opcje ",
    panel_cloud: " Replikacja w chmurze - tylko do odczytu ",
    panel_filters: " FILTRY ",
    panel_filters_count: " FILTRY ({count}) ",
    panel_recorded: " Zapisano ({count}) ",
    list_observations: " Obserwacje",
    list_sessions: " Sesje",
    list_prompts: " Prompty",
    scope_one_project: " w {project} ",
    scope_many_projects: " w {count} projektach ",
    list_matching: " pasujące do \"{query}\"",
    list_position: " {position} z {total} ",
    search_placeholder: "szukaj wspomnień",

    stat_observations: "OBSERWACJE",
    stat_sessions: "SESJE",
    stat_prompts: "PROMPTY",
    page_home: "START",
    page_dashboard: "PULPIT",
    page_detail: "SZCZEGÓŁY",
    page_session: "SESJA",
    page_timeline: "OŚ CZASU",
    page_setup: "KONFIGURACJA",
    page_cloud: "CHMURA",
    page_help: "POMOC",
    page_options: "OPCJE",

    no_observations: "Nie znaleziono obserwacji",
    no_sessions: "Nie znaleziono sesji",
    no_prompts: "Nie znaleziono promptów",
    no_projects: "Jeszcze nie ma projektów",
    no_observation_selected: "Nie wybrano obserwacji",
    no_session_selected: "Nie wybrano sesji",
    no_timeline_loaded: "Nie wczytano osi czasu",
    no_summary: "Brak podsumowania",
    nothing_to_search: "Nic jeszcze nie zapisano — nie ma czego szukać",
    cancelled: "Przerwano",

    field_type: "Rodzaj",
    field_project: "Projekt",
    field_scope: "Zakres",
    field_session: "Sesja",
    field_topic: "Temat",
    field_started: "Początek",
    field_ended: "Koniec",
    field_summary: "Podsumowanie",
    session_active: "aktywna",
    timeline_session: "Sesja: {session}",
    timeline_focus: "Punkt: #{id} {title} | {total} obserwacji łącznie",
    timeline_focus_marker: "PUNKT",

    cloud_server: "Serwer:      ",
    cloud_background: "W tle:       ",
    cloud_replicating: "Replikacja:  ",
    cloud_enrolled: "Zapisane:    ",
    cloud_queued: "W kolejce:   ",
    cloud_deferred: "Odłożone:    ",
    cloud_not_configured: "nieskonfigurowane",
    cloud_enabled: "włączone",
    cloud_disabled: "wyłączone",
    cloud_none: "brak",
    cloud_unknown: "nieznane",
    cloud_mutations: "{count} zmian",
    cloud_deferred_dead: "{deferred} odłożonych, {dead} martwych",
    cloud_unreadable: "Nie udało się odczytać zbioru: {reason}",
    cloud_configure_hint: "Skonfiguruj: leteo cloud config set --server URL --token TOKEN \
                           --enable",
    cloud_state: "Stan:        ",
    cloud_failures: "{count} niepowodzeń z rzędu",
    cloud_backoff: "czeka do {until}",

    menu_start_setup: "Zacznij konfigurację",
    menu_dashboard: "Pulpit",
    menu_cloud: "Replikacja w chmurze",
    menu_options: "Opcje",
    menu_help: "Pomoc",
    menu_quit: "Wyjście",
    menu_uninstall: "Odinstaluj Leteo",
    uninstall_heading: "Usunąć Leteo z tej maszyny?",
    uninstall_agents: "{count} agentów, w których jest skonfigurowane",
    uninstall_warning: "Wszystko powyżej znika. Tego nie da się cofnąć.",

    delete_memory: "Usunąć wspomnienie #{id}?",
    delete_prompt: "Usunąć prompt #{id}?",
    delete_session: "Usunąć sesję {id}?",
    delete_project: "Usunąć projekt {name}?",
    delete_permanent_warning: "Tego nie da się cofnąć.",
    delete_prompts_warning: "Wspomnienia da się odzyskać. Promptów nie.",
    delete_recoverable: "To da się odzyskać ze zbioru.",
    gone_permanently: "usunięte na zawsze",
    gone: "usunięte",
    count_memories: "{count} wspomnień",
    count_sessions: "{count} sesji",
    count_prompts: "{count} promptów",
    copied_to_clipboard: "Skopiowano {count} znaków do schowka",
    data_refreshed: "Dane wczytane na nowo",
    deleted_memory: "Wspomnienie #{id} {gone}",
    deleted_prompt: "Prompt #{id} usunięty",
    deleted_session: "Sesja {id} {gone}, z {memories} wspomnieniami i {prompts} promptami",
    deleted_project: "Projekt {name} {gone}, z {memories} wspomnieniami, {sessions} sesjami \
                      i {prompts} promptami",
    sessions_kept: " — zachowano {count} sesji, trzymają wiersze innych projektów",
    refreshed_query: "\"{query}\": {observations} obserwacji, {sessions} sesji, \
                      {prompts} promptów",

    keys_confirm: "y potwierdź  n/Esc anuluj",
    keys_confirm_footer: "y usuń    każdy inny klawisz anuluje",
    keys_confirm_window: "y  usuń              każdy inny klawisz  anuluj",
    keys_home: "j/k ruch  Enter wybór  / szukaj  ? pomoc  q wyjście",
    keys_query: "pisz, żeby szukać  Ctrl-U wyczyść  Enter/Esc powrót do listy",
    keys_filters: "j/k projekt  spacja zaznacz  f/Esc gotowe  q wyjście",
    keys_dashboard_searching: "j/k wybór  Enter otwórz  Tab następna  f filtr  / edytuj  \
                               Esc wyczyść",
    keys_dashboard_sessions: "j/k sesja  Enter otwórz  Tab następna  f filtr  / szukaj  \
                              Esc wstecz",
    keys_dashboard_prompts: "j/k prompt  Enter czytaj  Tab następna  f filtr  / szukaj  \
                             Esc wstecz",
    keys_dashboard: "j/k wybór  Enter szczegóły  Tab następna  f filtr  / szukaj  y kopiuj  \
                     d usuń",
    keys_detail: "j/k przewiń  Enter/t oś czasu  y kopiuj  d usuń  Esc wstecz  q wyjście",
    keys_session: "j/k wybór  PgDn/End dalej  Enter szczegóły  y kopiuj  d usuń  Esc wstecz",
    keys_timeline: "j/k wybór  Enter szczegóły  Esc wstecz  / szukaj  q wyjście",
    keys_setup: "j/k wybór  spacja zaznacz  Enter dalej  Backspace wstecz  Esc anuluj",
    keys_options: "j/k wybór  Enter zmień  Esc wstecz",
    keys_cloud: "R odśwież  Esc wstecz  q wyjście",
    keys_help: "Enter/Esc wstecz  q wyjście",

    help_body: "\
Poruszanie się
  j / Dół        przesuń wybór albo przewiń w dół
  k / Góra       przesuń wybór albo przewiń w górę
  PgDn / PgUp    po jednym ekranie
  End / Home     koniec listy, i początek
  Tab            pulpit: pokaż następną listę —
                 obserwacje, sesje, prompty
  Enter          otwórz wybór / oś czasu
  Esc            anuluj albo wróć

Zawężanie list
  f              filtruj po projekcie; spacja zaznacza i
                 odznacza. Bez zaznaczeń — wszystkie projekty.
  /              szukaj. Działa w trakcie pisania, a pisane
                 słowo dopasowuje po początku. Oba filtry
                 działają naraz, na wszystkich trzech listach.
  Esc            wyjdź z filtrów, potem porzuć wyszukiwanie,
                 potem wyjdź ze strony — w tej kolejności.

Widoki
  g / r          pulpit, na obserwacjach
  s              pulpit, na sesjach
  t              oś czasu ze szczegółów
  S              konfiguracja agentów
  c              replikacja w chmurze
  ? / h          ta pomoc

Działania
  y              skopiuj wybrane wspomnienie do schowka
  d              usuń to, co pod kursorem — wspomnienie,
                 prompt, sesję z wszystkim, co zapisała, albo
                 cały projekt. Najpierw pyta.
  D              to samo, na zawsze. Najpierw pyta.
                 Wspomnienia wracają; prompty nie.

Ogólne
  R              wczytaj dane pulpitu na nowo
  Ctrl-U         wyczyść wyszukiwanie
  q              wyjdź (poza polem wyszukiwania)",
};
