//! The interactive setup flow.
//!
//! `leteo setup` with no agent has to serve two callers: a person who just
//! installed Leteo and wants to be walked through it, and a script that wants
//! the list of agents as JSON. The terminal tells them apart.
//!
//! The flow is split in two on purpose. [`Wizard`] is a state machine that
//! knows nothing about terminals — every key becomes a method call, and the
//! screen is a `Vec<String>` — so the whole thing can be driven by a test. The
//! crossterm driver at the bottom only maps keys onto it and paints what it
//! returns.

mod offer;
mod screen;
mod terminal;

pub use offer::{AgentChoice, Offer, adoption_note, offer};
pub use screen::{Role, Row};
pub use terminal::run_interactive;

use screen::{BRAND, checkbox, entry, radio, windowed};

use anyhow::Result;

use crate::engram;
use crate::i18n::fill;
use crate::sardi;
use crate::setup::SetupOptions;

/// Where the flow currently is.
///
/// Two flows share these steps and neither visits the other's. Setting up
/// agents asks which agents and what to install in them; the options page asks
/// what the *store* is like — see [`Flow`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Whether to take the detected Engram installation's memories.
    AdoptEngram,
    /// Which agents to configure. Several may be chosen.
    ChooseAgents,
    /// Whether those agents also get the lifecycle hooks.
    InstallHooks,
    /// The options page itself: one row per setting, each showing the answer in
    /// force, and you open the one you came to change.
    ///
    /// The three below used to be asked one after another, which is right for a
    /// setup somebody is being walked through and wrong for a settings page:
    /// changing how loud Sardi is meant answering two questions about language
    /// first. This is the resting place of that flow — every screen returns
    /// here, and leaving here is what saves.
    Options,
    /// Which language Leteo's own screens are written in.
    InterfaceLanguage,
    /// Which language Sardi speaks, when it is not Leteo's.
    SardiLanguage,
    /// Which language memories are written in, or auto.
    MemoryLanguage,
    /// How much of its work Sardi says out loud.
    SardiVoice,
    /// Everything has been decided; the caller should apply it.
    Ready,
    /// The person backed out.
    Cancelled,
}

/// The rows of [`Step::Options`], in the order they are listed.
///
/// One list rather than a set of rows in the renderer and a set of arms in the
/// key handler: those would be edited separately, and then the row somebody
/// moved the cursor onto and the screen pressing enter opens would drift apart.
/// The three languages first and the volume last, and Sardi's language directly
/// under Leteo's because that is what it follows until it is given one.
const OPTIONS: [Step; 4] = [
    Step::InterfaceLanguage,
    Step::SardiLanguage,
    Step::MemoryLanguage,
    Step::SardiVoice,
];

impl Step {
    /// Whether this question belongs to the store rather than to any agent.
    ///
    /// One answer serves every agent pointed at the store, including the ones
    /// added next month, and all three land in the same `settings.json`.
    ///
    /// Only the tests read this. The flow itself names the steps directly, and
    /// a second source of truth for "which of these is global" is exactly what
    /// they are there to check against.
    #[cfg(test)]
    fn is_global(self) -> bool {
        matches!(
            self,
            Self::InterfaceLanguage | Self::SardiLanguage | Self::MemoryLanguage | Self::SardiVoice
        )
    }
}

/// Which of the two flows a wizard is running.
///
/// # Why the split, and what was wrong before
///
/// Everything global used to be asked at the tail of the agent setup, and the
/// arrangement kept producing the same defect in different places: a setting
/// that belongs to the store, reachable only by answering questions about
/// agents. It cost the memory language first — behind a hook decision, so
/// anybody configuring an agent that takes no hooks was never asked — and the
/// fix at the time only moved it down the same corridor.
///
/// [`crate::settings::Voice`] was the one left. It reads as per-agent, and the
/// argument was that its lines are emitted by hooks, so with hooks declined it
/// governs nothing. But it is stored once, in the store's own `settings.json`,
/// and it governs every agent that has hooks — which is the definition of the
/// other kind. Somebody who wanted Sardi quieter had to walk an agent
/// installation to say so, and got asked again on every later run.
///
/// So: [`Flow::Agents`] asks which agents and whether they get hooks or only
/// the MCP tools. [`Flow::Options`] asks the three that describe the store.
/// Neither reaches the other's steps, and the options are on the home menu
/// rather than at the end of an install.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Flow {
    Agents,
    Options,
}

/// What the wizard decided.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub adopted: Option<i64>,
    pub configured: Vec<String>,
    /// Agents Leteo was taken out of, because their box was unticked.
    pub removed: Vec<String>,
    pub cancelled: bool,
    /// The level that was written to the settings file.
    pub voice: crate::settings::Voice,
}

