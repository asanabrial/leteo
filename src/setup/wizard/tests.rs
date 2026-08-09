use std::path::{Path, PathBuf};

use tempfile::TempDir;

use super::*;
use crate::settings::{self, Settings, Voice};

/// Paths under `directory`, so that applying writes nowhere real.
///
/// Applying is the half of the wizard that changes files, and it used to build
/// its own defaults — which meant the only way to exercise it was against the
/// machine running the tests. Every path it resolves now hangs off a temporary
/// directory instead.
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
        // Empty, which is what an Engram adoption is offered over: the two
        // questions read the same field from opposite ends.
        store_has_memories: false,
        probe: probe_in(fixture_data_dir()),
        // Claude Code is the only real agent that takes hooks, and the
        // fixture keeps that split so the hook question is exercised both
        // ways.
        agents: vec![
            choice("claude-code", "Claude Code", true),
            choice("opencode", "OpenCode", false),
            choice("codex", "Codex", false),
        ],
    }
}

/// A directory the fixture's database can claim to live in.
///
/// The wizard reads the settings file beside its database as it opens, so a
/// hardcoded `/tmp/leteo.db` made every test's result depend on whether the
/// developer running it happened to have a `/tmp/settings.json`. Per-process,
/// so anything a test applies lands here instead of on the real machine.
///
/// Not empty, though it was. The interface language falls back to the machine's
/// own when nothing is set, which is the behaviour somebody installing Leteo
/// wants and the last thing a test wants: with the file absent, every
/// assertion below about an English screen passed in Britain and failed in
/// Spain. Pinned here rather than in each test — a test that means to exercise
/// a language says so by writing its own settings, and the rest inherit an
/// answer that does not depend on where the machine was bought.
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

/// An Engram installation worth adopting.
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

/// Opens one of the options page's rows the way somebody would: move the
/// cursor down to it, and press enter.
///
/// By the setting rather than by a number of keystrokes, so a test says which
/// screen it meant and a row inserted above it does not silently send every
/// caller somewhere else.
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

/// The first rendered line holding `needle`, which on the options index is the
/// whole row: what the setting is called, and what it is set to.
fn row_with(wizard: &Wizard, needle: &str) -> String {
    wizard
        .render()
        .into_iter()
        .map(|row| row.text)
        .find(|text| text.contains(needle))
        .unwrap_or_default()
}

/// What the wizard tagged the first line containing `needle` as.
fn role_of(wizard: &Wizard, needle: &str) -> Option<Role> {
    wizard
        .render()
        .into_iter()
        .find(|row| row.text.contains(needle))
        .map(|row| row.role)
}

/// The wizard's current screen as one string, for asserting on.
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
    // The box says where Leteo lives, not what to do to it. So the screen
    // opens showing the truth, leaving it alone changes nothing, and taking
    // Leteo out of an agent is unticking it.
    let mut offer = offer_with(None);
    offer.agents[0].configured = true; // Claude Code
    let wizard = Wizard::new(offer);
    assert_eq!(wizard.chosen, vec![0], "opens ticked where Leteo is");

    let painted = screen(&wizard);
    assert!(painted.contains("[\u{2713}] Claude Code"), "{painted}");
    assert!(painted.contains("[ ] OpenCode"), "{painted}");
    // Nothing pending, so nothing is announced.
    assert!(!painted.contains("will be"), "{painted}");
}

#[test]
fn the_row_says_what_the_tick_is_about_to_do() {
    // Removing a working setup by moving a cursor and pressing space should
    // not be something somebody discovers afterwards.
    let mut offer = offer_with(None);
    offer.agents[0].configured = true;
    let mut wizard = Wizard::new(offer);

    wizard.toggle(); // untick Claude Code, which is configured
    wizard.down();
    wizard.toggle(); // tick OpenCode, which is not
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
    // The one in between stays untouched.
    assert!(painted.contains("  [ ] OpenCode"), "{painted}");

    // Space again unticks it.
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
    // Say no to adopting, then move on.
    wizard.down();
    wizard.toggle();
    wizard.advance();
    assert_eq!(wizard.step(), Step::ChooseAgents);

    // Tick an agent, go on to hooks, then come back.
    wizard.toggle();
    wizard.advance();
    assert_eq!(wizard.step(), Step::InstallHooks);
    wizard.back();
    assert_eq!(wizard.step(), Step::ChooseAgents);
    assert!(
        screen(&wizard).contains("[✓] Claude Code"),
        "the choice must survive going back"
    );

    // All the way back to the first question, still answered.
    wizard.back();
    assert_eq!(wizard.step(), Step::AdoptEngram);
    assert!(screen(&wizard).contains("▸ (●) No, start empty"));
}

