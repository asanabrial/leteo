use std::path::{Path, PathBuf};

use tempfile::TempDir;

use super::*;
use crate::settings::{self, Settings, Voice};

fn probe_in(directory: &Path) -> SetupOptions {
    SetupOptions {
        home_dir: Some(directory.to_path_buf()),
        config_home: Some(directory.join("config")),
        app_data: Some(directory.join("appdata")),
        ..SetupOptions::default()
    }
}

fn offer_with(engram: Option<engram::Installation>) -> Offer {
    Offer {
        engram,
        database: fixture_data_dir().join("leteo.db"),
        store_has_memories: false,
        probe: probe_in(fixture_data_dir()),
        agents: vec![
            choice("claude-code", "Claude Code", true),
            choice("opencode", "OpenCode", false),
            choice("codex", "Codex", false),
        ],
    }
}

fn fixture_data_dir() -> &'static Path {
    static DIRECTORY: std::sync::OnceLock<TempDir> = std::sync::OnceLock::new();
    DIRECTORY
        .get_or_init(|| {
            let directory = TempDir::new().expect("a temporary directory");
            settings::save(
                directory.path(),
                &Settings {
                    interface: Some(settings::Interface::English),
                    voice_language: None,
                    ..Settings::default()
                },
            )
            .expect("pin the fixture's language");
            directory
        })
        .path()
}

fn found() -> engram::Installation {
    engram::Installation {
        database: PathBuf::from("/home/someone/.engram/engram.db"),
        binary: None,
        sessions: 12,
        observations: 3223,
        prompts: 58,
        relations: 7,
    }
}

fn open_option(wizard: &mut Wizard, setting: Step) {
    assert_eq!(wizard.step(), Step::Options, "not on the options page");
    let at = OPTIONS
        .iter()
        .position(|row| *row == setting)
        .expect("a row of the index");
    for _ in 0..at {
        wizard.down();
    }
    wizard.advance();
    assert_eq!(wizard.step(), setting);
}

fn row_with(wizard: &Wizard, needle: &str) -> String {
    wizard
        .render()
        .into_iter()
        .map(|row| row.text)
        .find(|text| text.contains(needle))
        .unwrap_or_default()
}

fn role_of(wizard: &Wizard, needle: &str) -> Option<Role> {
    wizard
        .render()
        .into_iter()
        .find(|row| row.text.contains(needle))
        .map(|row| row.role)
}

fn screen(wizard: &Wizard) -> String {
    wizard
        .render()
        .into_iter()
        .map(|row| row.text)
        .collect::<Vec<_>>()
        .join("\n")
}

fn choice(slug: &str, display_name: &str, supports_hooks: bool) -> AgentChoice {
    AgentChoice {
        slug: slug.to_owned(),
        display_name: display_name.to_owned(),
        supports_hooks,
        configured: false,
    }
}

#[test]
fn a_configured_agent_arrives_ticked_and_unticking_removes_it() {
    let mut offer = offer_with(None);
    offer.agents[0].configured = true;
    let wizard = Wizard::new(offer);
    assert_eq!(wizard.chosen, vec![0], "opens ticked where Leteo is");

    let painted = screen(&wizard);
    assert!(painted.contains("[\u{2713}] Claude Code"), "{painted}");
    assert!(painted.contains("[ ] OpenCode"), "{painted}");
    assert!(!painted.contains("will be"), "{painted}");
}

#[test]
fn the_row_says_what_the_tick_is_about_to_do() {
    let mut offer = offer_with(None);
    offer.agents[0].configured = true;
    let mut wizard = Wizard::new(offer);

    wizard.toggle();
    wizard.down();
    wizard.toggle();
    let painted = screen(&wizard);

    assert!(
        painted.contains("Claude Code — will be removed"),
        "{painted}"
    );
    assert!(
        painted.contains("OpenCode — will be installed"),
        "{painted}"
    );
    assert!(!painted.contains("Codex — will"), "untouched: {painted}");
}