/// The interactive flow, as a state machine.
#[derive(Debug)]
pub struct Wizard {
    offer: Offer,
    step: Step,
    /// Which line the cursor is on, within the current step.
    cursor: usize,
    /// Positions in `offer.agents` that are ticked.
    chosen: Vec<usize>,
    adopt: bool,
    hooks: bool,
    voice: crate::settings::Voice,
    language: Option<String>,
    language_choices: Vec<Option<String>>,
    /// The interface language as it currently stands, resolved rather than
    /// optional: the screen has to be painted in *some* language, and `None`
    /// means "follow the machine", which is an answer this already has.
    interface: crate::settings::Interface,
    /// The same setting as it sits in the file, which `None` is a real value
    /// of — see the write in [`Wizard::apply`] for why both are kept.
    interface_setting: Option<crate::settings::Interface>,
    /// The language Sardi speaks, where `None` means Leteo's own.
    ///
    /// Kept unresolved, and there is nothing to resolve it against here:
    /// `None` is the answer "follow the other setting", which stays true as
    /// that one changes. [`Wizard::voice_interface`] is what resolves it for
    /// the lines the character actually says.
    voice_language: Option<crate::settings::Interface>,
    /// Which of the two flows this is; see [`Flow`].
    ///
    /// It governs where the flow starts, what backing out of the first question
    /// means, and — the one that had to be found by testing for it — that
    /// [`Wizard::apply`] stops before the agent loop in [`Flow::Options`].
    ///
    /// That last one looked unnecessary. The ticks come off disk and match what
    /// is installed, so the difference to apply is empty; every arm keyed on
    /// *changing* a tick is dead down that path. What is not dead is the arm for
    /// a tick that did not change: it re-runs setup to upsert hooks, keyed on
    /// `self.hooks`, which down that path is not an answer but the constructor's
    /// default. Somebody opening the options screen would have had lifecycle
    /// hooks installed for every configured agent, silently.
    flow: Flow,
}

impl Wizard {
    pub fn new(offer: Offer) -> Self {
        // With nothing to adopt there is no question to ask, so the flow starts
        // where the first real choice is.
        let step = if offer.engram.is_some() {
            Step::AdoptEngram
        } else {
            Step::ChooseAgents
        };
        // Ticked where Leteo already lives. The box says where Leteo is
        // installed rather than what to do to it, so arriving at this screen
        // shows the current state and leaving a box as found changes nothing.
        // Unticking one is how it is taken out.
        let chosen = offer
            .agents
            .iter()
            .enumerate()
            .filter(|(_, agent)| agent.configured)
            .map(|(index, _)| index)
            .collect();
        // Shown as it currently stands rather than as a fresh default, so
        // somebody who silenced Sardi last month and runs setup again to add an
        // agent does not have it turned back on behind them.
        let settings = crate::settings::load_beside(&offer.database);
        let voice = settings.voice;
        let language = settings.language.clone();
        let language_choices = {
            let mut choices =
                crate::settings::language_choices(crate::settings::system_language().as_deref());
            // A language somebody set by hand is on the menu too, or the
            // wizard would offer them no way back to the answer they already
            // have and quietly move them off it.
            if let Some(current) = &language
                && !choices
                    .iter()
                    .any(|choice| choice.as_deref() == Some(current.as_str()))
            {
                choices.push(Some(current.clone()));
            }
            choices
        };
        // Resolved here rather than stored as an `Option`: the wizard paints
        // every screen in this language, including the one that offers to
        // change it, so it needs the answer that is in force right now.
        let interface = settings.interface();
        Self {
            offer,
            step,
            cursor: 0,
            chosen,
            adopt: true,
            hooks: true,
            voice,
            language,
            language_choices,
            interface,
            interface_setting: settings.interface,
            voice_language: settings.voice_language,
            flow: Flow::Agents,
        }
    }

    /// The settings that belong to the store, as a page rather than a flow.
    ///
    /// What language Leteo speaks in is not something somebody should have to
    /// walk an agent-configuration flow to reach. It governs every screen and
    /// every line Sardi says, it is asked once for the whole store, and until
    /// this existed the only way back to it was `leteo setup` — six questions
    /// about agents, hooks and adoption to arrive at one about language, with
    /// every answer along the way a chance to change something nobody came to
    /// change.
    ///
    /// It opens on [`Step::Options`]: the three settings side by side, each
    /// showing what it is set to, and one screen behind whichever is chosen. A
    /// person opening this has come to change one thing, and asking them the
    /// other two on the way is the same defect one size smaller.
    ///
    /// Same state and same apply. Only the entrance and the shape are
    /// different, and the ticks are left exactly as they were found so that
    /// leaving here writes preferences and touches no agent.
    pub fn preferences(offer: Offer) -> Self {
        let mut wizard = Self::new(offer);
        wizard.flow = Flow::Options;
        // No adoption down this path whatever was detected: somebody who opened
        // the options screen did not ask to take another program's memories,
        // and `apply` would otherwise do it on the way past.
        wizard.adopt = false;
        wizard.cursor = 0;
        wizard.step = Step::Options;
        wizard
    }