#[test]
fn choosing_nothing_skips_the_hooks_question() {
    // There is no point asking about hooks for no agents, and nothing else in
    // this flow is owed — what the store is like is asked on its own page.
    let mut wizard = Wizard::new(offer_with(None));
    wizard.advance();
    assert_eq!(wizard.step(), Step::Ready);
}

#[test]
fn backing_out_of_the_first_question_cancels() {
    let mut wizard = Wizard::new(offer_with(Some(found())));
    wizard.back();
    assert_eq!(wizard.step(), Step::Cancelled);

    // And with no Engram to adopt, the first question is the agent list.
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
    // The driver colours by role, so a line given the wrong one is painted
    // wrong — and nothing about the words would show it. The one that
    // matters most is the cursor: if the focused choice does not say it is
    // focused, the highlight lands on the wrong row or nowhere.
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

    // And the roles follow the cursor rather than the wording.
    wizard.down();
    assert_eq!(
        role_of(&wizard, "Yes, bring them across"),
        Some(Role::Choice)
    );
    assert_eq!(role_of(&wizard, "No, start empty"), Some(Role::Focused));
}

#[test]
fn a_transition_that_changes_nothing_says_so() {
    // The driver only repaints when one of these reports true, so a
    // transition that lies here costs a full redraw per keystroke — or,
    // worse, leaves the screen showing the previous state.
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
    // The regression this guards is silence: the wizard drops the adoption
    // question once the store has memories, and without this note Engram
    // goes unmentioned even when it is installed and further ahead.
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
    // Requesting hooks for an agent that has nowhere to put them is an
    // error, and that error costs the agent its whole setup. So the
    // question is skipped rather than asked and then ignored.
    let mut wizard = Wizard::new(offer_with(None));
    wizard.down(); // OpenCode
    wizard.toggle();
    wizard.advance();
    assert_eq!(
        wizard.step(),
        Step::Ready,
        "no chosen agent takes hooks, so that question is skipped"
    );

    let mut wizard = Wizard::new(offer_with(None));
    wizard.toggle(); // Claude Code
    wizard.down();
    wizard.toggle(); // and OpenCode alongside it
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

/// A wizard whose store is `directory`, with the hook-capable agent ticked
/// so the flow reaches the voice question.
fn wizard_in(directory: &Path) -> Wizard {
    Wizard::new(offer_in(directory))
}

/// The offer `wizard_in` builds, for the flows that enter it another way.
fn offer_in(directory: &Path) -> Offer {
    // English unless the test has already said otherwise, for the reason given
    // at `fixture_data_dir`: an unset interface language follows the machine,
    // so without this every assertion about an English screen below would be a
    // test of where the developer lives. Written only when nothing is there, so
    // a test that saved its own settings first keeps them.
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
    offer.agents[0].configured = true; // Claude Code, the one taking hooks
    offer
}

#[test]
fn the_options_flow_leaves_every_agent_alone() {
    // What the home menu opens. It is the same wizard in its other flow, and
    // the property that makes that safe is this one: the ticks are loaded from
    // disk and never shown, so the difference `apply` works out is empty and no
    // agent is configured, removed or reconfigured by somebody who came to
    // change what language Leteo speaks.
    let temp = TempDir::new().unwrap();
    let mut wizard = Wizard::preferences(offer_in(temp.path()));

    assert_eq!(wizard.step(), Step::Options);
    // Leaving the index leaves the page. It must not reverse into the agent
    // questions this flow never asked.
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
    // The shape this replaces asked the three in a fixed order, one after
    // another: somebody who wanted Sardi quieter answered two questions about
    // language on the way, and had to walk past the answer they already had to
    // reach the one they came for. That is right for a setup being walked
    // through and wrong for a settings page — so the page is now the list, and
    // the questions are behind it.
    let temp = TempDir::new().unwrap();
    let mut wizard = Wizard::preferences(offer_in(temp.path()));

    assert_eq!(wizard.step(), Step::Options, "it opens on the list");
    let painted = screen(&wizard);
    assert!(
        painted.contains("What would you like to change?"),
        "{painted}"
    );
    // Every row says what it is set to, so the page answers the question
    // somebody arrived with before they press anything at all.
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
    // And none of the three questions is on it. The list is doors, not choices.
    assert!(!painted.contains("say out loud"), "{painted}");
    assert!(!painted.contains("Which language"), "{painted}");

    // The row somebody came for opens on its own, without the two above it.
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

    // Enter takes the row under the cursor and comes back to the list, which
    // now shows the new answer against that setting.
    wizard.down();
    wizard.advance();
    assert_eq!(wizard.step(), Step::Options);
    let row = row_with(&wizard, "Sardi's voice");
    assert!(row.contains("reminders"), "{row}");
    assert!(
        row.starts_with('\u{25b8}'),
        "the cursor stays on the row just changed: {row}"
    );

    // Leaving the list is what writes the file, and it says so: this is the
    // only screen in the program that saves on the way out.
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
    // Two settings because the lines are read in two places: Leteo's screens
    // are a program somebody opens, and Sardi's lines are written by hooks into
    // an agent's conversation, beside whatever language that is being held in.
    let temp = TempDir::new().unwrap();
    let mut wizard = Wizard::preferences(offer_in(temp.path()));

    // Following is a named answer on the row, not a blank one, and not the
    // language it resolves to: "the same as above" and "this one, pinned" are
    // different answers and the row has to say which this is.
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

    // Pinning one moves the voice and leaves the screens where they were.
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
    // The trap this guards is the one the interface language already fell into
    // once: `None` here is a live answer — "whatever Leteo speaks" — and
    // writing back what it resolves to today would pin the voice to a language
    // nobody chose for it, on a page they opened to change something else.
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
    wizard.advance(); // change something else entirely
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
    // The two flows mean different things by Esc, and both are what the key
    // already means where it is pressed. Setting up agents is one act, so
    // leaving it half done undoes it. The options page is a list of settings
    // that each take effect as they are picked and are shown as taken on the
    // row — so there is nothing there to undo, and a page that threw the
    // answers away on the way out would be discarding what it had just
    // reported as done.
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

    // The agent flow is unchanged: there Esc is the way to abandon it.
    let mut agents = Wizard::new(offer_with(None));
    agents.cancel();
    assert_eq!(agents.step(), Step::Cancelled);
}

#[test]
fn setting_up_agents_asks_about_agents_and_leaves_the_store_alone() {
    // The defect this exists to prevent, and it shipped twice in two shapes.
    //
    // First: both agent questions jumped straight to `Ready` when they came out
    // negative, and the global questions sat behind them — so anybody who
    // declined hooks, or set up an agent that cannot take them, was never asked
    // what language to remember in, while `mem_context` went on reading that
    // setting on every call, hooks or no hooks. That was fixed by putting the
    // globals after every exit.
    //
    // Second, and what this now checks: the globals had no business in this
    // flow at all. Somebody adding an agent should not be walked through what
    // language the store speaks and how loud Sardi is; somebody changing those
    // should not have to walk an agent installation. They live on their own
    // page — see `the_options_flow_asks_the_three_store_questions_and_nothing_else`
    // — and this asserts the other half of that split: every route here reaches
    // the end without asking one of them.
    /// One way of getting through the per-agent half of the flow.
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
                wizard.down(); // OpenCode
                wizard.toggle();
                wizard.advance();
            }),
        ),
        (
            "hooks declined",
            Box::new(|wizard: &mut Wizard| {
                wizard.toggle(); // Claude Code
                wizard.advance();
                wizard.down();
                wizard.toggle(); // "No, MCP tools only"
                wizard.advance();
            }),
        ),
        (
            "hooks accepted",
            Box::new(|wizard: &mut Wizard| {
                wizard.toggle(); // Claude Code
                wizard.advance();
                wizard.advance(); // keep hooks
            }),
        ),
    ];

    for (route, walk) in routes {
        let mut wizard = Wizard::new(offer_with(None));
        walk(&mut wizard);
        let mut seen = Vec::new();
        // Walk to the end, collecting anything global on the way.
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
    // Somebody who silenced Sardi last month and opens the options again for
    // another reason must not have it turned back on behind them.
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

    // And leaving the screen alone keeps it, rather than writing the
    // default over the top.
    wizard.back();
    wizard.back();
    let mut report = Vec::new();
    let outcome = wizard.apply(&mut report).unwrap();
    assert_eq!(outcome.voice, Voice::Quiet);
    assert_eq!(settings::load(temp.path()).voice, Voice::Quiet);
}