#[test]
fn agents_are_checkboxes_and_space_ticks_them() {
    let mut wizard = Wizard::new(offer_with(None));
    assert_eq!(wizard.step(), Step::ChooseAgents);
    assert!(screen(&wizard).contains("▸ [ ] Claude Code"));

    wizard.toggle();
    assert!(screen(&wizard).contains("▸ [✓] Claude Code"));

    wizard.down();
    wizard.down();
    wizard.toggle();
    let painted = screen(&wizard);
    assert!(painted.contains("  [✓] Claude Code"), "{painted}");
    assert!(painted.contains("▸ [✓] Codex"), "{painted}");
    assert!(painted.contains("  [ ] OpenCode"), "{painted}");

    wizard.toggle();
    assert!(screen(&wizard).contains("▸ [ ] Codex"));
}

#[test]
fn a_single_choice_is_a_radio_where_picking_one_drops_the_other() {
    let mut wizard = Wizard::new(offer_with(Some(found())));
    assert_eq!(wizard.step(), Step::AdoptEngram);
    let painted = screen(&wizard);
    assert!(
        painted.contains("▸ (●) Yes, bring them across"),
        "{painted}"
    );
    assert!(painted.contains("  ( ) No, start empty"), "{painted}");

    wizard.down();
    wizard.toggle();
    let painted = screen(&wizard);
    assert!(
        painted.contains("  ( ) Yes, bring them across"),
        "{painted}"
    );
    assert!(painted.contains("▸ (●) No, start empty"), "{painted}");
}

#[test]
fn enter_goes_forward_and_backspace_returns_with_the_answer_intact() {
    let mut wizard = Wizard::new(offer_with(Some(found())));
    wizard.down();
    wizard.toggle();
    wizard.advance();
    assert_eq!(wizard.step(), Step::ChooseAgents);

    wizard.toggle();
    wizard.advance();
    assert_eq!(wizard.step(), Step::InstallHooks);
    wizard.back();
    assert_eq!(wizard.step(), Step::ChooseAgents);
    assert!(
        screen(&wizard).contains("[✓] Claude Code"),
        "the choice must survive going back"
    );

    wizard.back();
    assert_eq!(wizard.step(), Step::AdoptEngram);
    assert!(screen(&wizard).contains("▸ (●) No, start empty"));
}

#[test]
fn choosing_nothing_skips_the_hooks_question() {
    let mut wizard = Wizard::new(offer_with(None));
    wizard.advance();
    assert_eq!(wizard.step(), Step::Ready);
}

#[test]
fn backing_out_of_the_first_question_cancels() {
    let mut wizard = Wizard::new(offer_with(Some(found())));
    wizard.back();
    assert_eq!(wizard.step(), Step::Cancelled);

    let mut wizard = Wizard::new(offer_with(None));
    wizard.back();
    assert_eq!(wizard.step(), Step::Cancelled);
}

#[test]
fn the_cursor_wraps_so_a_long_list_is_reachable_from_either_end() {
    let mut wizard = Wizard::new(offer_with(None));
    wizard.up();
    assert!(
        screen(&wizard).contains("▸ [ ] Codex"),
        "up from the top wraps"
    );
    wizard.down();
    assert!(screen(&wizard).contains("▸ [ ] Claude Code"));
}

#[test]
fn each_line_carries_what_it_is_for() {
    let mut wizard = Wizard::new(offer_with(Some(found())));
    assert_eq!(role_of(&wizard, BRAND), Some(Role::Brand));
    assert_eq!(
        role_of(&wizard, "Adopt these memories"),
        Some(Role::Heading)
    );
    assert_eq!(role_of(&wizard, ".engram"), Some(Role::Detail));
    assert_eq!(role_of(&wizard, "space select"), Some(Role::Hint));
    assert_eq!(
        role_of(&wizard, "Yes, bring them across"),
        Some(Role::Focused)
    );
    assert_eq!(role_of(&wizard, "No, start empty"), Some(Role::Choice));

    wizard.down();
    assert_eq!(
        role_of(&wizard, "Yes, bring them across"),
        Some(Role::Choice)
    );
    assert_eq!(role_of(&wizard, "No, start empty"), Some(Role::Focused));
}