    pub fn step(&self) -> Step {
        self.step
    }

    /// The language the screens are currently being painted in.
    ///
    /// Read by the terminal UI on every key, not only by the tests: the options
    /// page can change it, and the window around the page is painted from the
    /// dashboard's own copy. Without this the border, the header and the footer
    /// stayed in the old language until the page was closed — one screen in two
    /// languages, which reads as a setting that half took.
    pub fn interface(&self) -> crate::settings::Interface {
        self.interface
    }

    /// The language Sardi is currently speaking: its own, or Leteo's.
    ///
    /// Every line the character says goes through this — the wizard's own
    /// report as well as the dashboard header the caller paints — so that
    /// choosing a language for the voice takes effect the moment it is chosen,
    /// exactly as choosing one for the screens does.
    pub fn voice_interface(&self) -> crate::settings::Interface {
        self.voice_language.unwrap_or(self.interface)
    }

    /// What the voice's language question offers: Leteo's own, then each of
    /// the twelve.
    ///
    /// `None` heads the list rather than trailing it, because it is what the
    /// setting does until somebody says otherwise and the list should open on
    /// the ordinary answer.
    fn voice_language_choices() -> Vec<Option<crate::settings::Interface>> {
        std::iter::once(None)
            .chain(crate::settings::Interface::ALL.map(Some))
            .collect()
    }

    pub fn is_finished(&self) -> bool {
        matches!(self.step, Step::Ready | Step::Cancelled)
    }

    /// How many selectable lines the current step has.
    fn line_count(&self) -> usize {
        match self.step {
            Step::AdoptEngram | Step::InstallHooks => 2,
            Step::Options => OPTIONS.len(),
            Step::SardiVoice => crate::settings::Voice::ALL.len(),
            Step::InterfaceLanguage => crate::settings::Interface::ALL.len(),
            Step::SardiLanguage => Self::voice_language_choices().len(),
            Step::MemoryLanguage => self.language_choices.len(),
            Step::ChooseAgents => self.offer.agents.len(),
            Step::Ready | Step::Cancelled => 0,
        }
    }

    /// Moves the cursor up. Reports whether the screen changed, so the caller
    /// can skip a redraw that would paint exactly what is already there.
    pub fn up(&mut self) -> bool {
        let count = self.line_count();
        if count == 0 {
            return false;
        }
        // Wrapping keeps a long agent list reachable from either end.
        let next = (self.cursor + count - 1) % count;
        let moved = next != self.cursor;
        self.cursor = next;
        moved
    }

    /// Moves the cursor down. Reports whether the screen changed.
    pub fn down(&mut self) -> bool {
        let count = self.line_count();
        if count == 0 {
            return false;
        }
        let next = (self.cursor + 1) % count;
        let moved = next != self.cursor;
        self.cursor = next;
        moved
    }

    /// Space: ticks a box, or picks one of the two radio options. Reports
    /// whether the screen changed.
    pub fn toggle(&mut self) -> bool {
        match self.step {
            Step::AdoptEngram => {
                let next = self.cursor == 0;
                let changed = self.adopt != next;
                self.adopt = next;
                changed
            }
            Step::InstallHooks => {
                let next = self.cursor == 0;
                let changed = self.hooks != next;
                self.hooks = next;
                changed
            }
            Step::SardiVoice => {
                let next = crate::settings::Voice::ALL[self.cursor];
                let changed = self.voice != next;
                self.voice = next;
                changed
            }
            // Takes effect on this very screen. The question is asked in the
            // language being chosen, so the answer has to be legible from the
            // answer — picking Spanish and watching the words stay English
            // would read as a setting that did not take.
            Step::InterfaceLanguage => {
                let next = crate::settings::Interface::ALL[self.cursor];
                let changed = self.interface != next;
                self.interface = next;
                // Picking one is what turns "follow the machine" into a choice,
                // and what makes it survive the write in `apply`.
                self.interface_setting = Some(next);
                changed
            }
            // Takes effect on this screen too, for the same reason: the hint
            // under it is one of Sardi's own lines, so a choice that did not
            // show would be a choice nothing confirmed.
            Step::SardiLanguage => {
                let next = Self::voice_language_choices()[self.cursor];
                let changed = self.voice_language != next;
                self.voice_language = next;
                changed
            }
            Step::MemoryLanguage => {
                let next = self.language_choices[self.cursor].clone();
                let changed = self.language != next;
                self.language = next;
                changed
            }
            Step::ChooseAgents => {
                if let Some(at) = self.chosen.iter().position(|index| *index == self.cursor) {
                    self.chosen.remove(at);
                } else {
                    self.chosen.push(self.cursor);
                }
                true
            }
            // An index of settings has nothing to tick. Enter opens the row —
            // see `advance` — and space is left doing nothing rather than being
            // given a second meaning nobody would guess.
            Step::Options | Step::Ready | Step::Cancelled => false,
        }
    }

