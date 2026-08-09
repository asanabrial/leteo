//! Every sentence Leteo puts on a screen, in one place per language.
//!
//! # Why a struct of literals rather than a resource file
//!
//! The usual answer in Rust is `fluent` — Mozilla's `.ftl` files, loaded and
//! parsed at runtime — or `rust-i18n`, which reads YAML at build time and gives
//! you `t!("some.key")`. Both decouple the words from the code, which is the
//! right instinct and the reason this module exists at all: prose sitting
//! inline in render code cannot be reviewed, cannot be handed to a translator,
//! and gets missed one line at a time.
//!
//! What they cost is the thing that catches the mistake. `t!("some.key")` with
//! a key that no longer exists compiles; it fails when somebody opens that
//! screen, and it fails by printing the key. Fluent buys real plural rules and
//! bidi handling for that price, which is worth it for a browser shipping
//! ninety locales and translated by people who do not build the project.
//!
//! Here the whole surface is one struct. Adding a language is adding one
//! `const` of this type, and the compiler will not accept it until every field
//! has a sentence — so a screen cannot be half translated, and a field nobody
//! filled in is a build error rather than a blank line in front of somebody.
//! One file to read, one file to translate, and no key that can go stale.
//!
//! The trade is real and it has a direction: this stops paying at the point
//! where translators are not the people editing Rust, or where plural rules
//! stop being "one or not one" — Polish needs three forms and Arabic six, and
//! neither is expressible in a table of fixed strings. Fluent is the answer
//! then, and moving to it is a rewrite of this one module rather than of every
//! screen, which is most of why the text is gathered here now.
//!
//! # What belongs here
//!
//! Fixed labels, headings, hints and the words around a number. Anything whose
//! wording changes with a count belongs in [`crate::sardi`] instead, where the
//! plural forms are chosen — that module is this one's twin for the voice, and
//! the split is between "a label" and "a sentence about how many".
//!
//! Nothing an agent parses. MCP replies and the CLI's JSON are a contract, and
//! a contract in the reader's language is not a contract.

use crate::settings::Interface;

/// The words for one language.
///
/// Fields carrying `{...}` are templates, filled with [`fill`]. They are named
/// rather than positional so a translation can move the value within the
/// sentence, which word order in some languages requires and `{}` forbids.
pub struct Screens {
    // The setup wizard, in the order the screens appear.
    pub found_engram: &'static str,
    /// `{observations}`, `{sessions}`, `{prompts}`, `{relations}`.
    pub engram_counts: &'static str,
    pub adopt_question: &'static str,
    pub adopt_yes: &'static str,
    pub adopt_no: &'static str,
    pub choose_agents: &'static str,
    pub will_be_removed: &'static str,
    pub will_be_installed: &'static str,
    /// `{agents}` — the chosen agents that can actually take hooks.
    pub hooks_question: &'static str,
    pub yes: &'static str,
    pub hooks_no: &'static str,
    /// `{name}` — the character's name, which is never translated.
    pub voice_question: &'static str,
    pub voice_all: &'static str,
    pub voice_reminders: &'static str,
    pub voice_quiet: &'static str,
    pub interface_question: &'static str,
    pub interface_hint_first: &'static str,
    /// `{name}` — the character's name, which is never translated.
    pub interface_hint_second: &'static str,
    /// `{name}`.
    pub voice_language_question: &'static str,
    /// What the voice follows when it has not been given a language of its own.
    /// A named answer rather than a blank: it keeps following as Leteo's own
    /// language changes, which is a decision and not an omission.
    pub voice_language_same: &'static str,
    pub voice_language_same_detail: &'static str,
    /// `{name}` — why this setting exists at all, said where it is chosen.
    pub voice_language_hint: &'static str,
    pub memory_language_question: &'static str,
    pub language_auto: &'static str,
    pub language_auto_detail: &'static str,
    pub language_pinned_detail: &'static str,
    pub language_kept_warning: &'static str,
    pub language_split_warning_first: &'static str,
    pub language_split_warning_second: &'static str,
    pub language_other_hint: &'static str,
    pub nothing_changed: &'static str,
    pub legend: &'static str,

