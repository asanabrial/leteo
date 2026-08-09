//! English, the language every other file here is translated from.

use super::Screens;

pub const SCREENS: Screens = Screens {
    found_engram: "Found an Engram installation",
    engram_counts: "{observations} memories, {sessions} sessions, {prompts} prompts, \
                    {relations} relations",
    adopt_question: "Adopt these memories into Leteo?",
    adopt_yes: "Yes, bring them across",
    adopt_no: "No, start empty",
    choose_agents: "Which agents should Leteo configure?",
    will_be_removed: "will be removed",
    will_be_installed: "will be installed",
    hooks_question: "Install the lifecycle hooks that keep memory automatic in {agents}?",
    yes: "Yes",
    hooks_no: "No, MCP tools only",
    voice_question: "How much should {name} say out loud?",
    voice_all: "greeting, hints, captures and reminders",
    voice_reminders: "the save reminder only",
    voice_quiet: "nothing, not even the reminder to save",
    interface_question: "Which language should Leteo speak to you in?",
    interface_hint_first: "  Leteo's own screens: the panels, the menus, the help and this page.",
    interface_hint_second: "  What {name} says and what memories are written in are set on their own.",
    voice_language_question: "Which language should {name} speak in?",
    voice_language_same: "as Leteo",
    voice_language_same_detail: "whatever language Leteo itself is speaking",
    voice_language_hint: "  {name} speaks inside your agent's conversation, not only here.",
    memory_language_question: "Which language should memories be written in?",
    language_auto: "auto",
    language_auto_detail: "the language you write in, whichever it is",
    language_pinned_detail: "always, however you are written to",
    language_kept_warning: "  Memories already saved keep the language they were written in.",
    language_split_warning_first: "  Changing this leaves the store in two languages, and a \
                                   search finds",
    language_split_warning_second: "  the half it is asked in.",
    language_other_hint: "  Any other language: set \"language\" in settings.json.",
    nothing_changed: "  Nothing was changed.",
    legend: "  space select    enter continue    backspace back    esc quit",

    options_question: "What would you like to change?",
    option_interface: "Leteo's language",
    option_voice_language: "{name}'s language",
    option_memory_language: "Memory language",
    option_voice: "{name}'s voice",
    preferences_saved: "Preferences saved",

    could_not_adopt: "  could not adopt: {error}",
    could_not_save: "  could not save preferences: {error}",
    could_not_configure: "  could not configure {agent}: {error}",
    could_not_remove: "  could not remove from {agent}: {error}",
    removed_from: "  removed from {agent}",
    restart_them: "\n  restart them to pick it up",

    empty_dashboard_what_happens: "Memories appear here as your agents save them.",
    empty_dashboard_keys: "Press Esc for the menu, or ? for help.",
    setup_cancelled: "Setup cancelled. Nothing was changed.",
    setup_failed: "setup failed: {error}",

    panel_setup: " Setup ",
    panel_dashboard: " Dashboard ",
    panel_detail: " Detail ",
    panel_content: " Content ",
    panel_session: " Session ",
    panel_timeline: " Timeline ",
    panel_context: " Context ",
    panel_session_timeline: " Session timeline ",
    panel_help: " Help ",
    panel_options: " Options ",
    panel_cloud: " Cloud replication - read only ",
    panel_filters: " FILTERS ",
    panel_filters_count: " FILTERS ({count}) ",
    panel_recorded: " Recorded ({count}) ",
    list_observations: " Observations",
    list_sessions: " Sessions",
    list_prompts: " Prompts",
    scope_one_project: " in {project} ",
    scope_many_projects: " in {count} projects ",
    list_matching: " matching \"{query}\"",
    list_position: " {position} of {total} ",
    search_placeholder: "search memories",

    stat_observations: "OBSERVATIONS",
    stat_sessions: "SESSIONS",
    stat_prompts: "PROMPTS",
    page_home: "HOME",
    page_dashboard: "DASHBOARD",
    page_detail: "DETAIL",
    page_session: "SESSION",
    page_timeline: "TIMELINE",
    page_setup: "SETUP",
    page_cloud: "CLOUD",
    page_help: "HELP",
    page_options: "OPTIONS",

    no_observations: "No observations found",
    no_sessions: "No sessions found",
    no_prompts: "No prompts found",
    no_projects: "No projects yet",
    no_observation_selected: "No observation selected",
    no_session_selected: "No session selected",
    no_timeline_loaded: "No timeline loaded",
    no_summary: "No summary",
    nothing_to_search: "Nothing saved yet — there is nothing to search",
    cancelled: "Cancelled",

    field_type: "Type",
    field_project: "Project",
    field_scope: "Scope",
    field_session: "Session",
    field_topic: "Topic",
    field_started: "Started",
    field_ended: "Ended",
    field_summary: "Summary",
    session_active: "active",
    timeline_session: "Session: {session}",
    timeline_focus: "Focus: #{id} {title} | {total} total observation(s)",
    timeline_focus_marker: "FOCUS",

    cloud_server: "Server:      ",
    cloud_background: "Background:  ",
    cloud_replicating: "Replicating: ",
    cloud_enrolled: "Enrolled:    ",
    cloud_queued: "Queued:      ",
    cloud_deferred: "Deferred:    ",
    cloud_not_configured: "not configured",
    cloud_enabled: "enabled",
    cloud_disabled: "disabled",
    cloud_none: "none",
    cloud_unknown: "unknown",
    cloud_mutations: "{count} mutation(s)",
    cloud_deferred_dead: "{deferred} deferred, {dead} dead",
    cloud_unreadable: "The store could not be read: {reason}",
    cloud_configure_hint: "Configure with: leteo cloud config set --server URL --token TOKEN \
                           --enable",
    cloud_state: "State:       ",
    cloud_failures: "{count} failure(s) in a row",
    cloud_backoff: "waiting until {until}",

    menu_start_setup: "Start setup",
    menu_dashboard: "Dashboard",
    menu_cloud: "Cloud replication",
    menu_options: "Options",
    menu_help: "Help",
    menu_quit: "Quit",
    menu_uninstall: "Uninstall Leteo",
    uninstall_heading: "Remove Leteo from this machine?",
    uninstall_agents: "{count} agent(s) it is configured in",
    uninstall_warning: "Everything above goes. This cannot be undone.",

    delete_memory: "Delete memory #{id}?",
    delete_prompt: "Delete prompt #{id}?",
    delete_session: "Delete session {id}?",
    delete_project: "Delete project {name}?",
    delete_permanent_warning: "This cannot be undone.",
    delete_prompts_warning: "Memories can be recovered. Prompts cannot.",
    delete_recoverable: "This can be recovered from the store.",
    gone_permanently: "permanently deleted",
    gone: "deleted",
    count_memories: "{count} memory(s)",
    count_sessions: "{count} session(s)",
    count_prompts: "{count} prompt(s)",
    copied_to_clipboard: "Copied {count} characters to the clipboard",
    data_refreshed: "Data refreshed",
    deleted_memory: "Memory #{id} {gone}",
    deleted_prompt: "Prompt #{id} deleted",
    deleted_session: "Session {id} {gone}, with {memories} memory(s) and {prompts} prompt(s)",
    // These two carried twenty-three literal spaces in the middle of the
    // sentence — what is left when a `\` continuation is deleted and the
    // indentation it was hiding is not. They are status-line messages, so the
    // gap went on screen every time somebody deleted a project or ran a search.
    deleted_project: "Project {name} {gone}, with {memories} memory(s), \
                      {sessions} session(s) and {prompts} prompt(s)",
    sessions_kept: " — {count} session(s) kept, they hold other projects' rows",
    refreshed_query: "\"{query}\": {observations} observation(s), {sessions} session(s), \
                      {prompts} prompt(s)",

    keys_confirm: "y confirm  n/Esc cancel",
    keys_confirm_footer: "y delete    any other key cancels",
    keys_confirm_window: "y  delete            any other key  cancel",
    keys_home: "j/k navigate  Enter select  / search  ? help  q quit",
    keys_query: "type to search as you go  Ctrl-U clear  Enter/Esc back to the list",
    keys_filters: "j/k project  space toggle  f/Esc done  q quit",
    // Paging is not listed here. It was, and it took these past eighty columns
    // — where the tail is cut off, and the tail is where the keys nobody
    // remembers live. The help page has the full set; the footer is a reminder,
    // and a reminder that does not fit reminds nobody of the last third of it.
    keys_dashboard_searching: "j/k select  Enter open  Tab next  f filter  / edit  \
                               Esc clear the search",
    keys_dashboard_sessions: "j/k session  Enter open  Tab next  f filter  / search  Esc back",
    keys_dashboard_prompts: "j/k prompt  Enter read  Tab next  f filter  / search  Esc back",
    keys_dashboard: "j/k select  Enter detail  Tab next  f filter  / search  y copy  d delete",
    keys_detail: "j/k scroll  Enter/t timeline  y copy  d delete  Esc back  q quit",
    keys_session: "j/k select  PgDn/End move  Enter detail  y copy  d delete  Esc back",
    keys_timeline: "j/k select  Enter detail  Esc back  / search  q quit",
    keys_setup: "j/k select  space tick  Enter continue  Backspace back  Esc cancel",
    keys_options: "j/k select  Enter choose  Esc back",
    keys_cloud: "R refresh  Esc back  q quit",
    keys_help: "Enter/Esc back  q quit",

    help_body: "\
Navigation
  j / Down       move selection or scroll down
  k / Up         move selection or scroll up
  PgDn / PgUp    a screenful at a time
  End / Home     the end of the list, and the start
  Tab            dashboard: show the next list —
                 observations, sessions, prompts
  Enter          open selection / timeline
  Esc            cancel or go back

Narrowing the lists
  f              filter by project, where space marks and
                 unmarks. No marks means every project.
  /              search. It runs as you type, and matches
                 the word being typed by its start. Both
                 narrowings apply at once, to all three lists.
  Esc            leave the filters, then drop the search,
                 then leave the page — in that order.

Views
  g / r          dashboard, on the observations
  s              dashboard, on the sessions
  t              timeline from detail
  S              agent setup
  c              cloud replication
  ? / h          this help

Actions
  y              copy the selected memory to the clipboard
  d              delete what the cursor is on — a memory, a
                 prompt, a session and all it recorded, or a
                 project and everything in it. Asks first.
  D              the same, permanently. Asks first.
                 Memories come back; prompts do not.

General
  R              refresh dashboard data
  Ctrl-U         clear the search input
  q              quit (outside the search input)",
};