    /// Enter: moves to the next step. Reports whether the screen changed.
    ///
    /// # The defect this shape exists to prevent
    ///
    /// The global questions used to sit at the tail of this flow, behind the
    /// per-agent ones, and the arrangement kept costing the same thing. Both
    /// agent questions jumped straight to [`Step::Ready`] when they came out
    /// negative — no hook-capable agent chosen, or hooks declined — so anybody
    /// setting up an agent that cannot take hooks was never asked **what
    /// language to remember in**, a setting `mem_context` reads and hooks have
    /// nothing to do with.
    ///
    /// Putting them after every exit fixed that one and left the shape. What
    /// finally goes is the shape: a setting that describes the store is not
    /// reached by answering questions about agents at all. It has its own page,
    /// off the home menu, and this flow ends where the agents do.
    pub fn advance(&mut self) -> bool {
        let before = self.step;
        // Matched on the copy rather than on the field: the arms below call
        // methods that take `&mut self`, which they cannot while the scrutinee
        // is still borrowed.
        self.step = match before {
            Step::AdoptEngram => {
                self.cursor = 0;
                Step::ChooseAgents
            }
            Step::ChooseAgents => {
                // Only worth asking when one of the chosen agents could
                // actually take hooks. Most cannot, and a question whose answer
                // is ignored is worse than no question.
                if self.any_chosen_supports_hooks() {
                    self.cursor = if self.hooks { 0 } else { 1 };
                    Step::InstallHooks
                } else {
                    Step::Ready
                }
            }
            Step::InstallHooks => Step::Ready,
            // The index: enter opens whichever setting the cursor is on.
            Step::Options => self.open(OPTIONS[self.cursor]),
            // And enter on a setting takes what the cursor is on and returns to
            // the index, rather than walking on to the next question. Taking it
            // matters: the cursor opens on the answer in force, so somebody who
            // moves it and presses enter has said what they want, and a screen
            // that made them press space first would leave the cursor sitting
            // on one answer with another one picked.
            Step::InterfaceLanguage
            | Step::SardiLanguage
            | Step::MemoryLanguage
            | Step::SardiVoice => {
                self.toggle();
                self.back_to_index(before)
            }
            other => other,
        };
        self.step != before
    }

    /// Opens one of the settings, with the cursor on the answer in force.
    ///
    /// So that leaving a screen alone keeps what it was showing, and so that
    /// enter — which takes the row under the cursor — is a no-op rather than a
    /// change nobody asked for.
    fn open(&mut self, step: Step) -> Step {
        self.cursor = match step {
            Step::InterfaceLanguage => self.interface_cursor(),
            Step::SardiLanguage => self.voice_language_cursor(),
            Step::MemoryLanguage => self.language_cursor(),
            Step::SardiVoice => self.voice_cursor(),
            _ => 0,
        };
        step
    }

    /// Back to the index, with the cursor on the row just left.
    fn back_to_index(&mut self, from: Step) -> Step {
        self.cursor = OPTIONS.iter().position(|row| *row == from).unwrap_or(0);
        Step::Options
    }

    /// Where the cursor sits when the interface question opens.
    fn interface_cursor(&self) -> usize {
        crate::settings::Interface::ALL
            .iter()
            .position(|language| *language == self.interface)
            .unwrap_or(0)
    }

    /// Where the cursor sits when the voice's language question opens.
    fn voice_language_cursor(&self) -> usize {
        Self::voice_language_choices()
            .iter()
            .position(|choice| *choice == self.voice_language)
            .unwrap_or(0)
    }

    /// Where the cursor sits when the language question opens: on the answer
    /// already in force, for the same reason as the voice.
    fn language_cursor(&self) -> usize {
        self.language_choices
            .iter()
            .position(|choice| *choice == self.language)
            .unwrap_or(0)
    }

    /// Where the cursor sits when the voice question opens: on the answer that
    /// is in force, so leaving the screen alone keeps it.
    fn voice_cursor(&self) -> usize {
        crate::settings::Voice::ALL
            .iter()
            .position(|voice| *voice == self.voice)
            .unwrap_or(0)
    }

