//! French.

use super::Screens;

pub const SCREENS: Screens = Screens {
    found_engram: "Une installation d'Engram a été trouvée",
    engram_counts: "{observations} souvenirs, {sessions} sessions, {prompts} prompts, \
                    {relations} relations",
    adopt_question: "Adopter ces souvenirs dans Leteo ?",
    adopt_yes: "Oui, les reprendre",
    adopt_no: "Non, partir de zéro",
    choose_agents: "Quels agents Leteo doit-il configurer ?",
    will_be_removed: "sera retiré",
    will_be_installed: "sera installé",
    hooks_question: "Installer les hooks de cycle de vie qui rendent la mémoire automatique \
                     dans {agents} ?",
    yes: "Oui",
    hooks_no: "Non, seulement les outils MCP",
    voice_question: "Combien {name} doit-il dire à voix haute ?",
    voice_all: "salutation, suggestions, captures et rappels",
    voice_reminders: "seulement le rappel d'enregistrer",
    voice_quiet: "rien, pas même le rappel d'enregistrer",
    interface_question: "Dans quelle langue Leteo doit-il vous parler ?",
    interface_hint_first: "  Les écrans de Leteo : les panneaux, les menus, l'aide et cette page.",
    interface_hint_second: "  Ce que dit {name} et la langue des mémoires se règlent à part.",
    voice_language_question: "Dans quelle langue doit parler {name} ?",
    voice_language_same: "comme Leteo",
    voice_language_same_detail: "la langue que Leteo parle lui-même",
    voice_language_hint: "  {name} parle dans la conversation de votre agent, pas seulement ici.",
    memory_language_question: "Dans quelle langue les souvenirs doivent-ils être écrits ?",
    language_auto: "auto",
    language_auto_detail: "la langue dans laquelle vous écrivez, quelle qu'elle soit",
    language_pinned_detail: "toujours, quelle que soit celle qu'on vous écrit",
    language_kept_warning: "  Les souvenirs déjà gardés conservent la langue de leur écriture.",
    language_split_warning_first: "  Changer ceci laisse le dépôt en deux langues, et une \
                                   recherche trouve",
    language_split_warning_second: "  la moitié où elle est posée.",
    language_other_hint: "  Toute autre langue : réglez \"language\" dans settings.json.",
    nothing_changed: "  Rien n'a été changé.",
    legend: "  espace choisir   entrée continuer   retour arrière   échap quitter",

    options_question: "Que voulez-vous changer ?",
    option_interface: "Langue de Leteo",
    option_voice_language: "Langue de {name}",
    option_memory_language: "Langue des mémoires",
    option_voice: "Voix de {name}",
    preferences_saved: "Préférences enregistrées",

    could_not_adopt: "  adoption impossible : {error}",
    could_not_save: "  préférences non enregistrées : {error}",
    could_not_configure: "  configuration de {agent} impossible : {error}",
    could_not_remove: "  retrait de {agent} impossible : {error}",
    removed_from: "  retiré de {agent}",
    restart_them: "\n  redémarrez-les pour qu'ils le prennent en compte",

    empty_dashboard_what_happens: "Les souvenirs apparaissent ici à mesure que vos agents les \
                                   enregistrent.",
    empty_dashboard_keys: "Échap pour le menu, ou ? pour l'aide.",
    setup_cancelled: "Configuration annulée. Rien n'a été changé.",
    setup_failed: "échec de la configuration : {error}",

    panel_setup: " Configuration ",
    panel_dashboard: " Tableau de bord ",
    panel_detail: " Détail ",
    panel_content: " Contenu ",
    panel_session: " Session ",
    panel_timeline: " Chronologie ",
    panel_context: " Contexte ",
    panel_session_timeline: " Chronologie de la session ",
    panel_help: " Aide ",
    panel_options: " Options ",
    panel_cloud: " Réplication dans le nuage - lecture seule ",
    panel_filters: " FILTRES ",
    panel_filters_count: " FILTRES ({count}) ",
    panel_recorded: " Enregistré ({count}) ",
    list_observations: " Observations",
    list_sessions: " Sessions",
    list_prompts: " Prompts",
    scope_one_project: " dans {project} ",
    scope_many_projects: " dans {count} projets ",
    list_matching: " correspondant à \"{query}\"",
    list_position: " {position} sur {total} ",
    search_placeholder: "chercher des souvenirs",

    stat_observations: "OBSERVATIONS",
    stat_sessions: "SESSIONS",
    stat_prompts: "PROMPTS",
    page_home: "ACCUEIL",
    page_dashboard: "TABLEAU DE BORD",
    page_detail: "DÉTAIL",
    page_session: "SESSION",
    page_timeline: "CHRONOLOGIE",
    page_setup: "CONFIGURATION",
    page_cloud: "NUAGE",
    page_help: "AIDE",
    page_options: "OPTIONS",

    no_observations: "Aucune observation trouvée",
    no_sessions: "Aucune session trouvée",
    no_prompts: "Aucun prompt trouvé",
    no_projects: "Pas encore de projets",
    no_observation_selected: "Aucune observation sélectionnée",
    no_session_selected: "Aucune session sélectionnée",
    no_timeline_loaded: "Aucune chronologie chargée",
    no_summary: "Pas de résumé",
    nothing_to_search: "Rien d'enregistré pour l'instant — il n'y a rien à chercher",
    cancelled: "Annulé",

    field_type: "Type",
    field_project: "Projet",
    field_scope: "Portée",
    field_session: "Session",
    field_topic: "Sujet",
    field_started: "Début",
    field_ended: "Fin",
    field_summary: "Résumé",
    session_active: "active",
    timeline_session: "Session : {session}",
    timeline_focus: "Point : #{id} {title} | {total} observation(s) au total",
    timeline_focus_marker: "POINT",

    cloud_server: "Serveur :    ",
    cloud_background: "En fond :    ",
    cloud_replicating: "Réplication : ",
    cloud_enrolled: "Inscrits :   ",
    cloud_queued: "En attente : ",
    cloud_deferred: "Différés :   ",
    cloud_not_configured: "non configuré",
    cloud_enabled: "activé",
    cloud_disabled: "désactivé",
    cloud_none: "aucun",
    cloud_unknown: "inconnu",
    cloud_mutations: "{count} mutation(s)",
    cloud_deferred_dead: "{deferred} différées, {dead} mortes",
    cloud_unreadable: "Le dépôt n'a pas pu être lu : {reason}",
    cloud_configure_hint: "Configurez avec : leteo cloud config set --server URL --token TOKEN \
                           --enable",
    cloud_state: "État :       ",
    cloud_failures: "{count} échec(s) d'affilée",
    cloud_backoff: "en attente jusqu'à {until}",

    menu_start_setup: "Lancer la configuration",
    menu_dashboard: "Tableau de bord",
    menu_cloud: "Réplication dans le nuage",
    menu_options: "Options",
    menu_help: "Aide",
    menu_quit: "Quitter",
    menu_uninstall: "Désinstaller Leteo",
    uninstall_heading: "Retirer Leteo de cette machine ?",
    uninstall_agents: "{count} agent(s) où il est configuré",
    uninstall_warning: "Tout ce qui précède disparaît. C'est sans retour.",

    delete_memory: "Supprimer le souvenir #{id} ?",
    delete_prompt: "Supprimer le prompt #{id} ?",
    delete_session: "Supprimer la session {id} ?",
    delete_project: "Supprimer le projet {name} ?",
    delete_permanent_warning: "C'est sans retour.",
    delete_prompts_warning: "Les souvenirs se récupèrent. Les prompts non.",
    delete_recoverable: "Cela peut se récupérer depuis le dépôt.",
    gone_permanently: "supprimé définitivement",
    gone: "supprimé",
    count_memories: "{count} souvenir(s)",
    count_sessions: "{count} session(s)",
    count_prompts: "{count} prompt(s)",
    copied_to_clipboard: "{count} caractères copiés dans le presse-papiers",
    data_refreshed: "Données rechargées",
    deleted_memory: "Souvenir #{id} {gone}",
    deleted_prompt: "Prompt #{id} supprimé",
    deleted_session: "Session {id} {gone}, avec {memories} souvenir(s) et {prompts} prompt(s)",
    deleted_project: "Projet {name} {gone}, avec {memories} souvenir(s), \
                      {sessions} session(s) et {prompts} prompt(s)",
    sessions_kept: " — {count} session(s) gardées, elles tiennent d'autres projets",
    refreshed_query: "\"{query}\" : {observations} observation(s), {sessions} session(s), \
                      {prompts} prompt(s)",

    keys_confirm: "y confirmer  n/Échap annuler",
    keys_confirm_footer: "y supprimer    toute autre touche annule",
    keys_confirm_window: "y  supprimer         toute autre touche  annuler",
    keys_home: "j/k naviguer  Entrée choisir  / chercher  ? aide  q quitter",
    keys_query: "tapez pour chercher  Ctrl-U effacer  Entrée/Échap retour à la liste",
    keys_filters: "j/k projet  espace cocher  f/Échap fini  q quitter",
    keys_dashboard_searching: "j/k choisir  Entrée ouvrir  Tab suivant  f filtrer  / éditer  \
                               Échap effacer",
    keys_dashboard_sessions: "j/k session  Entrée ouvrir  Tab suivant  f filtrer  / chercher  \
                              Échap retour",
    keys_dashboard_prompts: "j/k prompt  Entrée lire  Tab suivant  f filtrer  / chercher  \
                             Échap retour",
    keys_dashboard: "j/k bouger  Entrée détail  Tab suivant  f filtrer  / chercher  \
                     y copier  d jeter",
    keys_detail: "j/k défiler  Entrée/t chronologie  y copier  d supprimer  Échap retour",
    keys_session: "j/k choisir  PgDn/End avancer  Entrée détail  y copier  d supprimer",
    keys_timeline: "j/k choisir  Entrée détail  Échap retour  / chercher  q quitter",
    keys_setup: "j/k choisir  espace cocher  Entrée continuer  Retour arrière  Échap annuler",
    keys_options: "j/k naviguer  Entrée choisir  Échap retour",
    keys_cloud: "R recharger  Échap retour  q quitter",
    keys_help: "Entrée/Échap retour  q quitter",

    help_body: "\
Navigation
  j / Bas        déplacer la sélection ou descendre
  k / Haut       déplacer la sélection ou monter
  PgDn / PgUp    un écran à la fois
  End / Home     la fin de la liste, et le début
  Tab            tableau de bord : la liste suivante —
                 observations, sessions, prompts
  Entrée         ouvrir la sélection / la chronologie
  Échap          annuler ou revenir en arrière

Restreindre les listes
  f              filtrer par projet, où espace coche et
                 décoche. Sans coche, tous les projets.
  /              chercher. Cela court pendant que vous tapez,
                 et le mot en cours est apparié par son début.
                 Les deux filtres s'appliquent en même temps.
  Échap          quitter les filtres, puis lâcher la recherche,
                 puis quitter la page — dans cet ordre.

Vues
  g / r          tableau de bord, sur les observations
  s              tableau de bord, sur les sessions
  t              chronologie depuis le détail
  S              configuration des agents
  c              réplication dans le nuage
  ? / h          cette aide

Actions
  y              copier le souvenir choisi dans le presse-papiers
  d              supprimer ce qui est sous le curseur — un
                 souvenir, un prompt, une session avec tout ce
                 qu'elle a gardé, ou un projet entier. Demande.
  D              pareil, définitivement. Demande d'abord.
                 Les souvenirs reviennent ; les prompts non.

Général
  R              recharger les données du tableau de bord
  Ctrl-U         effacer la recherche
  q              quitter (hors de la recherche)",
};