#[test]
fn an_unset_language_stays_unset_rather_than_being_pinned_to_todays_machine() {
    // `interface` unset means "follow this machine", which keeps being true as
    // the machine changes. The wizard resolves it to paint its own screens, and
    // writing *that* back would freeze it: the language the locale happened to
    // report the day somebody installed Leteo, recorded as though they had
    // chosen it. Nobody would ever see the question again either, because the
    // flow that used to ask it no longer does.
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

    // And the options page, which does ask, writes the answer. Which entry the
    // cursor starts on depends on the machine, so the answer is named rather
    // than counted in keystrokes.
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
    // The flows are split, so this one no longer asks what the store is like —
    // which makes it the flow that must not answer for it either. `apply`
    // writes all three settings whatever happened, and the values it writes
    // come from the file it read on the way in. Without that they would be a
    // fresh `Settings`: default voice, no language, no interface, silently over
    // the top of somebody's answers every time they added an agent.
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
    wizard.toggle(); // untick the configured agent, so this run does something
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
    // A file that only appears once somebody dissents leaves no way to tell
    // "never asked" from "asked and chose the loud one", so a later release
    // changing the default would move them without asking.
    let temp = TempDir::new().unwrap();
    let mut wizard = Wizard::preferences(offer_in(temp.path()));
    open_option(&mut wizard, Step::SardiVoice);
    wizard.down();
    wizard.down();
    wizard.toggle(); // quiet
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
    // Thirteen answers under a question runs to twenty-one rows, and a forty
    // row terminal is not the only kind. Cut off at the bottom — which is what
    // happened — the last few languages and the key legend were simply not
    // drawn, and the cursor could be moved onto a row nobody could see.
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

    // Walking to the far end of the list brings it into view rather than
    // running off the bottom of the panel.
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

    // And a window with room to spare is left exactly as it was.
    assert_eq!(wizard.render_within(80), wizard.render());
}