#[test]
fn a_transition_that_changes_nothing_says_so() {
    let mut wizard = Wizard::new(offer_with(Some(found())));

    assert!(wizard.down(), "moving between the two options redraws");
    assert!(wizard.toggle(), "picking the other option redraws");
    assert!(
        !wizard.toggle(),
        "picking the option already picked changes nothing"
    );

    assert!(wizard.cancel(), "cancelling leaves the question behind");
    assert!(!wizard.cancel(), "cancelling twice changes nothing");
    assert!(!wizard.advance(), "a finished wizard has nowhere to go");
    assert!(!wizard.back(), "and nowhere to return to");
    assert!(!wizard.up(), "a screen with no options has no cursor");
    assert!(!wizard.toggle(), "and nothing to tick");
}

#[test]
fn a_store_that_is_not_empty_still_hears_about_engram() {
    let note = adoption_note(&found(), 3100);
    assert!(note.contains("3223 memories"), "{note}");
    assert!(note.contains("Leteo already holds 3100"), "{note}");
    assert!(
        note.contains("replaces the Leteo database"),
        "the reason it is not offered has to be in the note: {note}"
    );
    assert!(
        note.contains("leteo import --from-engram --dry-run"),
        "a note with no way forward is just an apology: {note}"
    );
}

#[test]
fn hooks_are_only_asked_about_when_a_chosen_agent_can_take_them() {
    let mut wizard = Wizard::new(offer_with(None));
    wizard.down();
    wizard.toggle();
    wizard.advance();
    assert_eq!(
        wizard.step(),
        Step::Ready,
        "no chosen agent takes hooks, so that question is skipped"
    );

    let mut wizard = Wizard::new(offer_with(None));
    wizard.toggle();
    wizard.down();
    wizard.toggle();
    wizard.advance();
    let painted = screen(&wizard);
    assert!(painted.contains("Install the lifecycle hooks"), "{painted}");
    assert!(
        painted.contains("in Claude Code?"),
        "the question has to name who it applies to: {painted}"
    );
    assert!(
        !painted.contains("OpenCode?"),
        "OpenCode cannot take hooks and must not be named: {painted}"
    );
}

#[test]
fn a_cancelled_wizard_applies_nothing() {
    let mut wizard = Wizard::new(offer_with(Some(found())));
    wizard.cancel();
    let mut report = Vec::new();
    let outcome = wizard.apply(&mut report).unwrap();
    assert!(outcome.cancelled);
    assert!(outcome.configured.is_empty());
    assert_eq!(outcome.adopted, None);
}

fn wizard_in(directory: &Path) -> Wizard {
    Wizard::new(offer_in(directory))
}

fn offer_in(directory: &Path) -> Offer {
    if !settings::path_in(directory).exists() {
        settings::save(
            directory,
            &Settings {
                interface: Some(settings::Interface::English),
                voice_language: None,
                ..Settings::default()
            },
        )
        .expect("pin the language");
    }
    let mut offer = offer_with(None);
    offer.database = directory.join("leteo.db");
    offer.probe = probe_in(directory);
    offer.agents[0].configured = true;
    offer
}

#[test]
fn the_options_flow_leaves_every_agent_alone() {
    let temp = TempDir::new().unwrap();
    let mut wizard = Wizard::preferences(offer_in(temp.path()));

    assert_eq!(wizard.step(), Step::Options);
    let mut backed_out = Wizard::preferences(offer_in(temp.path()));
    backed_out.back();
    assert_eq!(backed_out.step(), Step::Ready);

    open_option(&mut wizard, Step::InterfaceLanguage);
    while wizard.interface() != settings::Interface::Spanish {
        wizard.down();
        wizard.toggle();
    }
    wizard.advance();
    assert_eq!(wizard.step(), Step::Options);
    wizard.back();
    assert_eq!(wizard.step(), Step::Ready);

    let mut report = Vec::new();
    let outcome = wizard.apply(&mut report).unwrap();
    assert!(
        outcome.configured.is_empty() && outcome.removed.is_empty(),
        "the options flow touched an agent: {outcome:?}"
    );
    assert_eq!(outcome.adopted, None);
    assert_eq!(
        settings::load_beside(&temp.path().join("leteo.db")).interface,
        Some(settings::Interface::Spanish),
        "the answer it did come to give was not saved"
    );
}

