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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    AdoptEngram,
    ChooseAgents,
    InstallHooks,
    Options,
    InterfaceLanguage,
    SardiLanguage,
    MemoryLanguage,
    SardiVoice,
    Ready,
    Cancelled,
}

const OPTIONS: [Step; 4] = [
    Step::InterfaceLanguage,
    Step::SardiLanguage,
    Step::MemoryLanguage,
    Step::SardiVoice,
];

impl Step {
    #[cfg(test)]
    fn is_global(self) -> bool {
        matches!(
            self,
            Self::InterfaceLanguage | Self::SardiLanguage | Self::MemoryLanguage | Self::SardiVoice
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Flow {
    Agents,
    Options,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub adopted: Option<i64>,
    pub configured: Vec<String>,
    pub removed: Vec<String>,
    pub cancelled: bool,
    pub voice: crate::settings::Voice,
}

#[derive(Debug)]
pub struct Wizard {
    offer: Offer,
    step: Step,
    cursor: usize,
    chosen: Vec<usize>,
    adopt: bool,
    hooks: bool,
    voice: crate::settings::Voice,
    language: Option<String>,
    language_choices: Vec<Option<String>>,
    interface: crate::settings::Interface,
    interface_setting: Option<crate::settings::Interface>,
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
        let step = if offer.engram.is_some() {
            Step::AdoptEngram
        } else {
            Step::ChooseAgents
        };
        let chosen = offer
            .agents
            .iter()
            .enumerate()
            .filter(|(_, agent)| agent.configured)
            .map(|(index, _)| index)
            .collect();
        let settings = crate::settings::load_beside(&offer.database);
        let voice = settings.voice;
        let language = settings.language.clone();
        let language_choices = {
            let mut choices =
                crate::settings::language_choices(crate::settings::system_language().as_deref());
            if let Some(current) = &language
                && !choices
                    .iter()
                    .any(|choice| choice.as_deref() == Some(current.as_str()))
            {
                choices.push(Some(current.clone()));
            }
            choices
        };
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

    pub fn preferences(offer: Offer) -> Self {
        let mut wizard = Self::new(offer);
        wizard.flow = Flow::Options;
        wizard.adopt = false;
        wizard.cursor = 0;
        wizard.step = Step::Options;
        wizard
    }

    pub fn step(&self) -> Step {
        self.step
    }

    pub fn interface(&self) -> crate::settings::Interface {
        self.interface
    }

    pub fn voice_interface(&self) -> crate::settings::Interface {
        self.voice_language.unwrap_or(self.interface)
    }

    fn voice_language_choices() -> Vec<Option<crate::settings::Interface>> {
        std::iter::once(None)
            .chain(crate::settings::Interface::ALL.map(Some))
            .collect()
    }

    pub fn is_finished(&self) -> bool {
        matches!(self.step, Step::Ready | Step::Cancelled)
    }

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

    pub fn up(&mut self) -> bool {
        let count = self.line_count();
        if count == 0 {
            return false;
        }
        let next = (self.cursor + count - 1) % count;
        let moved = next != self.cursor;
        self.cursor = next;
        moved
    }

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
            Step::InterfaceLanguage => {
                let next = crate::settings::Interface::ALL[self.cursor];
                let changed = self.interface != next;
                self.interface = next;
                self.interface_setting = Some(next);
                changed
            }
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
            Step::Options | Step::Ready | Step::Cancelled => false,
        }
    }

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
                if self.any_chosen_supports_hooks() {
                    self.cursor = if self.hooks { 0 } else { 1 };
                    Step::InstallHooks
                } else {
                    Step::Ready
                }
            }
            Step::InstallHooks => Step::Ready,
            Step::Options => self.open(OPTIONS[self.cursor]),
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

    fn back_to_index(&mut self, from: Step) -> Step {
        self.cursor = OPTIONS.iter().position(|row| *row == from).unwrap_or(0);
        Step::Options
    }

    fn interface_cursor(&self) -> usize {
        crate::settings::Interface::ALL
            .iter()
            .position(|language| *language == self.interface)
            .unwrap_or(0)
    }

    fn voice_language_cursor(&self) -> usize {
        Self::voice_language_choices()
            .iter()
            .position(|choice| *choice == self.voice_language)
            .unwrap_or(0)
    }

    fn language_cursor(&self) -> usize {
        self.language_choices
            .iter()
            .position(|choice| *choice == self.language)
            .unwrap_or(0)
    }