#[test]
fn the_key_legend_is_on_every_screen_that_takes_keys() {
    // Nobody can guess that space ticks and backspace goes back.
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
    // Once there is nothing left to answer, a legend would advertise keys
    // that do nothing. The screen reports the work instead.
    let mut wizard = Wizard::new(offer_with(Some(found())));
    // Adoption, agents, then the two questions the store is owed whatever was
    // chosen above.
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

/// A configured agent still gets the hooks it was just asked about.
///
/// `configured` means the MCP server is registered in the file and nothing
/// more — it says nothing about hooks. So somebody who answered "No, MCP tools
/// only" the first time arrives here already ticked, is asked the hook question
/// again, and used to have the answer thrown away: the arm for an agent already
/// on disk did nothing at all. The question was real; only its effect was
/// missing.
#[test]
fn hooks_reach_an_agent_that_was_already_configured_without_them() {
    let temp = TempDir::new().unwrap();
    let probe = probe_in(temp.path());
    let mut offer = offer_with(None);
    offer.database = temp.path().join("leteo.db");
    offer.probe = probe.clone();
    // Claude Code: registered, and nothing on disk says whether it has hooks.
    offer.agents[0].configured = true;

    // Configured agents open ticked, and the hook question defaults to yes, so
    // this is what somebody gets for pressing enter through the flow.
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
    // The command is the running executable rather than the word "leteo", so
    // the event is what identifies these as Leteo's.
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
    // The wizard asks about the voice and not about the language, so building
    // a fresh `Settings` from its own answers erases the language every time
    // somebody re-runs setup to add an agent. Silently, and noticed weeks
    // later when memories start coming back in English.
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
    // Auto is a named choice, not the absence of one. Somebody who never
    // thinks about it should still see what Leteo is doing, and somebody who
    // pinned a language must not have it quietly swapped for auto by walking
    // through setup again.
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

    // A pinned language is on the menu and selected, even one the offered
    // list would never have proposed.
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
    // The screen says what is being chosen. This says what happens next, which
    // is the part nobody would think to ask: a language governs what is
    // written from here on and nothing rewrites what is stored. A store with
    // memories in it therefore ends up holding two, and a search reaches the
    // half it is asked in.
    //
    // Somebody who learns that three weeks later has three weeks of memories
    // they cannot find and no reason to connect the two.
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
        // Through the options flow, which is the only one that asks it.
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

    // A store with nothing in it has nothing to lose, and the warning would be
    // a sentence about a problem that cannot happen to them.
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
    // The catalogue holds templates — `{agents}`, `{name}`, `{error}` — and a
    // call site that forgets one does not fail to compile. It renders the
    // placeholder, so the screen says "in {agents}?" to somebody's face.
    //
    // A test already checks the two languages declare the same placeholders.
    // That one compares the catalogue against itself; this one is the half it
    // cannot see, which is whether anything fills them.
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
        // Every screen the flow can reach, including the one that only exists
        // when there is an Engram installation to adopt.
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

        // And the options page, whose index carries a `{name}` of its own —
        // the row for Sardi's voice — on a screen the walk above never opens.
        for setting in OPTIONS {
            let mut wizard = Wizard::preferences(offer.clone());
            // The index, then the screen behind this one of its rows.
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
