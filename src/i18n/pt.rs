//! Portuguese.

use super::Screens;

pub const SCREENS: Screens = Screens {
    found_engram: "Encontrou-se uma instalação do Engram",
    engram_counts: "{observations} memórias, {sessions} sessões, {prompts} prompts, \
                    {relations} relações",
    adopt_question: "Adotar estas memórias no Leteo?",
    adopt_yes: "Sim, trazer todas",
    adopt_no: "Não, começar do zero",
    choose_agents: "Que agentes deve o Leteo configurar?",
    will_be_removed: "será removido",
    will_be_installed: "será instalado",
    hooks_question: "Instalar os hooks de ciclo de vida que tornam a memória automática \
                     em {agents}?",
    yes: "Sim",
    hooks_no: "Não, apenas as ferramentas MCP",
    voice_question: "Quanto deve o {name} dizer em voz alta?",
    voice_all: "saudação, sugestões, capturas e lembretes",
    voice_reminders: "apenas o lembrete para guardar",
    voice_quiet: "nada, nem sequer o lembrete para guardar",
    interface_question: "Em que língua deve o Leteo falar consigo?",
    interface_hint_first: "  Os ecrãs do Leteo: os painéis, os menus, a ajuda e esta página.",
    interface_hint_second: "  O que o {name} diz e em que se escrevem as memórias escolhem-se à parte.",
    voice_language_question: "Em que idioma deve falar o {name}?",
    voice_language_same: "igual ao Leteo",
    voice_language_same_detail: "o idioma em que o Leteo estiver a falar",
    voice_language_hint: "  O {name} fala dentro da conversa do teu agente, não só aqui.",
    memory_language_question: "Em que língua devem ser escritas as memórias?",
    language_auto: "auto",
    language_auto_detail: "a língua em que escreve, seja qual for",
    language_pinned_detail: "sempre, seja em que língua lhe escrevam",
    language_kept_warning: "  As memórias já guardadas mantêm a língua em que foram escritas.",
    language_split_warning_first: "  Mudar isto deixa o arquivo em duas línguas, e uma \
                                   pesquisa encontra",
    language_split_warning_second: "  a metade em que é feita.",
    language_other_hint: "  Qualquer outra língua: defina \"language\" em settings.json.",
    nothing_changed: "  Nada foi alterado.",
    legend: "  espaço escolher   enter continuar   backspace voltar   esc sair",

    options_question: "O que queres mudar?",
    option_interface: "Idioma do Leteo",
    option_voice_language: "Idioma do {name}",
    option_memory_language: "Idioma das memórias",
    option_voice: "Voz do {name}",
    preferences_saved: "Preferências guardadas",

    could_not_adopt: "  não foi possível adotar: {error}",
    could_not_save: "  não foi possível guardar as preferências: {error}",
    could_not_configure: "  não foi possível configurar {agent}: {error}",
    could_not_remove: "  não foi possível remover de {agent}: {error}",
    removed_from: "  removido de {agent}",
    restart_them: "\n  reinicie-os para que peguem a alteração",

    empty_dashboard_what_happens: "As memórias aparecem aqui à medida que os agentes as guardam.",
    empty_dashboard_keys: "Prima Esc para o menu, ou ? para a ajuda.",
    setup_cancelled: "Configuração cancelada. Nada foi alterado.",
    setup_failed: "a configuração falhou: {error}",

    panel_setup: " Configuração ",
    panel_dashboard: " Painel ",
    panel_detail: " Detalhe ",
    panel_content: " Conteúdo ",
    panel_session: " Sessão ",
    panel_timeline: " Cronologia ",
    panel_context: " Contexto ",
    panel_session_timeline: " Cronologia da sessão ",
    panel_help: " Ajuda ",
    panel_options: " Opções ",
    panel_cloud: " Replicação na nuvem - só leitura ",
    panel_filters: " FILTROS ",
    panel_filters_count: " FILTROS ({count}) ",
    panel_recorded: " Registado ({count}) ",
    list_observations: " Observações",
    list_sessions: " Sessões",
    list_prompts: " Prompts",
    scope_one_project: " em {project} ",
    scope_many_projects: " em {count} projetos ",
    list_matching: " que correspondem a \"{query}\"",
    list_position: " {position} de {total} ",
    search_placeholder: "pesquisar memórias",

    stat_observations: "OBSERVAÇÕES",
    stat_sessions: "SESSÕES",
    stat_prompts: "PROMPTS",
    page_home: "INÍCIO",
    page_dashboard: "PAINEL",
    page_detail: "DETALHE",
    page_session: "SESSÃO",
    page_timeline: "CRONOLOGIA",
    page_setup: "CONFIGURAÇÃO",
    page_cloud: "NUVEM",
    page_help: "AJUDA",
    page_options: "OPÇÕES",

    no_observations: "Não foram encontradas observações",
    no_sessions: "Não foram encontradas sessões",
    no_prompts: "Não foram encontrados prompts",
    no_projects: "Ainda não há projetos",
    no_observation_selected: "Nenhuma observação selecionada",
    no_session_selected: "Nenhuma sessão selecionada",
    no_timeline_loaded: "Nenhuma cronologia carregada",
    no_summary: "Sem resumo",
    nothing_to_search: "Ainda não há nada guardado — não há nada para pesquisar",
    cancelled: "Cancelado",

    field_type: "Tipo",
    field_project: "Projeto",
    field_scope: "Âmbito",
    field_session: "Sessão",
    field_topic: "Tema",
    field_started: "Início",
    field_ended: "Fim",
    field_summary: "Resumo",
    session_active: "ativa",
    timeline_session: "Sessão: {session}",
    timeline_focus: "Foco: #{id} {title} | {total} observação(ões) no total",
    timeline_focus_marker: "FOCO",

    cloud_server: "Servidor:    ",
    cloud_background: "Em segundo plano: ",
    cloud_replicating: "A replicar: ",
    cloud_enrolled: "Inscritos:   ",
    cloud_queued: "Na fila:     ",
    cloud_deferred: "Adiados:     ",
    cloud_not_configured: "não configurado",
    cloud_enabled: "ativado",
    cloud_disabled: "desativado",
    cloud_none: "nenhum",
    cloud_unknown: "desconhecido",
    cloud_mutations: "{count} mutação(ões)",
    cloud_deferred_dead: "{deferred} adiadas, {dead} mortas",
    cloud_unreadable: "Não foi possível ler o arquivo: {reason}",
    cloud_configure_hint: "Configure com: leteo cloud config set --server URL --token TOKEN \
                           --enable",
    cloud_state: "Estado:      ",
    cloud_failures: "{count} falha(s) seguidas",
    cloud_backoff: "à espera até {until}",

    menu_start_setup: "Iniciar configuração",
    menu_dashboard: "Painel",
    menu_cloud: "Replicação na nuvem",
    menu_options: "Opções",
    menu_help: "Ajuda",
    menu_quit: "Sair",
    menu_uninstall: "Desinstalar o Leteo",
    uninstall_heading: "Remover o Leteo desta máquina?",
    uninstall_agents: "{count} agente(s) em que está configurado",
    uninstall_warning: "Tudo o que está acima desaparece. Não há como desfazer.",

    delete_memory: "Apagar a memória #{id}?",
    delete_prompt: "Apagar o prompt #{id}?",
    delete_session: "Apagar a sessão {id}?",
    delete_project: "Apagar o projeto {name}?",
    delete_permanent_warning: "Não há como desfazer.",
    delete_prompts_warning: "As memórias podem recuperar-se. Os prompts não.",
    delete_recoverable: "Isto pode recuperar-se do arquivo.",
    gone_permanently: "apagada para sempre",
    gone: "apagada",
    count_memories: "{count} memória(s)",
    count_sessions: "{count} sessão(ões)",
    count_prompts: "{count} prompt(s)",
    copied_to_clipboard: "Copiados {count} caracteres para a área de transferência",
    data_refreshed: "Dados recarregados",
    deleted_memory: "Memória #{id} {gone}",
    deleted_prompt: "Prompt #{id} apagado",
    deleted_session: "Sessão {id} {gone}, com {memories} memória(s) e {prompts} prompt(s)",
    deleted_project: "Projeto {name} {gone}, com {memories} memória(s), \
                      {sessions} sessão(ões) e {prompts} prompt(s)",
    sessions_kept: " — {count} sessão(ões) mantidas, têm linhas de outros projetos",
    refreshed_query: "\"{query}\": {observations} observação(ões), {sessions} sessão(ões), \
                      {prompts} prompt(s)",

    keys_confirm: "y confirmar  n/Esc cancelar",
    keys_confirm_footer: "y apagar    qualquer outra tecla cancela",
    keys_confirm_window: "y  apagar            qualquer outra tecla  cancelar",
    keys_home: "j/k mover  Enter escolher  / pesquisar  ? ajuda  q sair",
    keys_query: "escreva para pesquisar  Ctrl-U limpar  Enter/Esc voltar à lista",
    keys_filters: "j/k projeto  espaço marcar  f/Esc pronto  q sair",
    keys_dashboard_searching: "j/k escolher  Enter abrir  Tab seguinte  f filtrar  / editar  \
                               Esc limpar",
    keys_dashboard_sessions: "j/k sessão  Enter abrir  Tab seguinte  f filtrar  / pesquisar  \
                              Esc voltar",
    keys_dashboard_prompts: "j/k prompt  Enter ler  Tab seguinte  f filtrar  / pesquisar  \
                             Esc voltar",
    keys_dashboard: "j/k mover  Enter detalhe  Tab próxima  f filtrar  / buscar  \
                     y copiar  d apagar",
    keys_detail: "j/k deslocar  Enter/t cronologia  y copiar  d apagar  Esc voltar  q sair",
    keys_session: "j/k escolher  PgDn/End mover  Enter detalhe  y copiar  d apagar  Esc voltar",
    keys_timeline: "j/k escolher  Enter detalhe  Esc voltar  / pesquisar  q sair",
    keys_setup: "j/k escolher  espaço marcar  Enter continuar  Backspace voltar  Esc cancelar",
    keys_options: "j/k mover  Enter escolher  Esc voltar",
    keys_cloud: "R recarregar  Esc voltar  q sair",
    keys_help: "Enter/Esc voltar  q sair",

    help_body: "\
Navegação
  j / Baixo      mover a seleção ou descer
  k / Cima       mover a seleção ou subir
  PgDn / PgUp    um ecrã de cada vez
  End / Home     o fim da lista, e o princípio
  Tab            painel: mostrar a lista seguinte —
                 observações, sessões, prompts
  Enter          abrir a seleção / a cronologia
  Esc            cancelar ou voltar atrás

Estreitar as listas
  f              filtrar por projeto, onde o espaço marca e
                 desmarca. Sem marcas, todos os projetos.
  /              pesquisar. Corre enquanto escreve, e casa a
                 palavra que escreve pelo princípio. Os dois
                 filtros aplicam-se ao mesmo tempo.
  Esc            sair dos filtros, depois largar a pesquisa,
                 depois sair da página — por essa ordem.

Vistas
  g / r          painel, sobre as observações
  s              painel, sobre as sessões
  t              cronologia a partir do detalhe
  S              configuração dos agentes
  c              replicação na nuvem
  ? / h          esta ajuda

Ações
  y              copiar a memória escolhida para a área de
                 transferência
  d              apagar o que está sob o cursor — uma memória,
                 um prompt, uma sessão com tudo o que registou,
                 ou um projeto inteiro. Pergunta primeiro.
  D              o mesmo, para sempre. Pergunta primeiro.
                 As memórias voltam; os prompts não.

Geral
  R              recarregar os dados do painel
  Ctrl-U         limpar a pesquisa
  q              sair (fora da pesquisa)",
};
