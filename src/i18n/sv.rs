//! Swedish.

use super::Screens;

pub const SCREENS: Screens = Screens {
    found_engram: "En Engram-installation hittades",
    engram_counts: "{observations} minnen, {sessions} sessioner, {prompts} prompter, \
                    {relations} relationer",
    adopt_question: "Ta över de här minnena till Leteo?",
    adopt_yes: "Ja, hämta hit dem",
    adopt_no: "Nej, börja tomt",
    choose_agents: "Vilka agenter ska Leteo ställa in?",
    will_be_removed: "tas bort",
    will_be_installed: "installeras",
    hooks_question: "Installera livscykelkrokarna som gör minnet automatiskt i {agents}?",
    yes: "Ja",
    hooks_no: "Nej, bara MCP-verktygen",
    voice_question: "Hur mycket ska {name} säga högt?",
    voice_all: "hälsning, tips, fångster och påminnelser",
    voice_reminders: "bara påminnelsen om att spara",
    voice_quiet: "ingenting, inte ens påminnelsen om att spara",
    interface_question: "Vilket språk ska Leteo tala med dig på?",
    interface_hint_first: "  Leteos egna skärmar: panelerna, menyerna, hjälpen och den här sidan.",
    interface_hint_second: "  Vad {name} säger och vilket språk minnen skrivs på väljs var för sig.",
    voice_language_question: "Vilket språk ska {name} tala?",
    voice_language_same: "som Leteo",
    voice_language_same_detail: "det språk Leteo självt talar",
    voice_language_hint: "  {name} talar i samtalet med din agent, inte bara här.",
    memory_language_question: "Vilket språk ska minnena skrivas på?",
    language_auto: "auto",
    language_auto_detail: "det språk du skriver på, vilket det än är",
    language_pinned_detail: "alltid, oavsett vad man skriver till dig på",
    language_kept_warning: "  Redan sparade minnen behåller språket de skrevs på.",
    language_split_warning_first: "  Att ändra detta lämnar lagret på två språk, och en \
                                   sökning hittar",
    language_split_warning_second: "  den halva den ställs på.",
    language_other_hint: "  Vilket annat språk som helst: sätt \"language\" i settings.json.",
    nothing_changed: "  Ingenting ändrades.",
    legend: "  mellanslag välj   enter fortsätt   backsteg tillbaka   esc avsluta",

    options_question: "Vad vill du ändra?",
    option_interface: "Leteos språk",
    option_voice_language: "{name}s språk",
    option_memory_language: "Minnenas språk",
    option_voice: "{name}s röst",
    preferences_saved: "Inställningar sparade",

    could_not_adopt: "  kunde inte ta över: {error}",
    could_not_save: "  kunde inte spara inställningarna: {error}",
    could_not_configure: "  kunde inte ställa in {agent}: {error}",
    could_not_remove: "  kunde inte ta bort från {agent}: {error}",
    removed_from: "  borttagen från {agent}",
    restart_them: "\n  starta om dem så att det slår igenom",

    empty_dashboard_what_happens: "Minnen dyker upp här allteftersom dina agenter sparar dem.",
    empty_dashboard_keys: "Tryck Esc för menyn, eller ? för hjälp.",
    setup_cancelled: "Inställningen avbruten. Ingenting ändrades.",
    setup_failed: "inställningen misslyckades: {error}",

    panel_setup: " Inställning ",
    panel_dashboard: " Översikt ",
    panel_detail: " Detalj ",
    panel_content: " Innehåll ",
    panel_session: " Session ",
    panel_timeline: " Tidslinje ",
    panel_context: " Sammanhang ",
    panel_session_timeline: " Sessionens tidslinje ",
    panel_help: " Hjälp ",
    panel_options: " Alternativ ",
    panel_cloud: " Molnreplikering - endast läsning ",
    panel_filters: " FILTER ",
    panel_filters_count: " FILTER ({count}) ",
    panel_recorded: " Antecknat ({count}) ",
    list_observations: " Iakttagelser",
    list_sessions: " Sessioner",
    list_prompts: " Prompter",
    scope_one_project: " i {project} ",
    scope_many_projects: " i {count} projekt ",
    list_matching: " som stämmer med \"{query}\"",
    list_position: " {position} av {total} ",
    search_placeholder: "sök minnen",

    stat_observations: "IAKTTAGELSER",
    stat_sessions: "SESSIONER",
    stat_prompts: "PROMPTER",
    page_home: "START",
    page_dashboard: "ÖVERSIKT",
    page_detail: "DETALJ",
    page_session: "SESSION",
    page_timeline: "TIDSLINJE",
    page_setup: "INSTÄLLNING",
    page_cloud: "MOLN",
    page_help: "HJÄLP",
    page_options: "ALTERNATIV",

    no_observations: "Inga iakttagelser hittades",
    no_sessions: "Inga sessioner hittades",
    no_prompts: "Inga prompter hittades",
    no_projects: "Inga projekt än",
    no_observation_selected: "Ingen iakttagelse vald",
    no_session_selected: "Ingen session vald",
    no_timeline_loaded: "Ingen tidslinje inläst",
    no_summary: "Ingen sammanfattning",
    nothing_to_search: "Ingenting sparat än — det finns inget att söka i",
    cancelled: "Avbrutet",

    field_type: "Slag",
    field_project: "Projekt",
    field_scope: "Omfång",
    field_session: "Session",
    field_topic: "Ämne",
    field_started: "Början",
    field_ended: "Slut",
    field_summary: "Sammanfattning",
    session_active: "aktiv",
    timeline_session: "Session: {session}",
    timeline_focus: "Fokus: #{id} {title} | {total} iakttagelse(r) totalt",
    timeline_focus_marker: "FOKUS",

    cloud_server: "Server:      ",
    cloud_background: "I bakgrunden: ",
    cloud_replicating: "Replikerar:  ",
    cloud_enrolled: "Anmälda:     ",
    cloud_queued: "I kö:        ",
    cloud_deferred: "Uppskjutna:  ",
    cloud_not_configured: "inte inställt",
    cloud_enabled: "påslaget",
    cloud_disabled: "avstängt",
    cloud_none: "inga",
    cloud_unknown: "okänt",
    cloud_mutations: "{count} ändring(ar)",
    cloud_deferred_dead: "{deferred} uppskjutna, {dead} döda",
    cloud_unreadable: "Lagret gick inte att läsa: {reason}",
    cloud_configure_hint: "Ställ in med: leteo cloud config set --server URL --token TOKEN \
                           --enable",
    cloud_state: "Tillstånd:   ",
    cloud_failures: "{count} misslyckande(n) i rad",
    cloud_backoff: "väntar till {until}",

    menu_start_setup: "Börja inställningen",
    menu_dashboard: "Översikt",
    menu_cloud: "Molnreplikering",
    menu_options: "Alternativ",
    menu_help: "Hjälp",
    menu_quit: "Avsluta",
    menu_uninstall: "Avinstallera Leteo",
    uninstall_heading: "Ta bort Leteo från den här maskinen?",
    uninstall_agents: "{count} agent(er) där det är inställt",
    uninstall_warning: "Allt ovanför försvinner. Det går inte att ångra.",

    delete_memory: "Radera minnet #{id}?",
    delete_prompt: "Radera prompten #{id}?",
    delete_session: "Radera sessionen {id}?",
    delete_project: "Radera projektet {name}?",
    delete_permanent_warning: "Det går inte att ångra.",
    delete_prompts_warning: "Minnen går att få tillbaka. Prompter gör det inte.",
    delete_recoverable: "Det här går att hämta tillbaka ur lagret.",
    gone_permanently: "raderat för gott",
    gone: "raderat",
    count_memories: "{count} minne(n)",
    count_sessions: "{count} session(er)",
    count_prompts: "{count} prompt(er)",
    copied_to_clipboard: "Kopierade {count} tecken till urklipp",
    data_refreshed: "Uppgifterna omlästa",
    deleted_memory: "Minnet #{id} {gone}",
    deleted_prompt: "Prompten #{id} raderad",
    deleted_session: "Sessionen {id} {gone}, med {memories} minne(n) och {prompts} prompt(er)",
    deleted_project: "Projektet {name} {gone}, med {memories} minne(n), \
                      {sessions} session(er) och {prompts} prompt(er)",
    sessions_kept: " — {count} session(er) behållna, de bär rader från andra projekt",
    refreshed_query: "\"{query}\": {observations} iakttagelse(r), {sessions} session(er), \
                      {prompts} prompt(er)",

    keys_confirm: "y bekräfta  n/Esc avbryt",
    keys_confirm_footer: "y radera    varje annan tangent avbryter",
    keys_confirm_window: "y  radera            varje annan tangent  avbryt",
    keys_home: "j/k flytta  Enter välj  / sök  ? hjälp  q avsluta",
    keys_query: "skriv för att söka  Ctrl-U rensa  Enter/Esc tillbaka till listan",
    keys_filters: "j/k projekt  mellanslag märk  f/Esc klart  q avsluta",
    keys_dashboard_searching: "j/k välj  Enter öppna  Tab nästa  f filtrera  / ändra  \
                               Esc rensa",
    keys_dashboard_sessions: "j/k session  Enter öppna  Tab nästa  f filtrera  / sök  \
                              Esc tillbaka",
    keys_dashboard_prompts: "j/k prompt  Enter läs  Tab nästa  f filtrera  / sök  Esc tillbaka",
    keys_dashboard: "j/k välj  Enter detalj  Tab nästa  f filtrera  / sök  y kopiera  d radera",
    keys_detail: "j/k rulla  Enter/t tidslinje  y kopiera  d radera  Esc tillbaka  q avsluta",
    keys_session: "j/k välj  PgDn/End framåt  Enter detalj  y kopiera  d radera  Esc tillbaka",
    keys_timeline: "j/k välj  Enter detalj  Esc tillbaka  / sök  q avsluta",
    keys_setup: "j/k välj  mellanslag märk  Enter fortsätt  Backsteg tillbaka  Esc avbryt",
    keys_options: "j/k välj  Enter ändra  Esc tillbaka",
    keys_cloud: "R läs om  Esc tillbaka  q avsluta",
    keys_help: "Enter/Esc tillbaka  q avsluta",

    help_body: "\
Att röra sig
  j / Ned        flytta valet eller rulla nedåt
  k / Upp        flytta valet eller rulla uppåt
  PgDn / PgUp    en skärm i taget
  End / Home     listans slut, och början
  Tab            översikt: visa nästa lista —
                 iakttagelser, sessioner, prompter
  Enter          öppna valet / tidslinjen
  Esc            avbryt eller gå tillbaka

Att smalna av listorna
  f              filtrera på projekt, där mellanslag märker
                 och avmärker. Utan märken gäller alla projekt.
  /              sök. Den löper medan du skriver, och matchar
                 ordet du skriver på dess början. Båda filtren
                 gäller samtidigt, för alla tre listorna.
  Esc            lämna filtren, släpp sedan sökningen, lämna
                 sedan sidan — i den ordningen.

Vyer
  g / r          översikt, på iakttagelserna
  s              översikt, på sessionerna
  t              tidslinje från detaljen
  S              inställning av agenter
  c              molnreplikering
  ? / h          den här hjälpen

Handlingar
  y              kopiera det valda minnet till urklipp
  d              radera det som står under markören — ett
                 minne, en prompt, en session med allt den
                 antecknade, eller ett helt projekt. Frågar
                 först.
  D              detsamma, för gott. Frågar först.
                 Minnen kommer tillbaka; prompter gör det inte.

Allmänt
  R              läs om översiktens uppgifter
  Ctrl-U         rensa sökningen
  q              avsluta (utanför sökrutan)",
};