    /// Backspace: returns to the previous step, or gives up on the first one.
    /// Reports whether the screen changed.
    pub fn back(&mut self) -> bool {
        let before = self.step;
        self.step = match before {
            // Backing out of the first question is the only way to leave
            // without deciding anything, so it has to mean cancel.
            Step::AdoptEngram => Step::Cancelled,
            Step::ChooseAgents => {
                if self.offer.engram.is_some() {
                    self.cursor = if self.adopt { 0 } else { 1 };
                    Step::AdoptEngram
                } else {
                    Step::Cancelled
                }
            }
            Step::InstallHooks => {
                self.cursor = 0;
                Step::ChooseAgents
            }
            // Every setting sits one step under the index, so backing out of
            // one is the index — never the previous setting, which is not
            // where it was opened from.
            Step::InterfaceLanguage
            | Step::SardiLanguage
            | Step::MemoryLanguage
            | Step::SardiVoice => self.back_to_index(before),
            // And leaving the index is leaving the page, which is what writes
            // the file. There is nothing to cancel here: each answer is one
            // setting, shown on the index the moment it is picked, so a screen
            // that threw them away on the way out would be discarding something
            // it had already reported as done.
            Step::Options => Step::Ready,
            other => other,
        };
        self.step != before
    }

    /// The chosen agents that can take lifecycle hooks, by display name.
    fn hook_capable_names(&self) -> Vec<&str> {
        self.chosen
            .iter()
            .filter_map(|index| self.offer.agents.get(*index))
            .filter(|agent| agent.supports_hooks)
            .map(|agent| agent.display_name.as_str())
            .collect()
    }

    fn any_chosen_supports_hooks(&self) -> bool {
        !self.hook_capable_names().is_empty()
    }

    /// Esc: abandons the setup, or steps back out of an options screen.
    ///
    /// Reports whether the screen changed.
    ///
    /// The two flows mean different things by it, and both are what the key
    /// already means where it is pressed. Setting up agents is one act with an
    /// end, so leaving it half done has to undo it. The options page is not an
    /// act at all — it is a list of settings, each of which takes effect as it
    /// is picked — so there is nothing there for Esc to undo, and it steps back
    /// one instead: out of a setting to the index, out of the index to the
    /// menu.
    pub fn cancel(&mut self) -> bool {
        if self.flow == Flow::Options {
            return self.back();
        }
        let changed = self.step != Step::Cancelled;
        self.step = Step::Cancelled;
        changed
    }

    /// One row of the options index: what the setting is called, and what it is
    /// currently set to.
    ///
    /// The value is read from the same field the screen behind the row edits,
    /// so the index cannot show one thing and the screen open another.
    fn option_row(&self, step: Step) -> (String, String) {
        let say = crate::i18n::screens(self.interface);
        match step {
            Step::InterfaceLanguage => (
                say.option_interface.to_owned(),
                self.interface.as_str().to_owned(),
            ),
            Step::SardiLanguage => (
                fill(say.option_voice_language, "name", sardi::NAME),
                // Following Leteo is a named answer, not a blank one. Showing
                // the language it resolves to would hide the difference
                // between "the same as above" and "this one, pinned" — which
                // is the whole of what this row is for.
                self.voice_language.map_or_else(
                    || say.voice_language_same.to_owned(),
                    |language| language.as_str().to_owned(),
                ),
            ),
            Step::MemoryLanguage => (
                say.option_memory_language.to_owned(),
                // Auto is a named answer rather than a blank: "the language you
                // write in" is a decision, and an empty column would read as a
                // setting nobody has got round to.
                self.language
                    .clone()
                    .unwrap_or_else(|| say.language_auto.to_owned()),
            ),
            Step::SardiVoice => (
                fill(say.option_voice, "name", sardi::NAME),
                self.voice.as_str().to_owned(),
            ),
            // Not one of `OPTIONS`, so this cannot be reached from the index.
            // An empty row rather than a panic: a settings page is the last
            // screen that should take the program down with it.
            _ => (String::new(), String::new()),
        }
    }

    /// The screen for the current step, in a window that many rows tall.
    ///
    /// What every driver should call: they are the ones that know how much room
    /// there is, and a screen that does not fit is not a rendering problem but a
    /// question whose answers cannot all be seen. See [`screen::windowed`] for
    /// what gives way to what.
    pub fn render_within(&self, height: usize) -> Vec<Row> {
        windowed(self.render(), height)
    }

