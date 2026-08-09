//! Basque.

use super::Screens;

pub const SCREENS: Screens = Screens {
    found_engram: "Engram instalazio bat aurkitu da",
    engram_counts: "{observations} memoria, {sessions} saio, {prompts} prompt, \
                    {relations} erlazio",
    adopt_question: "Memoria hauek Leteora ekarri?",
    adopt_yes: "Bai, ekarri denak",
    adopt_no: "Ez, hutsetik hasi",
    choose_agents: "Zein agente konfiguratu behar du Leteok?",
    will_be_removed: "kenduko da",
    will_be_installed: "instalatuko da",
    hooks_question: "Memoria automatiko egiten duten bizi-zikloko hook-ak instalatu \
                     {agents} agentean?",
    yes: "Bai",
    hooks_no: "Ez, MCP tresnak bakarrik",
    voice_question: "Zenbat esan behar du {name}k ozen?",
    voice_all: "agurra, iradokizunak, harrapaketak eta oroigarriak",
    voice_reminders: "gordetzeko oroigarria bakarrik",
    voice_quiet: "ezer ez, ezta gordetzeko oroigarria ere",
    interface_question: "Zein hizkuntzatan hitz egin behar dizu Leteok?",
    interface_hint_first: "  Leteoren pantailak: panelak, menuak, laguntza eta orri hau.",
    interface_hint_second: "  {name}k dioena eta oroitzapenen hizkuntza aparte hautatzen dira.",
    voice_language_question: "Zein hizkuntzatan hitz egin behar du {name}k?",
    voice_language_same: "Leteo bezala",
    voice_language_same_detail: "Leteok berak hitz egiten duen hizkuntza",
    voice_language_hint: "  {name}k zure agentearen elkarrizketan ere hitz egiten du, ez hemen soilik.",
    memory_language_question: "Zein hizkuntzatan idatzi behar dira memoriak?",
    language_auto: "auto",
    language_auto_detail: "zuk idazten duzun hizkuntza, edozein dela ere",
    language_pinned_detail: "beti, zein hizkuntzatan idazten dizuten kontuan hartu gabe",
    language_kept_warning: "  Gordeta dauden memoriek idatzi ziren hizkuntza gordetzen dute.",
    language_split_warning_first: "  Hau aldatzeak biltegia bi hizkuntzatan uzten du, eta \
                                   bilaketa batek",
    language_split_warning_second: "  galdetzen den erdia aurkitzen du.",
    language_other_hint: "  Beste edozein hizkuntza: jarri \"language\" settings.json fitxategian.",
    nothing_changed: "  Ez da ezer aldatu.",
    legend: "  zuriunea hautatu   sartu jarraitu   atzera itzuli   esc irten",

    options_question: "Zer aldatu nahi duzu?",
    option_interface: "Leteoren hizkuntza",
    option_voice_language: "{name}ren hizkuntza",
    option_memory_language: "Oroitzapenen hizkuntza",
    option_voice: "{name}ren ahotsa",
    preferences_saved: "Hobespenak gordeta",

    could_not_adopt: "  ezin izan da hartu: {error}",
    could_not_save: "  ezin izan dira hobespenak gorde: {error}",
    could_not_configure: "  ezin izan da {agent} konfiguratu: {error}",
    // Through the common noun rather than off the agent's own name: the case
    // ending Basque wants depends on what the word ends in, and an agent name
    // arrives at runtime. `{agent} agentetik` declines a word that is always
    // the same one, which is what the two lines above already do.
    could_not_remove: "  ezin izan da {agent} agentetik kendu: {error}",
    removed_from: "  {agent} agentetik kendua",
    restart_them: "\n  berrabiarazi itzazu aldaketa hartzeko",

    empty_dashboard_what_happens: "Memoriak hemen agertzen dira agenteek gordetzen dituzten heinean.",
    empty_dashboard_keys: "Sakatu Esc menurako, edo ? laguntzarako.",
    setup_cancelled: "Konfigurazioa bertan behera. Ez da ezer aldatu.",
    setup_failed: "konfigurazioak huts egin du: {error}",

    panel_setup: " Konfigurazioa ",
    panel_dashboard: " Panela ",
    panel_detail: " Xehetasuna ",
    panel_content: " Edukia ",
    panel_session: " Saioa ",
    panel_timeline: " Kronologia ",
    panel_context: " Testuingurua ",
    panel_session_timeline: " Saioaren kronologia ",
    panel_help: " Laguntza ",
    panel_options: " Aukerak ",
    panel_cloud: " Hodeiko replikazioa - irakurtzeko soilik ",
    panel_filters: " IRAGAZKIAK ",
    panel_filters_count: " IRAGAZKIAK ({count}) ",
    panel_recorded: " Erregistratua ({count}) ",
    list_observations: " Behaketak",
    list_sessions: " Saioak",
    list_prompts: " Promptak",
    scope_one_project: " {project} proiektuan ",
    scope_many_projects: " {count} proiektutan ",
    list_matching: " \"{query}\" bilaketarekin bat",
    list_position: " {position} / {total} ",
    search_placeholder: "memoriak bilatu",

    stat_observations: "BEHAKETAK",
    stat_sessions: "SAIOAK",
    stat_prompts: "PROMPTAK",
    page_home: "HASIERA",
    page_dashboard: "PANELA",
    page_detail: "XEHETASUNA",
    page_session: "SAIOA",
    page_timeline: "KRONOLOGIA",
    page_setup: "KONFIGURAZIOA",
    page_cloud: "HODEIA",
    page_help: "LAGUNTZA",
    page_options: "AUKERAK",

    no_observations: "Ez da behaketarik aurkitu",
    no_sessions: "Ez da saiorik aurkitu",
    no_prompts: "Ez da promptik aurkitu",
    no_projects: "Oraindik ez dago proiekturik",
    no_observation_selected: "Ez da behaketarik hautatu",
    no_session_selected: "Ez da saiorik hautatu",
    no_timeline_loaded: "Ez da kronologiarik kargatu",
    no_summary: "Laburpenik ez",
    nothing_to_search: "Oraindik ez dago ezer gordeta — ez dago zer bilaturik",
    cancelled: "Bertan behera utzia",

    field_type: "Mota",
    field_project: "Proiektua",
    field_scope: "Esparrua",
    field_session: "Saioa",
    field_topic: "Gaia",
    field_started: "Hasiera",
    field_ended: "Amaiera",
    field_summary: "Laburpena",
    session_active: "aktiboa",
    timeline_session: "Saioa: {session}",
    timeline_focus: "Fokua: #{id} {title} | {total} behaketa guztira",
    timeline_focus_marker: "FOKUA",

    cloud_server: "Zerbitzaria: ",
    cloud_background: "Bigarren planoan: ",
    cloud_replicating: "Replikatzen: ",
    cloud_enrolled: "Izena emanda: ",
    cloud_queued: "Ilaran:      ",
    cloud_deferred: "Atzeratuak:  ",
    cloud_not_configured: "konfiguratu gabe",
    cloud_enabled: "gaituta",
    cloud_disabled: "desgaituta",
    cloud_none: "bat ere ez",
    cloud_unknown: "ezezaguna",
    cloud_mutations: "{count} aldaketa",
    cloud_deferred_dead: "{deferred} atzeratuta, {dead} hilda",
    cloud_unreadable: "Ezin izan da biltegia irakurri: {reason}",
    cloud_configure_hint: "Konfiguratu honela: leteo cloud config set --server URL \
                           --token TOKEN --enable",
    cloud_state: "Egoera:      ",
    cloud_failures: "{count} hutsegite jarraian",
    cloud_backoff: "{until} arte itxaroten",

    menu_start_setup: "Hasi konfigurazioa",
    menu_dashboard: "Panela",
    menu_cloud: "Hodeiko replikazioa",
    menu_options: "Aukerak",
    menu_help: "Laguntza",
    menu_quit: "Irten",
    menu_uninstall: "Leteo desinstalatu",
    uninstall_heading: "Leteo makina honetatik kendu?",
    uninstall_agents: "{count} agentetan dago konfiguratuta",
    uninstall_warning: "Goiko guztia joango da. Hau ezin da desegin.",

    delete_memory: "#{id} memoria ezabatu?",
    delete_prompt: "#{id} prompta ezabatu?",
    delete_session: "{id} saioa ezabatu?",
    delete_project: "{name} proiektua ezabatu?",
    delete_permanent_warning: "Hau ezin da desegin.",
    delete_prompts_warning: "Memoriak berreskura daitezke. Promptak ez.",
    delete_recoverable: "Hau biltegitik berreskura daiteke.",
    gone_permanently: "betiko ezabatua",
    gone: "ezabatua",
    count_memories: "{count} memoria",
    count_sessions: "{count} saio",
    count_prompts: "{count} prompt",
    copied_to_clipboard: "{count} karaktere arbelera kopiatu dira",
    data_refreshed: "Datuak birkargatu dira",
    deleted_memory: "#{id} memoria {gone}",
    deleted_prompt: "#{id} prompta ezabatua",
    deleted_session: "{id} saioa {gone}, {memories} memoria eta {prompts} promptekin",
    deleted_project: "{name} proiektua {gone}, {memories} memoria, {sessions} saio eta \
                      {prompts} promptekin",
    sessions_kept: " — {count} saio mantendu dira, beste proiektuen lerroak dituzte",
    refreshed_query: "\"{query}\": {observations} behaketa, {sessions} saio, {prompts} prompt",

    keys_confirm: "y berretsi  n/Esc utzi",
    keys_confirm_footer: "y ezabatu    beste edozein teklak uzten du",
    keys_confirm_window: "y  ezabatu           beste edozein tekla  utzi",
    keys_home: "j/k mugitu  Sartu hautatu  / bilatu  ? laguntza  q irten",
    keys_query: "idatzi bilatzeko  Ctrl-U garbitu  Sartu/Esc zerrendara itzuli",
    keys_filters: "j/k proiektua  zuriunea markatu  f/Esc egina  q irten",
    keys_dashboard_searching: "j/k hautatu  Sartu ireki  Tab hurrengoa  f iragazi  / aldatu  \
                               Esc garbitu",
    keys_dashboard_sessions: "j/k saioa  Sartu ireki  Tab hurrengoa  f iragazi  / bilatu  \
                              Esc atzera",
    keys_dashboard_prompts: "j/k prompta  Sartu irakurri  Tab hurrengoa  f iragazi  / bilatu  \
                             Esc atzera",
    keys_dashboard: "j/k mugitu  Sartu xehea  Tab hurrengo  f iragazi  / bilatu  \
                     y kopia  d ezabatu",
    keys_detail: "j/k korritu  Sartu/t kronologia  y kopiatu  d ezabatu  Esc atzera  q irten",
    keys_session: "j/k hautatu  PgDn/End aurrera  Sartu xehetasuna  y kopiatu  d ezabatu",
    keys_timeline: "j/k hautatu  Sartu xehetasuna  Esc atzera  / bilatu  q irten",
    keys_setup: "j/k hautatu  zuriunea markatu  Sartu jarraitu  Atzera  Esc utzi",
    keys_options: "j/k mugitu  Sartu aukeratu  Esc atzera",
    keys_cloud: "R birkargatu  Esc atzera  q irten",
    keys_help: "Sartu/Esc atzera  q irten",

    help_body: "\
Nabigazioa
  j / Behera     hautapena mugitu edo behera korritu
  k / Gora       hautapena mugitu edo gora korritu
  PgDn / PgUp    pantaila bat aldi bakoitzean
  End / Home     zerrendaren amaiera, eta hasiera
  Tab            panela: hurrengo zerrenda erakutsi —
                 behaketak, saioak, promptak
  Sartu          hautapena ireki / kronologia
  Esc            utzi edo atzera egin

Zerrendak estutzea
  f              proiektuka iragazi; zuriuneak markatu eta
                 desmarkatzen du. Markarik gabe, proiektu guztiak.
  /              bilatu. Idatzi ahala doa, eta idazten ari zaren
                 hitza bere hasieratik parekatzen du. Bi
                 iragazkiak batera aplikatzen dira.
  Esc            iragazkietatik irten, gero bilaketa utzi, gero
                 orritik irten — hurrenkera horretan.

Ikuspegiak
  g / r          panela, behaketen gainean
  s              panela, saioen gainean
  t              kronologia xehetasunetik
  S              agenteen konfigurazioa
  c              hodeiko replikazioa
  ? / h          laguntza hau

Ekintzak
  y              hautatutako memoria arbelera kopiatu
  d              kurtsorearen azpian dagoena ezabatu — memoria
                 bat, prompt bat, saio bat erregistratu duen
                 guztiarekin, edo proiektu oso bat. Galdetu
                 egiten du lehenik.
  D              berdina, betiko. Galdetu egiten du lehenik.
                 Memoriak itzultzen dira; promptak ez.

Orokorra
  R              panelaren datuak birkargatu
  Ctrl-U         bilaketa garbitu
  q              irten (bilaketatik kanpo)",
};