    // The options page: an index of what can be changed, each row carrying the
    // answer in force, and one screen behind each row. Not a wizard — nobody
    // opens it to be walked through three questions, they open it to change the
    // one thing they came for.
    pub options_question: &'static str,
    pub option_interface: &'static str,
    /// `{name}`.
    pub option_voice_language: &'static str,
    pub option_memory_language: &'static str,
    /// `{name}` — the character's name, which is never translated.
    pub option_voice: &'static str,
    /// What leaving the page reports, so that a screen which saves on the way
    /// out says so rather than closing in silence.
    pub preferences_saved: &'static str,

    // What applying reports. Failures included: these are read by the person
    // who has to fix them, so they are the last thing that should stay in a
    // language they did not choose.
    /// `{error}`.
    pub could_not_adopt: &'static str,
    /// `{error}`.
    pub could_not_save: &'static str,
    /// `{agent}`, `{error}`.
    pub could_not_configure: &'static str,
    /// `{agent}`, `{error}`.
    pub could_not_remove: &'static str,
    /// `{agent}`.
    pub removed_from: &'static str,
    pub restart_them: &'static str,

    // The dashboard with nothing in it yet.
    pub empty_dashboard_what_happens: &'static str,
    pub empty_dashboard_keys: &'static str,
    pub setup_cancelled: &'static str,
    /// `{error}`.
    pub setup_failed: &'static str,

    // The dashboard's panels and the words around its numbers.
    pub panel_setup: &'static str,
    pub panel_dashboard: &'static str,
    pub panel_detail: &'static str,
    pub panel_content: &'static str,
    pub panel_session: &'static str,
    pub panel_timeline: &'static str,
    pub panel_context: &'static str,
    pub panel_session_timeline: &'static str,
    pub panel_help: &'static str,
    pub panel_options: &'static str,
    pub panel_cloud: &'static str,
    pub panel_filters: &'static str,
    /// `{count}`.
    pub panel_filters_count: &'static str,
    /// `{count}`.
    pub panel_recorded: &'static str,
    pub list_observations: &'static str,
    pub list_sessions: &'static str,
    pub list_prompts: &'static str,
    /// `{project}`.
    pub scope_one_project: &'static str,
    /// `{count}`.
    pub scope_many_projects: &'static str,
    /// `{query}`.
    pub list_matching: &'static str,
    /// `{position}`, `{total}`.
    pub list_position: &'static str,
    pub search_placeholder: &'static str,

    // The counter headings and the page name in the header. Upper case is the
    // style, not the content: `to_uppercase` is applied where they are drawn,
    // so a language whose casing rules differ is not fought here.
    pub stat_observations: &'static str,
    pub stat_sessions: &'static str,
    pub stat_prompts: &'static str,
    pub page_home: &'static str,
    pub page_dashboard: &'static str,
    pub page_detail: &'static str,
    pub page_session: &'static str,
    pub page_timeline: &'static str,
    pub page_setup: &'static str,
    pub page_cloud: &'static str,
    pub page_help: &'static str,
    pub page_options: &'static str,

    // Nothing to show. Each names what is missing rather than saying "empty",
    // because the lists sit side by side and a shared word says nothing.
    pub no_observations: &'static str,
    pub no_sessions: &'static str,
    pub no_prompts: &'static str,
    pub no_projects: &'static str,
    pub no_observation_selected: &'static str,
    pub no_session_selected: &'static str,
    pub no_timeline_loaded: &'static str,
    pub no_summary: &'static str,
    pub nothing_to_search: &'static str,
    pub cancelled: &'static str,

    // Field labels on the detail and session pages.
    pub field_type: &'static str,
    pub field_project: &'static str,
    pub field_scope: &'static str,
    pub field_session: &'static str,
    pub field_topic: &'static str,
    pub field_started: &'static str,
    pub field_ended: &'static str,
    pub field_summary: &'static str,
    pub session_active: &'static str,
    /// `{session}`.
    pub timeline_session: &'static str,
    /// `{id}`, `{title}`, `{total}`.
    pub timeline_focus: &'static str,
    pub timeline_focus_marker: &'static str,