    fn voice_cursor(&self) -> usize {
        crate::settings::Voice::ALL
            .iter()
            .position(|voice| *voice == self.voice)
            .unwrap_or(0)
    }

    pub fn back(&mut self) -> bool {
        let before = self.step;
        self.step = match before {
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
            Step::InterfaceLanguage
            | Step::SardiLanguage
            | Step::MemoryLanguage
            | Step::SardiVoice => self.back_to_index(before),
            Step::Options => Step::Ready,
            other => other,
        };
        self.step != before
    }

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

    pub fn cancel(&mut self) -> bool {
        if self.flow == Flow::Options {
            return self.back();
        }
        let changed = self.step != Step::Cancelled;
        self.step = Step::Cancelled;
        changed
    }

    fn option_row(&self, step: Step) -> (String, String) {
        let say = crate::i18n::screens(self.interface);
        match step {
            Step::InterfaceLanguage => (
                say.option_interface.to_owned(),
                self.interface.as_str().to_owned(),
            ),
            Step::SardiLanguage => (
                fill(say.option_voice_language, "name", sardi::NAME),
                self.voice_language.map_or_else(
                    || say.voice_language_same.to_owned(),
                    |language| language.as_str().to_owned(),
                ),
            ),
            Step::MemoryLanguage => (
                say.option_memory_language.to_owned(),
                self.language
                    .clone()
                    .unwrap_or_else(|| say.language_auto.to_owned()),
            ),
            Step::SardiVoice => (
                fill(say.option_voice, "name", sardi::NAME),
                self.voice.as_str().to_owned(),
            ),
            _ => (String::new(), String::new()),
        }
    }

    pub fn render_within(&self, height: usize) -> Vec<Row> {
        windowed(self.render(), height)
    }

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
                let names = self.hook_capable_names().join(", ");
                lines.push(Row::new(
                    Role::Heading,
                    fill(say.hooks_question, "agents", names),
                ));
                lines.push(Row::blank());
                lines.push(radio(self.cursor == 0, self.hooks, say.yes));
                lines.push(radio(self.cursor == 1, !self.hooks, say.hooks_no));
            }
            Step::Options => {
                lines.push(Row::new(Role::Heading, say.options_question));
                lines.push(Row::blank());
                let rows: Vec<(String, String)> =
                    OPTIONS.iter().map(|step| self.option_row(*step)).collect();
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
                if self.offer.store_has_memories {
                    lines.push(Row::new(Role::Hint, say.language_kept_warning));
                    lines.push(Row::new(Role::Hint, say.language_split_warning_first));
                    lines.push(Row::new(Role::Hint, say.language_split_warning_second));
                }
                lines.push(Row::new(Role::Hint, say.language_other_hint));
            }
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
        lines.push(Row::new(
            Role::Hint,
            match self.flow {
                Flow::Agents => say.legend.to_owned(),
                Flow::Options => format!("  {}", say.keys_options),
            },
        ));
        lines
    }

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
                Err(error) => writeln!(report, "{}", fill(say.could_not_adopt, "error", error))?,
            }
        }

        let mut saved = false;
        if let Some(data_dir) = self.offer.database.parent() {
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
                context_size: crate::settings::load(data_dir).context_size,
            };
            match crate::settings::save(data_dir, &settings) {
                Ok(()) => {
                    outcome.voice = self.voice;
                    saved = true;
                }
                Err(error) => writeln!(report, "{}", fill(say.could_not_save, "error", error))?,
            }
        }
        // Everything below is about agents, and the options page asked nothing
        // about one. Stopping here rather than trusting the difference to come
        // out empty: `self.hooks` and the ticks below are what the constructor
        // loaded, not what anybody answered, and the arm for an unchanged tick
        // acts on `self.hooks`.
        if self.flow == Flow::Options {
            if saved {
                writeln!(report, "  {}", say.preferences_saved)?;
            }
            return Ok(outcome);
        }
        for (index, agent) in self.offer.agents.iter().enumerate() {
            let wanted = self.chosen.contains(&index);
            let with_hooks = self.hooks
                && agent.supports_hooks
                && !crate::setup::plugin_registers_hooks(&agent.slug, &self.offer.probe)
                && !crate::setup::hook_runner_switched_off(&agent.slug, &self.offer.probe);
            let options = SetupOptions {
                install_hooks: with_hooks,
                ..self.offer.probe.clone()
            };
            match (wanted, agent.configured) {
                (true, false) => match crate::setup::setup(&agent.slug, &options) {
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