#[test]
fn the_options_page_shows_the_three_settings_side_by_side_and_opens_the_one_chosen() {
    let temp = TempDir::new().unwrap();
    let mut wizard = Wizard::preferences(offer_in(temp.path()));

    assert_eq!(wizard.step(), Step::Options, "it opens on the list");
    let painted = screen(&wizard);
    assert!(
        painted.contains("What would you like to change?"),
        "{painted}"
    );
    assert!(
        row_with(&wizard, "Leteo's language").contains("English"),
        "{painted}"
    );
    assert!(
        row_with(&wizard, "Sardi's language").contains("as Leteo"),
        "{painted}"
    );
    assert!(
        row_with(&wizard, "Memory language").contains("auto"),
        "{painted}"
    );
    assert!(
        row_with(&wizard, "Sardi's voice").contains("all"),
        "{painted}"
    );
    assert!(!painted.contains("say out loud"), "{painted}");
    assert!(!painted.contains("Which language"), "{painted}");

    open_option(&mut wizard, Step::SardiVoice);
    let painted = screen(&wizard);
    assert!(painted.contains("say out loud"), "{painted}");
    assert!(
        painted.contains("▸ (●) all"),
        "it opens on the answer in force: {painted}"
    );
    for voice in Voice::ALL {
        assert!(
            painted.contains(voice.description(settings::Interface::English)),
            "{painted}"
        );
    }

    wizard.down();
    wizard.advance();
    assert_eq!(wizard.step(), Step::Options);
    let row = row_with(&wizard, "Sardi's voice");
    assert!(row.contains("reminders"), "{row}");
    assert!(
        row.starts_with('\u{25b8}'),
        "the cursor stays on the row just changed: {row}"
    );

    wizard.back();
    assert_eq!(wizard.step(), Step::Ready);
    let mut report = Vec::new();
    let outcome = wizard.apply(&mut report).unwrap();
    assert_eq!(outcome.voice, Voice::Reminders);
    assert_eq!(settings::load(temp.path()).voice, Voice::Reminders);
    assert!(
        String::from_utf8_lossy(&report).contains("Preferences saved"),
        "a page that saves in silence cannot be told from one that failed: \
         {report:?}"
    );
}

#[test]
fn the_voice_can_be_given_a_language_of_its_own_and_follows_leteo_until_it_is() {
    let temp = TempDir::new().unwrap();
    let mut wizard = Wizard::preferences(offer_in(temp.path()));

    let row = row_with(&wizard, "Sardi's language");
    assert!(row.contains("as Leteo"), "{row}");
    assert_eq!(
        wizard.voice_interface(),
        settings::Interface::English,
        "and what it resolves to is what Leteo speaks"
    );

    open_option(&mut wizard, Step::SardiLanguage);
    let painted = screen(&wizard);
    assert!(
        painted.contains("Which language should Sardi speak in?"),
        "{painted}"
    );
    assert!(
        painted.contains("▸ (●) as Leteo"),
        "it opens on the answer in force, which is to follow: {painted}"
    );
    assert!(
        painted.contains("whatever language Leteo itself is speaking"),
        "and says what following means: {painted}"
    );
    assert!(
        painted.contains("your agent's conversation"),
        "why the setting exists is not guessable from the question: {painted}"
    );
    for language in settings::Interface::ALL {
        assert!(painted.contains(language.as_str()), "{painted}");
    }

    while wizard.voice_interface() != settings::Interface::Basque {
        wizard.down();
        wizard.toggle();
    }
    assert_eq!(
        wizard.interface(),
        settings::Interface::English,
        "choosing a language for the voice moved the screens"
    );
    wizard.advance();
    assert_eq!(wizard.step(), Step::Options);
    assert!(
        row_with(&wizard, "Sardi's language").contains("euskara"),
        "a pinned language is named on the row rather than shown as following"
    );

    wizard.back();
    wizard.apply(&mut Vec::new()).unwrap();
    let saved = settings::load(temp.path());
    assert_eq!(saved.voice_language, Some(settings::Interface::Basque));
    assert_eq!(saved.interface, Some(settings::Interface::English));
    assert_eq!(saved.voice_language(), settings::Interface::Basque);
}