    // The cloud page.
    pub cloud_server: &'static str,
    pub cloud_background: &'static str,
    pub cloud_replicating: &'static str,
    pub cloud_enrolled: &'static str,
    pub cloud_queued: &'static str,
    pub cloud_deferred: &'static str,
    pub cloud_not_configured: &'static str,
    pub cloud_enabled: &'static str,
    pub cloud_disabled: &'static str,
    pub cloud_none: &'static str,
    /// What a count reads as when the store could not be opened. Never `0` —
    /// "could not read" and "nothing there" are different answers.
    pub cloud_unknown: &'static str,
    /// `{count}`.
    pub cloud_mutations: &'static str,
    /// `{deferred}`, `{dead}`.
    pub cloud_deferred_dead: &'static str,
    /// `{reason}`.
    pub cloud_unreadable: &'static str,
    pub cloud_configure_hint: &'static str,
    pub cloud_state: &'static str,
    /// `{count}`.
    pub cloud_failures: &'static str,
    /// `{until}`.
    pub cloud_backoff: &'static str,

    // The home menu.
    pub menu_start_setup: &'static str,
    pub menu_dashboard: &'static str,
    pub menu_cloud: &'static str,
    pub menu_options: &'static str,
    pub menu_help: &'static str,
    pub menu_quit: &'static str,
    pub menu_uninstall: &'static str,
    pub uninstall_heading: &'static str,
    /// `{count}`.
    pub uninstall_agents: &'static str,
    pub uninstall_warning: &'static str,

    // Deleting, which asks first and says what it cannot undo.
    /// `{id}`.
    pub delete_memory: &'static str,
    /// `{id}`.
    pub delete_prompt: &'static str,
    /// `{id}`.
    pub delete_session: &'static str,
    /// `{name}`.
    pub delete_project: &'static str,
    pub delete_permanent_warning: &'static str,
    pub delete_prompts_warning: &'static str,
    pub delete_recoverable: &'static str,
    pub gone_permanently: &'static str,
    pub gone: &'static str,
    /// `{count}`.
    pub count_memories: &'static str,
    /// `{count}`.
    pub count_sessions: &'static str,
    /// `{count}`.
    pub count_prompts: &'static str,
    /// `{count}`.
    pub copied_to_clipboard: &'static str,
    pub data_refreshed: &'static str,
    /// `{id}`, `{gone}`.
    pub deleted_memory: &'static str,
    /// `{id}`.
    pub deleted_prompt: &'static str,
    /// `{id}`, `{gone}`, `{memories}`, `{prompts}`.
    pub deleted_session: &'static str,
    /// `{name}`, `{gone}`, `{memories}`, `{sessions}`, `{prompts}`.
    pub deleted_project: &'static str,
    /// `{count}`.
    pub sessions_kept: &'static str,
    /// `{query}`, `{observations}`, `{sessions}`, `{prompts}`.
    pub refreshed_query: &'static str,

    // The key legend along the bottom, one per page.
    pub keys_confirm: &'static str,
    pub keys_confirm_footer: &'static str,
    /// Inside the confirmation window rather than along the bottom, and spaced
    /// to sit in a box rather than in a strip.
    pub keys_confirm_window: &'static str,
    pub keys_home: &'static str,
    pub keys_query: &'static str,
    pub keys_filters: &'static str,
    pub keys_dashboard_searching: &'static str,
    pub keys_dashboard_sessions: &'static str,
    pub keys_dashboard_prompts: &'static str,
    pub keys_dashboard: &'static str,
    pub keys_detail: &'static str,
    pub keys_session: &'static str,
    pub keys_timeline: &'static str,
    pub keys_setup: &'static str,
    pub keys_options: &'static str,
    pub keys_cloud: &'static str,
    pub keys_help: &'static str,

    /// The whole help page, as lines.
    ///
    /// One block rather than forty fields. It is the only screen that is a
    /// document instead of a set of labels, and the columns only line up if
    /// whoever translates it can see the whole thing at once.
    pub help_body: &'static str,
}

mod ca;
mod de;
mod en;
mod es;
mod eu;
mod fr;
mod gl;
mod it;
mod nl;
mod pl;
mod pt;
mod sv;

/// The words for a language.
pub fn screens(language: Interface) -> &'static Screens {
    match language {
        Interface::English => &en::SCREENS,
        Interface::Spanish => &es::SCREENS,
        Interface::Portuguese => &pt::SCREENS,
        Interface::French => &fr::SCREENS,
        Interface::German => &de::SCREENS,
        Interface::Italian => &it::SCREENS,
        Interface::Catalan => &ca::SCREENS,
        Interface::Galician => &gl::SCREENS,
        Interface::Basque => &eu::SCREENS,
        Interface::Dutch => &nl::SCREENS,
        Interface::Polish => &pl::SCREENS,
        Interface::Swedish => &sv::SCREENS,
    }
}