    /// The screen for the current step, one entry per line and no window.
    pub fn render(&self) -> Vec<Row> {
        let say = crate::i18n::screens(self.interface);
        let mut lines = vec![Row::new(Role::Brand, BRAND), Row::blank()];
        match self.step {
            Step::AdoptEngram => {
                let found = self
                    .offer
                    .engram
                    .as_ref()
                    .expect("this step only exists when something was found");
                lines.push(Row::new(Role::Heading, say.found_engram));
                lines.push(Row::new(
                    Role::Detail,
                    format!("  {}", found.database.display()),
                ));
                let counts = fill(say.engram_counts, "observations", found.observations);
                let counts = fill(&counts, "sessions", found.sessions);
                let counts = fill(&counts, "prompts", found.prompts);
                let counts = fill(&counts, "relations", found.relations);
                lines.push(Row::new(Role::Detail, format!("  {counts}")));
                lines.push(Row::blank());
                lines.push(Row::new(Role::Heading, say.adopt_question));
                lines.push(Row::blank());
                lines.push(radio(self.cursor == 0, self.adopt, say.adopt_yes));
                lines.push(radio(self.cursor == 1, !self.adopt, say.adopt_no));
            }
            Step::ChooseAgents => {
                lines.push(Row::new(Role::Heading, say.choose_agents));
                lines.push(Row::blank());
                for (index, agent) in self.offer.agents.iter().enumerate() {
                    // The tick already says it is installed. What the row adds
                    // is what unticking would do, because removing a working
                    // setup by moving a cursor and pressing space should not be
                    // a surprise.
                    let label = match (agent.configured, self.chosen.contains(&index)) {
                        (true, false) => {
                            format!("{} — {}", agent.display_name, say.will_be_removed)
                        }
                        (false, true) => {
                            format!("{} — {}", agent.display_name, say.will_be_installed)
                        }
                        _ => agent.display_name.clone(),
                    };
                    lines.push(checkbox(
                        self.cursor == index,
                        self.chosen.contains(&index),
                        &label,
                    ));
                }
            }
            Step::InstallHooks => {
                // Name them: with a mixed selection the answer only reaches
                // some of what was ticked, and saying which avoids the
                // impression that the rest were configured with hooks too.
                let names = self.hook_capable_names().join(", ");
                lines.push(Row::new(
                    Role::Heading,
                    fill(say.hooks_question, "agents", names),
                ));
                lines.push(Row::blank());
                lines.push(radio(self.cursor == 0, self.hooks, say.yes));
                lines.push(radio(self.cursor == 1, !self.hooks, say.hooks_no));
            }
            // The page itself: what can be changed, and what each is set to.
            // Not a question with answers under it — the answers are on the
            // screens behind these rows, and this is the list of doors.
            Step::Options => {
                lines.push(Row::new(Role::Heading, say.options_question));
                lines.push(Row::blank());
                let rows: Vec<(String, String)> =
                    OPTIONS.iter().map(|step| self.option_row(*step)).collect();
                // The values line up in a column, so what is set reads as a
                // list of answers rather than as ragged prose.
                let width = rows
                    .iter()
                    .map(|(label, _)| label.chars().count())
                    .max()
                    .unwrap_or(0);
                for (index, (label, value)) in rows.iter().enumerate() {
                    lines.push(entry(self.cursor == index, label, width, value));
                }
            }
            Step::SardiVoice => {
                lines.push(Row::new(
                    Role::Heading,
                    fill(say.voice_question, "name", sardi::NAME),
                ));
                lines.push(Row::blank());
                for (index, voice) in crate::settings::Voice::ALL.iter().enumerate() {
                    lines.push(radio(
                        self.cursor == index,
                        self.voice == *voice,
                        &format!(
                            "{:<10} {}",
                            voice.as_str(),
                            voice.description(self.interface)
                        ),
                    ));
                }
            }
            Step::InterfaceLanguage => {
                lines.push(Row::new(Role::Heading, say.interface_question));
                lines.push(Row::blank());
                for (index, language) in crate::settings::Interface::ALL.iter().enumerate() {
                    lines.push(radio(
                        self.cursor == index,
                        self.interface == *language,
                        language.as_str(),
                    ));
                }
                lines.push(Row::blank());
                // Says which of the three questions this is. They sit next to
                // each other and all three are about language, and somebody
                // who reads one as another sets the wrong one.
                lines.push(Row::new(Role::Hint, say.interface_hint_first));
                lines.push(Row::new(
                    Role::Hint,
                    fill(say.interface_hint_second, "name", sardi::NAME),
                ));
            }
            Step::SardiLanguage => {
                lines.push(Row::new(
                    Role::Heading,
                    fill(say.voice_language_question, "name", sardi::NAME),
                ));
                lines.push(Row::blank());
                let choices = Self::voice_language_choices();
                // Wide enough for the longest label, which here is a sentence
                // fragment rather than a language name — `{:<10}` fits the
                // twelve and shears "the same as Leteo" in half the languages
                // that have to say it.
                let width = choices
                    .iter()
                    .map(|choice| match choice {
                        None => say.voice_language_same.chars().count(),
                        Some(language) => language.as_str().chars().count(),
                    })
                    .max()
                    .unwrap_or(0);
                for (index, choice) in choices.iter().enumerate() {
                    let label = match choice {
                        None => {
                            let padding = " ".repeat(
                                width.saturating_sub(say.voice_language_same.chars().count()),
                            );
                            format!(
                                "{}{padding}  {}",
                                say.voice_language_same, say.voice_language_same_detail
                            )
                        }
                        Some(language) => language.as_str().to_owned(),
                    };
                    lines.push(radio(
                        self.cursor == index,
                        self.voice_language == *choice,
                        &label,
                    ));
                }
                lines.push(Row::blank());
                // Why this setting exists at all, which is not guessable from
                // the question: the character's lines are emitted by hooks into
                // an agent's conversation, so they land beside whatever
                // language that conversation is being held in — somewhere the
                // rest of Leteo's screens never appear.
                lines.push(Row::new(
                    Role::Hint,
                    fill(say.voice_language_hint, "name", sardi::NAME),
                ));
            }
            Step::MemoryLanguage => {
                lines.push(Row::new(Role::Heading, say.memory_language_question));
                lines.push(Row::blank());
                for (index, choice) in self.language_choices.iter().enumerate() {
                    let label = match choice {
                        None => format!("{:<10} {}", say.language_auto, say.language_auto_detail),
                        Some(language) => {
                            format!("{language:<10} {}", say.language_pinned_detail)
                        }
                    };
                    lines.push(radio(
                        self.cursor == index,
                        self.language == *choice,
                        &label,
                    ));
                }
                lines.push(Row::blank());
                // The warning nobody would think to ask for.
                //
                // This answers what happens next, not what is being chosen: a
                // language governs what is *written from now on*. Nothing
                // rewrites what is already stored, so a store with memories in
                // it becomes a store in two languages — and a search only
                // reaches the half it is asked in. Somebody who learns that
                // three weeks later has three weeks of memories they cannot
                // find, and no reason to connect the two.
                if self.offer.store_has_memories {
                    lines.push(Row::new(Role::Hint, say.language_kept_warning));
                    lines.push(Row::new(Role::Hint, say.language_split_warning_first));
                    lines.push(Row::new(Role::Hint, say.language_split_warning_second));
                }
                lines.push(Row::new(Role::Hint, say.language_other_hint));
            }
            // Nothing left to ask. The legend below would offer keys that do
            // nothing, so these steps say what is happening instead.
            Step::Ready => {
                lines.push(Row::new(
                    Role::Heading,
                    format!("  {}", sardi::reading(self.voice_interface())),
                ));
                return lines;
            }
            Step::Cancelled => {
                lines.push(Row::new(Role::Hint, say.nothing_changed));
                return lines;
            }
        }
        lines.push(Row::blank());
        // Two flows, two sets of keys. Nothing is ticked on the options page,
        // there is nothing to continue to, and esc there steps back rather than
        // abandoning anything — so the setup legend would be wrong about all
        // three on the one page somebody opens without being led there.
        lines.push(Row::new(
            Role::Hint,
            match self.flow {
                Flow::Agents => say.legend.to_owned(),
                Flow::Options => format!("  {}", say.keys_options),
            },
        ));
        lines
    }