#[test]
fn a_voice_that_follows_is_written_as_following_rather_than_pinned() {
    let temp = TempDir::new().unwrap();
    settings::save(
        temp.path(),
        &Settings {
            interface: Some(settings::Interface::Spanish),
            ..Settings::default()
        },
    )
    .unwrap();

    let mut wizard = Wizard::preferences(offer_in(temp.path()));
    open_option(&mut wizard, Step::SardiVoice);
    wizard.down();
    wizard.advance();
    wizard.back();
    wizard.apply(&mut Vec::new()).unwrap();

    let saved = settings::load(temp.path());
    assert_eq!(saved.voice, Voice::Reminders, "the change asked for");
    assert_eq!(
        saved.voice_language, None,
        "and the voice was pinned to a language nobody chose"
    );
}

#[test]
fn esc_steps_back_out_of_the_options_page_rather_than_undoing_it() {
    let temp = TempDir::new().unwrap();
    let mut wizard = Wizard::preferences(offer_in(temp.path()));
    open_option(&mut wizard, Step::MemoryLanguage);
    wizard.cancel();
    assert_eq!(wizard.step(), Step::Options, "out of a setting to the list");
    wizard.cancel();
    assert_eq!(
        wizard.step(),
        Step::Ready,
        "and out of the list to the menu, saving what was picked"
    );

    let mut agents = Wizard::new(offer_with(None));
    agents.cancel();
    assert_eq!(agents.step(), Step::Cancelled);
}