/// Replaces `{name}` in a template with a value.
///
/// A named placeholder rather than `format!`, which needs a literal it can see
/// at compile time and so cannot take a sentence chosen at runtime. Chaining
/// these is how a template with several values is filled.
pub fn fill(template: &str, name: &str, value: impl std::fmt::Display) -> String {
    template.replace(&format!("{{{name}}}"), &value.to_string())
}

/// The first `{word}` left standing in something already painted, if any.
///
/// The inverse of [`fill`], and it lives beside it for that reason: two test
/// modules were each carrying their own copy — one for the wizard's screens and
/// one for the dashboard's — which is two statements of what a placeholder
/// looks like, free to disagree about it.
///
/// Written out rather than reached for with a regex crate: this is looking for
/// one shape, and the shape is two characters and what sits between them.
#[cfg(test)]
pub(crate) fn unfilled_placeholder(painted: &str) -> Option<String> {
    let mut rest = painted;
    while let Some(open) = rest.find('{') {
        let after = &rest[open + 1..];
        let close = after.find('}')?;
        let inside = &after[..close];
        if !inside.is_empty()
            && inside
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '_')
        {
            return Some(format!("{{{inside}}}"));
        }
        rest = &after[close..];
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every field of every language, so the checks below cannot pass by
    /// looking at a subset. Built by hand rather than by reflection, which Rust
    /// has none of — the compiler already guarantees no field is *missing*, and
    /// what this list guards is that no field is left untranslated.
    fn all_fields(screens: &'static Screens) -> Vec<(&'static str, &'static str)> {
        // Destructured rather than read field by field. A `let` pattern with no
        // `..` is exhaustive, so a field added to `Screens` and forgotten here
        // is a compile error — where before it was a row that simply never got
        // checked for a placeholder, a blank, or an untranslated copy.
        let Screens {
            found_engram,
            engram_counts,
            adopt_question,
            adopt_yes,
            adopt_no,
            choose_agents,
            will_be_removed,
            will_be_installed,
            hooks_question,
            yes,
            hooks_no,
            voice_question,
            voice_all,
            voice_reminders,
            voice_quiet,
            interface_question,
            interface_hint_first,
            interface_hint_second,
            voice_language_question,
            voice_language_same,
            voice_language_same_detail,
            voice_language_hint,
            memory_language_question,
            language_auto,
            language_auto_detail,
            language_pinned_detail,
            language_kept_warning,
            language_split_warning_first,
            language_split_warning_second,
            language_other_hint,
            nothing_changed,
            legend,
            options_question,
            option_interface,
            option_voice_language,
            option_memory_language,
            option_voice,
            preferences_saved,
            could_not_adopt,
            could_not_save,
            could_not_configure,
            could_not_remove,
            removed_from,
            restart_them,
            empty_dashboard_keys,
            empty_dashboard_what_happens,
            setup_cancelled,
            setup_failed,
            panel_setup,
            panel_dashboard,
            panel_detail,
            panel_content,
            panel_session,
            panel_timeline,
            panel_context,
            panel_session_timeline,
            panel_help,
            panel_options,
            panel_cloud,
            panel_filters,
            panel_filters_count,
            panel_recorded,
            list_observations,
            list_sessions,
            list_prompts,
            scope_one_project,
            scope_many_projects,
            list_matching,
            list_position,
            search_placeholder,
            stat_observations,
            stat_sessions,
            stat_prompts,
            page_home,
            page_dashboard,
            page_detail,
            page_session,
            page_timeline,
            page_setup,
            page_cloud,
            page_help,
            page_options,
            no_observations,
            no_sessions,
            no_prompts,
            no_projects,
            no_observation_selected,
            no_session_selected,
            no_timeline_loaded,
            no_summary,
            nothing_to_search,
            cancelled,
            field_type,
            field_project,
            field_scope,
            field_session,
            field_topic,
            field_started,
            field_ended,
            field_summary,
            session_active,
            timeline_session,
            timeline_focus,
            timeline_focus_marker,
            cloud_server,
            cloud_background,
            cloud_replicating,
            cloud_enrolled,
            cloud_queued,
            cloud_deferred,
            cloud_not_configured,
            cloud_enabled,
            cloud_disabled,
            cloud_none,
            cloud_unknown,
            cloud_mutations,
            cloud_deferred_dead,
            cloud_unreadable,
            cloud_configure_hint,
            cloud_state,
            cloud_failures,
            cloud_backoff,
            menu_start_setup,
            menu_dashboard,
            menu_cloud,
            menu_options,
            menu_help,
            menu_quit,
            menu_uninstall,
            uninstall_heading,
            uninstall_agents,
            uninstall_warning,
            delete_memory,
            delete_prompt,
            delete_session,
            delete_project,
            delete_permanent_warning,
            delete_prompts_warning,
            delete_recoverable,
            gone_permanently,
            gone,
            count_memories,
            count_sessions,
            count_prompts,
            copied_to_clipboard,
            data_refreshed,
            deleted_memory,
            deleted_prompt,
            deleted_session,
            deleted_project,
            sessions_kept,
            refreshed_query,
            keys_confirm,
            keys_confirm_footer,
            keys_confirm_window,
            keys_home,
            keys_query,
            keys_filters,
            keys_dashboard_searching,
            keys_dashboard_sessions,
            keys_dashboard_prompts,
            keys_dashboard,
            keys_detail,
            keys_session,
            keys_timeline,
            keys_setup,
            keys_options,
            keys_cloud,
            keys_help,
            help_body,
        } = screens;
        vec![
            ("found_engram", found_engram),
            ("engram_counts", engram_counts),
            ("adopt_question", adopt_question),
            ("adopt_yes", adopt_yes),
            ("adopt_no", adopt_no),
            ("choose_agents", choose_agents),
            ("will_be_removed", will_be_removed),
            ("will_be_installed", will_be_installed),
            ("hooks_question", hooks_question),
            ("yes", yes),
            ("hooks_no", hooks_no),
            ("voice_question", voice_question),
            ("voice_all", voice_all),
            ("voice_reminders", voice_reminders),
            ("voice_quiet", voice_quiet),
            ("interface_question", interface_question),
            ("interface_hint_first", interface_hint_first),
            ("interface_hint_second", interface_hint_second),
            ("voice_language_question", voice_language_question),
            ("voice_language_same", voice_language_same),
            ("voice_language_same_detail", voice_language_same_detail),
            ("voice_language_hint", voice_language_hint),
            ("memory_language_question", memory_language_question),
            ("language_auto", language_auto),
            ("language_auto_detail", language_auto_detail),
            ("language_pinned_detail", language_pinned_detail),
            ("language_kept_warning", language_kept_warning),
            ("language_split_warning_first", language_split_warning_first),
            (
                "language_split_warning_second",
                language_split_warning_second,
            ),
            ("language_other_hint", language_other_hint),
            ("nothing_changed", nothing_changed),
            ("legend", legend),
            ("options_question", options_question),
            ("option_interface", option_interface),
            ("option_voice_language", option_voice_language),
            ("option_memory_language", option_memory_language),
            ("option_voice", option_voice),
            ("preferences_saved", preferences_saved),
            ("could_not_adopt", could_not_adopt),
            ("could_not_save", could_not_save),
            ("could_not_configure", could_not_configure),
            ("could_not_remove", could_not_remove),
            ("removed_from", removed_from),
            ("restart_them", restart_them),
            ("empty_dashboard_keys", empty_dashboard_keys),
            ("empty_dashboard_what_happens", empty_dashboard_what_happens),
            ("setup_cancelled", setup_cancelled),
            ("setup_failed", setup_failed),
            ("panel_setup", panel_setup),
            ("panel_dashboard", panel_dashboard),
            ("panel_detail", panel_detail),
            ("panel_content", panel_content),
            ("panel_session", panel_session),
            ("panel_timeline", panel_timeline),
            ("panel_context", panel_context),
            ("panel_session_timeline", panel_session_timeline),
            ("panel_help", panel_help),
            ("panel_options", panel_options),
            ("panel_cloud", panel_cloud),
            ("panel_filters", panel_filters),
            ("panel_filters_count", panel_filters_count),
            ("panel_recorded", panel_recorded),
            ("list_observations", list_observations),
            ("list_sessions", list_sessions),
            ("list_prompts", list_prompts),
            ("scope_one_project", scope_one_project),
            ("scope_many_projects", scope_many_projects),
            ("list_matching", list_matching),
            ("list_position", list_position),
            ("search_placeholder", search_placeholder),
            ("stat_observations", stat_observations),
            ("stat_sessions", stat_sessions),
            ("stat_prompts", stat_prompts),
            ("page_home", page_home),
            ("page_dashboard", page_dashboard),
            ("page_detail", page_detail),
            ("page_session", page_session),
            ("page_timeline", page_timeline),
            ("page_setup", page_setup),
            ("page_cloud", page_cloud),
            ("page_help", page_help),
            ("page_options", page_options),
            ("no_observations", no_observations),
            ("no_sessions", no_sessions),
            ("no_prompts", no_prompts),
            ("no_projects", no_projects),
            ("no_observation_selected", no_observation_selected),
            ("no_session_selected", no_session_selected),
            ("no_timeline_loaded", no_timeline_loaded),
            ("no_summary", no_summary),
            ("nothing_to_search", nothing_to_search),
            ("cancelled", cancelled),
            ("field_type", field_type),
            ("field_project", field_project),
            ("field_scope", field_scope),
            ("field_session", field_session),
            ("field_topic", field_topic),
            ("field_started", field_started),
            ("field_ended", field_ended),
            ("field_summary", field_summary),
            ("session_active", session_active),
            ("timeline_session", timeline_session),
            ("timeline_focus", timeline_focus),
            ("timeline_focus_marker", timeline_focus_marker),
            ("cloud_server", cloud_server),
            ("cloud_background", cloud_background),
            ("cloud_replicating", cloud_replicating),
            ("cloud_enrolled", cloud_enrolled),
            ("cloud_queued", cloud_queued),
            ("cloud_deferred", cloud_deferred),
            ("cloud_not_configured", cloud_not_configured),
            ("cloud_enabled", cloud_enabled),
            ("cloud_disabled", cloud_disabled),
            ("cloud_none", cloud_none),
            ("cloud_unknown", cloud_unknown),
            ("cloud_mutations", cloud_mutations),
            ("cloud_deferred_dead", cloud_deferred_dead),
            ("cloud_unreadable", cloud_unreadable),
            ("cloud_configure_hint", cloud_configure_hint),
            ("cloud_state", cloud_state),
            ("cloud_failures", cloud_failures),
            ("cloud_backoff", cloud_backoff),
            ("menu_start_setup", menu_start_setup),
            ("menu_dashboard", menu_dashboard),
            ("menu_cloud", menu_cloud),
            ("menu_options", menu_options),
            ("menu_help", menu_help),
            ("menu_quit", menu_quit),
            ("menu_uninstall", menu_uninstall),
            ("uninstall_heading", uninstall_heading),
            ("uninstall_agents", uninstall_agents),
            ("uninstall_warning", uninstall_warning),
            ("delete_memory", delete_memory),
            ("delete_prompt", delete_prompt),
            ("delete_session", delete_session),
            ("delete_project", delete_project),
            ("delete_permanent_warning", delete_permanent_warning),
            ("delete_prompts_warning", delete_prompts_warning),
            ("delete_recoverable", delete_recoverable),
            ("gone_permanently", gone_permanently),
            ("gone", gone),
            ("count_memories", count_memories),
            ("count_sessions", count_sessions),
            ("count_prompts", count_prompts),
            ("copied_to_clipboard", copied_to_clipboard),
            ("data_refreshed", data_refreshed),
            ("deleted_memory", deleted_memory),
            ("deleted_prompt", deleted_prompt),
            ("deleted_session", deleted_session),
            ("deleted_project", deleted_project),
            ("sessions_kept", sessions_kept),
            ("refreshed_query", refreshed_query),
            ("keys_confirm", keys_confirm),
            ("keys_confirm_footer", keys_confirm_footer),
            ("keys_confirm_window", keys_confirm_window),
            ("keys_home", keys_home),
            ("keys_query", keys_query),
            ("keys_filters", keys_filters),
            ("keys_dashboard_searching", keys_dashboard_searching),
            ("keys_dashboard_sessions", keys_dashboard_sessions),
            ("keys_dashboard_prompts", keys_dashboard_prompts),
            ("keys_dashboard", keys_dashboard),
            ("keys_detail", keys_detail),
            ("keys_session", keys_session),
            ("keys_timeline", keys_timeline),
            ("keys_setup", keys_setup),
            ("keys_options", keys_options),
            ("keys_cloud", keys_cloud),
            ("keys_help", keys_help),
            ("help_body", help_body),
        ]
    }

    #[test]
    fn no_screen_is_left_blank_in_any_language() {
        // The compiler enforces that every field exists. It has nothing to say
        // about a field somebody filled with "" to make a build pass.
        for language in Interface::ALL {
            for (field, text) in all_fields(screens(language)) {
                assert!(!text.trim().is_empty(), "{field} is blank in {language:?}");
            }
        }
    }

    #[test]
    fn every_placeholder_survives_translation() {
        // The failure that a table of strings invites and a compiler cannot
        // see: a translation that drops `{agents}`, or spells it `{agentes}`.
        // The value then never appears, and the screen reads as a question
        // about nothing in particular.
        for (field, english) in all_fields(screens(Interface::English)) {
            let wanted = placeholders(english);
            for language in Interface::ALL {
                let (_, text) = all_fields(screens(language))
                    .into_iter()
                    .find(|(name, _)| *name == field)
                    .expect("the same fields in every language");
                assert_eq!(
                    placeholders(text),
                    wanted,
                    "{field} in {language:?} does not carry the same values: {text}"
                );
            }
        }
    }

    /// The `{name}` placeholders in a template, in order.
    fn placeholders(text: &str) -> Vec<&str> {
        text.match_indices('{')
            .filter_map(|(at, _)| {
                let rest = &text[at + 1..];
                rest.find('}').map(|end| &rest[..end])
            })
            .collect()
    }

    /// The key legends have to fit the terminal, in every language.
    #[test]
    fn no_key_legend_outgrows_a_narrow_terminal() {
        // Spanish runs longer than English — "espacio elegir" against "space
        // select" — and a terminal UI has a fixed width. The footer is one
        // line: a legend wider than the screen is a legend with its last keys
        // cut off, which is exactly where the keys nobody remembers live.
        //
        // Eighty columns because that is the width nobody has to ask about.
        const NARROW: usize = 80;
        let mut too_wide = Vec::new();
        for language in Interface::ALL {
            let say = screens(language);
            for (field, legend) in [
                ("keys_confirm", say.keys_confirm),
                ("keys_confirm_footer", say.keys_confirm_footer),
                ("keys_confirm_window", say.keys_confirm_window),
                ("keys_home", say.keys_home),
                ("keys_query", say.keys_query),
                ("keys_filters", say.keys_filters),
                ("keys_dashboard_searching", say.keys_dashboard_searching),
                ("keys_dashboard_sessions", say.keys_dashboard_sessions),
                ("keys_dashboard_prompts", say.keys_dashboard_prompts),
                ("keys_dashboard", say.keys_dashboard),
                ("keys_detail", say.keys_detail),
                ("keys_session", say.keys_session),
                ("keys_timeline", say.keys_timeline),
                ("keys_setup", say.keys_setup),
                ("keys_options", say.keys_options),
                ("keys_cloud", say.keys_cloud),
                ("keys_help", say.keys_help),
                ("legend", say.legend),
            ] {
                let width = legend.chars().count();
                if width > NARROW {
                    too_wide.push(format!("{field} ({language:?}) is {width}: {legend}"));
                }
            }
        }
        assert!(
            too_wide.is_empty(),
            "these legends do not fit {NARROW} columns:
{}",
            too_wide.join(
                "
"
            )
        );
    }

    /// Nothing in the help page runs off a narrow terminal either.
    #[test]
    fn the_help_page_fits_a_narrow_terminal() {
        const NARROW: usize = 80;
        for language in Interface::ALL {
            for line in screens(language).help_body.lines() {
                let width = line.chars().count();
                assert!(
                    width <= NARROW,
                    "a help line is {width} columns in {language:?}: {line}"
                );
            }
        }
    }

    #[test]
    fn filling_a_template_puts_the_value_where_the_name_was() {
        assert_eq!(fill("a {b} c", "b", 3), "a 3 c");
        // A name that is not there changes nothing, rather than appending.
        assert_eq!(fill("a {b} c", "d", 3), "a {b} c");
    }

    #[test]
    fn a_translation_does_not_quietly_keep_the_english() {
        // Copying the English table and forgetting to rewrite a row is the
        // ordinary way a file like this rots, and every other check here passes
        // on it. It is also how a twelfth language gets added in an afternoon.
        //
        // Some rows are the same word by right — "auto" and "prompt" travel
        // untranslated, "Type" is Dutch for type, "Session" is Swedish for
        // session. Every one of those is named below, per language, so that
        // agreeing with English is a claim somebody wrote down and a reviewer
        // can disagree with, rather than a silence.
        let shared = |language: Interface| -> &'static [&'static str] {
            match language {
                // A translation of English into English is every row.
                Interface::English => &[],
                Interface::Spanish => &[
                    "language_auto",
                    "list_prompts",
                    "count_prompts",
                    "stat_prompts",
                ],
                Interface::Portuguese | Interface::Galician => &[
                    "language_auto",
                    "list_prompts",
                    "count_prompts",
                    "stat_prompts",
                    "page_detail",
                ],
                Interface::Catalan => &[
                    "language_auto",
                    "list_prompts",
                    "count_prompts",
                    "stat_prompts",
                    "list_sessions",
                    "stat_sessions",
                    "panel_context",
                    "timeline_focus_marker",
                ],
                Interface::French => &[
                    "language_auto",
                    "list_observations",
                    "list_sessions",
                    "list_prompts",
                    "count_prompts",
                    "stat_observations",
                    "stat_sessions",
                    "stat_prompts",
                    "field_type",
                    "field_session",
                    "page_session",
                    "panel_session",
                    "session_active",
                    "cloud_mutations",
                    "count_sessions",
                    "menu_options",
                    "panel_options",
                    "page_options",
                    "timeline_focus_marker",
                ],
                Interface::German => &[
                    "language_auto",
                    "list_prompts",
                    "stat_prompts",
                    "panel_detail",
                    "page_detail",
                    "page_cloud",
                    "scope_one_project",
                    "cloud_server",
                ],
                Interface::Italian => &[
                    "language_auto",
                    "page_cloud",
                    "scope_one_project",
                    "cloud_server",
                ],
                Interface::Basque => &["language_auto"],
                Interface::Dutch => &[
                    "language_auto",
                    "list_prompts",
                    "count_prompts",
                    "stat_prompts",
                    "field_project",
                    "field_session",
                    "panel_detail",
                    "panel_session",
                    "panel_context",
                    "page_detail",
                    "page_session",
                    "page_cloud",
                    "panel_filters",
                    "panel_filters_count",
                    "scope_one_project",
                    "cloud_server",
                    "timeline_focus_marker",
                ],
                Interface::Polish => &["language_auto"],
                Interface::Swedish => &[
                    "language_auto",
                    "field_project",
                    "field_session",
                    "panel_detail",
                    "panel_session",
                    "page_session",
                    "timeline_session",
                    "cloud_server",
                    "timeline_focus_marker",
                ],
            }
        };

        // Collected rather than asserted one at a time, so that adding a
        // language reports every row it left behind in one run instead of one
        // per compile.
        let english = all_fields(screens(Interface::English));
        let mut untranslated = Vec::new();
        for language in Interface::ALL {
            if language == Interface::English {
                continue;
            }
            let allowed = shared(language);
            for (field, translated) in all_fields(screens(language)) {
                if allowed.contains(&field) {
                    continue;
                }
                let (_, english) = english
                    .iter()
                    .find(|(name, _)| *name == field)
                    .expect("the same fields in every language");
                if *english == translated {
                    untranslated.push(format!("{field} ({language:?}): {translated}"));
                }
            }
        }
        assert!(
            untranslated.is_empty(),
            "these rows are still the English ones:\n{}",
            untranslated.join("\n")
        );
    }

    #[test]
    fn a_language_is_named_the_same_way_wherever_it_is_written() {
        // The settings file, the setup menu and the memory-language list all
        // spell a language, and they used to spell it from two different
        // tables. This is the round trip that says they are one: the word the
        // menu offers is the word the file holds is the word `parse` reads
        // back.
        for language in Interface::ALL {
            let written = serde_json::to_string(&language).expect("a language serializes");
            let read: Interface = serde_json::from_str(&written).expect("and reads back");
            assert_eq!(read, language, "{language:?} did not survive the file");
            assert_eq!(
                Interface::parse(language.as_str()),
                Some(language),
                "{language:?} is not readable by its own name"
            );
            assert_eq!(
                Interface::parse(language.code()),
                Some(language),
                "{language:?} is not readable by its code"
            );
            assert_eq!(
                crate::settings::language_choices(None)
                    .iter()
                    .filter(|choice| choice.as_deref() == Some(language.as_str()))
                    .count(),
                1,
                "{language:?} is speakable but not offered for memories, or offered twice"
            );
        }
    }
}