    /// Carries out what was decided.
    pub fn apply(&self, report: &mut impl std::io::Write) -> Result<Outcome> {
        let say = crate::i18n::screens(self.interface);
        let mut outcome = Outcome::default();
        if self.step == Step::Cancelled {
            outcome.cancelled = true;
            return Ok(outcome);
        }

        if self.adopt
            && let Some(found) = &self.offer.engram
        {
            match engram::adopt(&found.database, &self.offer.database, false) {
                Ok(adoption) => {
                    let count = adoption.adopted.map_or(0, |counts| counts.observations);
                    outcome.adopted = Some(count);
                    writeln!(
                        report,
                        "  {}",
                        sardi::adopted(self.voice_interface(), count)
                    )?;
                }
                // A failed adoption is worth reporting, not worth abandoning
                // the agent setup over.
                Err(error) => writeln!(report, "{}", fill(say.could_not_adopt, "error", error))?,
            }
        }

        // Written whatever the answer, including the default: a file that only
        // appears once somebody dissents leaves no way to tell "never asked"
        // from "asked and chose the loud one", and the next release changing
        // its default would silently move them.
        let mut saved = false;
        if let Some(data_dir) = self.offer.database.parent() {
            // Every field, and each of them either an answer or what was found
            // on disk — never a fresh `Settings`, which is how a setting
            // nobody was asked about gets silently erased on the next run.
            //
            // `interface_setting` rather than the resolved `interface`, and
            // that distinction is the whole point of keeping both. Unset means
            // "follow this machine", which is a live answer that changes with
            // the machine; `interface` is what that resolves to *today*.
            // Writing the resolved value back would turn "follow the machine"
            // into "the machine's language the day this was installed", pinned
            // by a flow that no longer even asks the question.
            let settings = crate::settings::Settings {
                voice: self.voice,
                language: self.language.clone(),
                interface: self.interface_setting,
                // Unresolved for the same reason, one setting along: `None`
                // here means "whatever Leteo speaks", which goes on being true
                // as that changes. Writing what it resolves to today would pin
                // the voice to a language nobody chose for it.
                voice_language: self.voice_language,
                // Carried, not asked. This flow writes the whole file, so a
                // setting it does not offer would be erased by somebody
                // changing their language here — and `context_size` is set
                // from the command line rather than in this menu.
                context_size: crate::settings::load(data_dir).context_size,
            };
            match crate::settings::save(data_dir, &settings) {
                Ok(()) => {
                    outcome.voice = self.voice;
                    saved = true;
                }
                // Worth reporting, not worth abandoning the agent setup over.
                Err(error) => writeln!(report, "{}", fill(say.could_not_save, "error", error))?,
            }
        }
        // Everything below is about agents, and the options page asked nothing
        // about one. Stopping here rather than trusting the difference to come
        // out empty: `self.hooks` and the ticks below are what the constructor
        // loaded, not what anybody answered, and the arm for an unchanged tick
        // acts on `self.hooks`.
        if self.flow == Flow::Options {
            // And it says so. Leaving the page is what writes the file, so a
            // page that closed in silence would leave somebody with no way to
            // tell a saved setting from one that never reached the disk — the
            // failure above being the case that matters.
            if saved {
                writeln!(report, "  {}", say.preferences_saved)?;
            }
            return Ok(outcome);
        }
        // The ticks are the wanted state, not a list of things to do, so what
        // gets applied is the difference between them and what is on disk.
        for (index, agent) in self.offer.agents.iter().enumerate() {
            let wanted = self.chosen.contains(&index);
            // Hooks only for the agents that have somewhere to put them. Asking
            // for them elsewhere is an error, and it would cost that agent its
            // setup entirely over a part of the answer that never applied to it.
            //
            // And hooks a plugin bundle already registers are hooks nobody has
            // to install: `setup` refuses to write a second copy, which is the
            // right answer to a typed command and the wrong one to a flow that
            // gets re-run. Asked per agent, because the bundles are per agent.
            let with_hooks = self.hooks
                && agent.supports_hooks
                && !crate::setup::plugin_registers_hooks(&agent.slug, &self.offer.probe);
            let options = SetupOptions {
                install_hooks: with_hooks,
                ..self.offer.probe.clone()
            };
            match (wanted, agent.configured) {
                (true, false) => {
                    match crate::setup::setup(&agent.slug, &options) {
                        Ok(_) => {
                            outcome.configured.push(agent.slug.clone());
                            writeln!(
                                report,
                                "  {}",
                                sardi::configured(
                                    self.voice_interface(),
                                    &agent.display_name,
                                    with_hooks
                                )
                            )?;
                        }
                        // One agent failing should not cost the others theirs.
                        Err(error) => writeln!(
                            report,
                            "{}",
                            fill(
                                &fill(say.could_not_configure, "agent", &agent.display_name),
                                "error",
                                error
                            )
                        )?,
                    }
                }
                // Already registered — which says the MCP server is in the
                // file, and says nothing at all about hooks. Somebody who chose
                // "MCP tools only" the first time and has now ticked hooks was
                // asked a question whose answer went nowhere, because this arm
                // did nothing. Setup is an upsert, so running it again costs a
                // read when everything already matches; `changed_files` is what
                // decides whether there is anything worth reporting.
                (true, true) if with_hooks => match crate::setup::setup(&agent.slug, &options) {
                    Ok(result) if result.changed_files() > 0 => {
                        outcome.configured.push(agent.slug.clone());
                        writeln!(
                            report,
                            "  {}",
                            sardi::configured(
                                self.voice_interface(),
                                &agent.display_name,
                                with_hooks
                            )
                        )?;
                    }
                    Ok(_) => {}
                    Err(error) => writeln!(
                        report,
                        "{}",
                        fill(
                            &fill(say.could_not_configure, "agent", &agent.display_name),
                            "error",
                            error
                        )
                    )?,
                },
                (false, true) => match crate::setup::uninstall(&agent.slug, &self.offer.probe) {
                    Ok(_) => {
                        outcome.removed.push(agent.slug.clone());
                        writeln!(
                            report,
                            "{}",
                            fill(say.removed_from, "agent", &agent.display_name)
                        )?;
                    }
                    Err(error) => writeln!(
                        report,
                        "{}",
                        fill(
                            &fill(say.could_not_remove, "agent", &agent.display_name),
                            "error",
                            error
                        )
                    )?,
                },
                _ => {}
            }
        }

        if outcome.adopted.is_none() && outcome.configured.is_empty() && outcome.removed.is_empty()
        {
            writeln!(report, "  {}", sardi::idle(self.voice_interface()))?;
        } else if !outcome.configured.is_empty() || !outcome.removed.is_empty() {
            writeln!(report, "{}", say.restart_them)?;
        }
        Ok(outcome)
    }
}

#[cfg(test)]
mod tests;