#[test]
fn setting_up_agents_asks_about_agents_and_leaves_the_store_alone() {
    type Route = (&'static str, Box<dyn Fn(&mut Wizard)>);

    let routes: Vec<Route> = vec![
        (
            "no agent chosen at all",
            Box::new(|wizard: &mut Wizard| {
                wizard.advance();
            }),
        ),
        (
            "an agent that cannot take hooks",
            Box::new(|wizard: &mut Wizard| {
                wizard.down();
                wizard.toggle();
                wizard.advance();
            }),
        ),
        (
            "hooks declined",
            Box::new(|wizard: &mut Wizard| {
                wizard.toggle();
                wizard.advance();
                wizard.down();
                wizard.toggle();
                wizard.advance();
            }),
        ),
        (
            "hooks accepted",
            Box::new(|wizard: &mut Wizard| {
                wizard.toggle();
                wizard.advance();
                wizard.advance();
            }),
        ),
    ];

    for (route, walk) in routes {
        let mut wizard = Wizard::new(offer_with(None));
        walk(&mut wizard);
        let mut seen = Vec::new();
        for _ in 0..8 {
            if wizard.step().is_global() {
                seen.push(wizard.step());
            }
            if wizard.is_finished() {
                break;
            }
            wizard.advance();
        }
        assert!(
            seen.is_empty(),
            "setting up agents asked {seen:?} when {route}"
        );
        assert_eq!(wizard.step(), Step::Ready, "{route}");
    }
}

#[test]
fn the_voice_question_opens_on_the_answer_already_in_force() {
    let temp = TempDir::new().unwrap();
    settings::save(
        temp.path(),
        &Settings {
            language: None,
            voice: Voice::Quiet,
            interface: Some(settings::Interface::English),
            voice_language: None,
            context_size: None,
        },
    )
    .unwrap();
    let mut wizard = Wizard::preferences(offer_in(temp.path()));
    open_option(&mut wizard, Step::SardiVoice);
    let painted = screen(&wizard);
    assert!(painted.contains("▸ (●) quiet"), "{painted}");

    wizard.back();
    wizard.back();
    let mut report = Vec::new();
    let outcome = wizard.apply(&mut report).unwrap();
    assert_eq!(outcome.voice, Voice::Quiet);
    assert_eq!(settings::load(temp.path()).voice, Voice::Quiet);
}

#[test]
fn an_unset_language_stays_unset_rather_than_being_pinned_to_todays_machine() {
    let temp = TempDir::new().unwrap();
    assert!(!settings::path_in(temp.path()).exists());

    let mut fresh = Wizard::new({
        let mut offer = offer_with(None);
        offer.database = temp.path().join("leteo.db");
        offer.probe = probe_in(temp.path());
        offer
    });
    fresh.toggle();
    while !fresh.is_finished() {
        fresh.advance();
    }
    fresh.apply(&mut Vec::new()).unwrap();
    assert_eq!(
        settings::load(temp.path()).interface,
        None,
        "setting up an agent pinned a language nobody was asked about"
    );

    let mut wizard = Wizard::preferences(offer_in(temp.path()));
    open_option(&mut wizard, Step::InterfaceLanguage);
    while wizard.interface() != settings::Interface::Swedish {
        wizard.down();
        wizard.toggle();
    }
    wizard.back();
    wizard.back();
    assert!(wizard.is_finished());
    wizard.apply(&mut Vec::new()).unwrap();
    assert_eq!(
        settings::load(temp.path()).interface,
        Some(settings::Interface::Swedish)
    );
}

#[test]
fn setting_up_an_agent_keeps_the_preferences_it_never_asked_about() {
    let temp = TempDir::new().unwrap();
    settings::save(
        temp.path(),
        &Settings {
            language: Some("euskara".to_owned()),
            voice: Voice::Quiet,
            interface: Some(settings::Interface::Basque),
            voice_language: None,
            context_size: None,
        },
    )
    .unwrap();

    let mut wizard = wizard_in(temp.path());
    wizard.toggle();
    wizard.advance();
    while !wizard.is_finished() {
        wizard.advance();
    }
    wizard.apply(&mut Vec::new()).unwrap();

    let kept = settings::load(temp.path());
    assert_eq!(kept.voice, Voice::Quiet);
    assert_eq!(kept.language.as_deref(), Some("euskara"));
    assert_eq!(kept.interface, Some(settings::Interface::Basque));
}

#[test]
fn the_level_is_written_even_when_it_was_never_touched() {
    let temp = TempDir::new().unwrap();
    let mut wizard = Wizard::preferences(offer_in(temp.path()));
    open_option(&mut wizard, Step::SardiVoice);
    wizard.down();
    wizard.down();
    wizard.toggle();
    wizard.advance();
    wizard.back();

    let mut report = Vec::new();
    let outcome = wizard.apply(&mut report).unwrap();
    assert_eq!(outcome.voice, Voice::Quiet);
    assert_eq!(settings::load(temp.path()).voice, Voice::Quiet);
    assert!(settings::path_in(temp.path()).exists());
}

#[test]
fn a_window_too_short_for_the_answers_scrolls_them_and_keeps_the_question() {
    let temp = TempDir::new().unwrap();
    let mut wizard = Wizard::preferences(offer_in(temp.path()));
    open_option(&mut wizard, Step::SardiLanguage);

    let painted = |wizard: &Wizard| {
        wizard
            .render_within(12)
            .into_iter()
            .map(|row| row.text)
            .collect::<Vec<_>>()
    };
    let shown = painted(&wizard);
    assert!(shown.len() <= 12, "{shown:#?}");
    assert!(
        shown
            .iter()
            .any(|row| row.contains("Which language should Sardi speak in?")),
        "the question has to survive: {shown:#?}"
    );
    assert!(
        shown.iter().any(|row| row.contains("Enter choose")),
        "and so does the legend, which nobody can guess: {shown:#?}"
    );
    assert!(
        shown.iter().any(|row| row.starts_with('\u{25b8}')),
        "the cursor has to be in the window it is moving through: {shown:#?}"
    );
    assert!(
        shown.iter().any(|row| row.contains('\u{22ef}')),
        "and the screen has to say there is more than this: {shown:#?}"
    );

    while wizard.voice_interface() != settings::Interface::Swedish {
        wizard.down();
        wizard.toggle();
    }
    let shown = painted(&wizard);
    assert!(
        shown.iter().any(|row| row.contains("svenska")),
        "the last answer is unreachable: {shown:#?}"
    );
    assert!(
        shown.iter().any(|row| row.contains("Enter choose")),
        "{shown:#?}"
    );

    assert_eq!(wizard.render_within(80), wizard.render());
}

#[test]
fn the_key_legend_is_on_every_screen_that_takes_keys() {
    for wizard in [
        Wizard::new(offer_with(Some(found()))),
        Wizard::new(offer_with(None)),
    ] {
        let painted = screen(&wizard);
        assert!(painted.contains("space select"), "{painted}");
        assert!(painted.contains("enter continue"), "{painted}");
        assert!(painted.contains("backspace back"), "{painted}");
    }
}

#[test]
fn the_last_screen_offers_no_keys_and_says_what_is_happening() {
    let mut wizard = Wizard::new(offer_with(Some(found())));
    while !wizard.is_finished() {
        wizard.advance();
    }
    assert!(wizard.is_finished());
    let painted = screen(&wizard);
    assert!(!painted.contains("space select"), "{painted}");
    assert!(
        painted.contains(&sardi::reading(settings::Interface::English)),
        "{painted}"
    );

    let mut cancelled = Wizard::new(offer_with(Some(found())));
    cancelled.cancel();
    let painted = screen(&cancelled);
    assert!(!painted.contains("space select"), "{painted}");
    assert!(painted.contains("Nothing was changed."), "{painted}");
}

#[test]
fn hooks_reach_an_agent_that_was_already_configured_without_them() {
    let temp = TempDir::new().unwrap();
    let probe = probe_in(temp.path());
    let mut offer = offer_with(None);
    offer.database = temp.path().join("leteo.db");
    offer.probe = probe.clone();
    offer.agents[0].configured = true;

    let wizard = Wizard::new(offer);
    assert!(wizard.hooks, "the hook question defaults to yes");
    let mut report = Vec::new();
    let outcome = wizard.apply(&mut report).unwrap();

    let hooks = crate::setup::resolve_agent_paths("claude-code", &probe)
        .expect("Claude Code is a known agent")
        .hooks
        .expect("Claude Code takes hooks");
    let written = std::fs::read_to_string(&hooks)
        .unwrap_or_else(|error| panic!("{}: {error}", hooks.display()));
    assert!(
        written.contains("hook session-start"),
        "the hooks the wizard asked about have to reach the file: {written}"
    );
    assert!(
        outcome.configured.contains(&"claude-code".to_owned()),
        "and what changed has to be reported: {outcome:?}"
    );
}

#[test]
fn running_setup_again_does_not_forget_which_language_to_remember_in() {
    let temp = TempDir::new().unwrap();
    settings::save(
        temp.path(),
        &Settings {
            language: Some("español".to_owned()),
            voice: Voice::Quiet,
            interface: Some(settings::Interface::English),
            voice_language: None,
            context_size: None,
        },
    )
    .unwrap();

    let mut wizard = wizard_in(temp.path());
    while wizard.step() != Step::Ready && wizard.step() != Step::Cancelled {
        wizard.advance();
    }
    let mut report = Vec::new();
    wizard.apply(&mut report).unwrap();

    assert_eq!(
        settings::load(temp.path()).language.as_deref(),
        Some("español"),
        "the wizard answered a question nobody asked it"
    );
}

#[test]
fn the_language_question_offers_auto_and_opens_on_the_answer_in_force() {
    let temp = TempDir::new().unwrap();
    let mut wizard = Wizard::preferences(offer_in(temp.path()));
    open_option(&mut wizard, Step::MemoryLanguage);
    let painted = screen(&wizard);
    assert!(painted.contains("auto"), "{painted}");
    assert!(
        painted.contains("the language you write in"),
        "auto has to say what it does: {painted}"
    );
    assert!(
        painted.contains("settings.json"),
        "and the way out to any other language: {painted}"
    );

    settings::save(
        temp.path(),
        &Settings {
            language: Some("português do Brasil".to_owned()),
            voice: Voice::All,
            interface: Some(settings::Interface::English),
            voice_language: None,
            context_size: None,
        },
    )
    .unwrap();
    let mut wizard = Wizard::preferences(offer_in(temp.path()));
    open_option(&mut wizard, Step::MemoryLanguage);
    let painted = screen(&wizard);
    assert!(
        painted.contains("(●) português do Brasil"),
        "a hand-set language has to be offered back: {painted}"
    );
}

#[test]
fn a_store_that_already_holds_memories_is_warned_what_changing_the_language_costs() {
    let temp = TempDir::new().unwrap();
    settings::save(
        temp.path(),
        &Settings {
            interface: Some(settings::Interface::English),
            voice_language: None,
            ..Settings::default()
        },
    )
    .unwrap();
    let language_screen = |store_has_memories: bool| {
        let mut offer = offer_with(None);
        offer.database = temp.path().join("leteo.db");
        offer.probe = probe_in(temp.path());
        offer.agents[0].configured = true;
        offer.store_has_memories = store_has_memories;
        let mut wizard = Wizard::preferences(offer);
        open_option(&mut wizard, Step::MemoryLanguage);
        screen(&wizard)
    };

    let warned = language_screen(true);
    assert!(
        warned.contains("keep the language they were written in"),
        "a populated store has to be told what it keeps: {warned}"
    );
    assert!(
        warned.contains("two languages"),
        "and what that leaves behind: {warned}"
    );

    let quiet = language_screen(false);
    assert!(
        !quiet.contains("two languages"),
        "an empty store is not warned about memories it does not have: {quiet}"
    );
    assert!(
        quiet.contains("settings.json"),
        "the way out to any other language stays: {quiet}"
    );
}

#[test]
fn no_screen_is_painted_with_a_placeholder_still_in_it() {
    let unfilled = crate::i18n::unfilled_placeholder;
    for language in [settings::Interface::English, settings::Interface::Spanish] {
        let temp = TempDir::new().unwrap();
        settings::save(
            temp.path(),
            &Settings {
                interface: Some(language),
                voice_language: None,
                ..Settings::default()
            },
        )
        .unwrap();
        let mut offer = offer_with(Some(found()));
        offer.database = temp.path().join("leteo.db");
        offer.probe = probe_in(temp.path());
        offer.agents[0].configured = true;
        offer.store_has_memories = true;
        let mut wizard = Wizard::new(offer.clone());
        for _ in 0..10 {
            let painted = screen(&wizard);
            assert!(
                unfilled(&painted).is_none(),
                "{language:?} left {:?} unfilled on {:?}:\n{painted}",
                unfilled(&painted).unwrap(),
                wizard.step()
            );
            if wizard.is_finished() {
                break;
            }
            wizard.advance();
        }

        for setting in OPTIONS {
            let mut wizard = Wizard::preferences(offer.clone());
            for _ in 0..2 {
                let painted = screen(&wizard);
                assert!(
                    unfilled(&painted).is_none(),
                    "{language:?} left {:?} unfilled on {:?}:\n{painted}",
                    unfilled(&painted).unwrap(),
                    wizard.step()
                );
                if wizard.step() == Step::Options {
                    open_option(&mut wizard, setting);
                }
            }
        }
    }
}
