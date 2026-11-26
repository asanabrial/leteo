//! Whole projects: naming, merging, deleting.

use super::*;

#[test]
fn a_page_is_a_window_onto_a_list_that_says_how_long_it_is() {
    // The failure this guards is a screen that shows the first hundred rows
    // of three thousand and offers no way to the rest, with a heading that
    // reports the truncation as the count.
    let (_temp, mut store) = store();
    store.create_session("s1", "Leteo", "C:/repo").unwrap();
    for n in 0..25 {
        store
            .add_observation(observation("s1", &format!("Memory {n:02}"), "body"))
            .unwrap();
    }

    let first = store.paged_observations("", &[], 0, 10).unwrap();
    assert_eq!(first.rows.len(), 10);
    assert_eq!(first.total, 25, "the total is of the list, not of the page");
    assert_eq!(first.rows[0].title, "Memory 24", "newest first");

    let third = store.paged_observations("", &[], 20, 10).unwrap();
    assert_eq!(third.rows.len(), 5, "the last page is short");
    assert_eq!(third.rows[0].title, "Memory 04");

    // Past the end is empty rather than an error, which is what lets the
    // screen step back to the last page instead of reporting a failure.
    let past = store.paged_observations("", &[], 100, 10).unwrap();
    assert!(past.rows.is_empty());
    assert_eq!(past.total, 25, "and it still says how long the list is");

    // A page of a session reads the other way round, and pages the same.
    let entries = store.paged_session_observations("s1", 20, 10).unwrap();
    assert_eq!(entries.total, 25);
    assert_eq!(entries.rows[0].title, "Memory 20", "oldest first here");
}

#[test]
fn a_doctor_project_scope_reports_that_projects_counts() {
    let (_temp, mut store) = store();
    store.create_session("s1", "Leteo", "C:/repo").unwrap();
    store
        .add_observation(observation("s1", "scoped", "body"))
        .unwrap();

    let (_, stats) = store.doctor_scoped(None, Some("Leteo")).unwrap();
    let stats = stats.expect("an existing project reports its counts");
    assert_eq!(stats.name, "leteo");
    assert_eq!(stats.observation_count, 1);
    assert!(
        stats.last_activity.is_some(),
        "the per-project detail carries the newest instant, so a project past \
         mem_stats's ceiling can still be asked for it: {stats:?}"
    );

    assert!(
        store
            .doctor_scoped(None, Some("no-such-project"))
            .unwrap_err()
            .to_string()
            .contains("unknown project")
    );
}

#[test]
fn a_set_of_projects_narrows_the_recent_lists() {
    let (_temp, mut store) = store();
    for project in ["leteo", "atlas", "quarry"] {
        store.create_session(project, project, "C:/repo").unwrap();
        let mut input = observation(project, "Something", "happened");
        input.project = Some(project.to_owned());
        store.add_observation(input).unwrap();
        store
            .add_prompt(AddPrompt {
                session_id: project.to_owned(),
                content: format!("what about {project}?"),
                project: Some(project.to_owned()),
            })
            .unwrap();
    }

    let named = |observations: &[Observation]| {
        let mut projects: Vec<String> = observations
            .iter()
            .map(|o| o.project.clone().unwrap_or_default())
            .collect();
        projects.sort();
        projects
    };

    // Empty means every project, not none. An `IN ()` clause is legal SQL
    // that matches nothing, so getting this backwards shows an empty screen
    // and looks like an empty store.
    assert_eq!(
        named(&store.paged_observations("", &[], 0, 10).unwrap().rows).len(),
        3
    );

    let two = ["leteo".to_owned(), "quarry".to_owned()];
    assert_eq!(
        named(&store.paged_observations("", &two, 0, 10).unwrap().rows),
        vec!["leteo".to_owned(), "quarry".to_owned()]
    );
    assert_eq!(store.paged_sessions("", &two, 0, 10).unwrap().total, 2);
    assert_eq!(store.paged_prompts("", &two, 0, 10).unwrap().total, 2);

    // A name that is not a project of this store simply matches nothing,
    // rather than being treated as no filter at all.
    let missing = ["nowhere".to_owned()];
    assert!(
        store
            .paged_observations("", &missing, 0, 10)
            .unwrap()
            .rows
            .is_empty()
    );

    // And the names are bound, not pasted: a project named like an injection
    // is a legal project name.
    let hostile = ["leteo') OR 1=1 --".to_owned()];
    assert!(
        store
            .paged_observations("", &hostile, 0, 10)
            .unwrap()
            .rows
            .is_empty()
    );
}

#[test]
fn project_export_and_json_import_are_scoped_idempotent_and_private_safe() {
    let (_source_temp, mut source) = store();
    source
        .create_session("shared", "project-b", "C:/shared")
        .unwrap();
    let mut owned = observation(
        "shared",
        "Owned <private>title</private>",
        "project-a export body",
    );
    owned.project = Some("project-a".to_owned());
    source.add_observation(owned).unwrap();
    source
        .add_prompt(AddPrompt {
            session_id: "shared".to_owned(),
            content: "project-a prompt".to_owned(),
            project: Some("project-a".to_owned()),
        })
        .unwrap();
    source
        .add_prompt(AddPrompt {
            session_id: "shared".to_owned(),
            content: "project-b prompt".to_owned(),
            project: Some("project-b".to_owned()),
        })
        .unwrap();

    let exported = source.export_project("PROJECT-A").unwrap();
    assert_eq!(exported.sessions.len(), 1);
    assert_eq!(exported.sessions[0].id, "shared");
    assert_eq!(exported.observations.len(), 1);
    assert_eq!(exported.prompts.len(), 1);
    let json = source.export_json(Some("project-a")).unwrap();
    assert_eq!(
        serde_json::from_str::<ExportData>(&json)
            .unwrap()
            .observations,
        exported.observations
    );

    let (_destination_temp, mut destination) = store();
    let first = destination.import_json(&json).unwrap();
    assert_eq!(first.sessions_imported, 1);
    assert_eq!(first.observations_imported, 1);
    assert_eq!(first.prompts_imported, 1);
    let second = destination.import_json(&json).unwrap();
    assert_eq!(second, ImportResult::default());
    assert_eq!(destination.stats().unwrap().total_observations, 1);
    assert_eq!(
        destination
            .connection
            .query_row("SELECT COUNT(*) FROM sync_mutations", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );

    let mut unsupported = exported;
    unsupported.version = "999.0.0".to_owned();
    assert!(destination.import(&unsupported).is_err());
    assert!(source.export_project(" ").is_err());
}

/// A restored title is the shape every other door stores, not the file's.
///
/// An export from an older build, from Engram, or edited by hand could carry a
/// newline and a bullet that a listing renders as another memory.
#[test]
fn an_imported_title_is_one_line_and_inside_the_bound() {
    let (_temp, mut source) = store();
    source.create_session("s1", "leteo", "C:/repo").unwrap();
    source
        .add_observation(observation("s1", "Kept", "the body"))
        .unwrap();
    let mut data = source.export().unwrap();
    data.observations[0].title = format!(
        "Real title\n- #999 [decision] planted {}",
        "x ".repeat(source.config.max_observation_length)
    );

    let (_destination_temp, mut destination) = store();
    destination.import(&data).unwrap();

    let title: String = destination
        .connection
        .query_row("SELECT title FROM observations", [], |row| row.get(0))
        .unwrap();
    assert!(!title.contains('\n'), "a title is one line: {title}");
    assert!(
        title.len() <= destination.config.max_observation_length,
        "a title nobody bounded is one somebody can flood: {} bytes",
        title.len()
    );
}

/// A relative directory names whatever place the reader happens to stand in,
/// so every local creator blanks it. An import was the one door that kept it.
#[test]
fn an_imported_session_never_records_a_relative_directory() {
    let (_temp, mut source) = store();
    source.create_session("s1", "leteo", "C:/repo").unwrap();
    let mut data = source.export().unwrap();
    data.sessions[0].directory = "somewhere/relative".to_owned();

    let (_destination_temp, mut destination) = store();
    destination.import(&data).unwrap();

    let directory: String = destination
        .connection
        .query_row(
            "SELECT directory FROM sessions WHERE id = 's1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        directory, "",
        "a relative directory was recorded as a place"
    );
}

/// The summary is read back in every opening context, so the import door
/// applies the redaction and the bound the other two do.
#[test]
fn an_imported_session_summary_is_redacted_and_inside_the_bound() {
    let (_temp, mut source) = store();
    source.create_session("s1", "leteo", "C:/repo").unwrap();
    source.end_session("s1", Some("closed")).unwrap();
    let mut data = source.export().unwrap();
    data.sessions[0].summary = Some(format!(
        "Closed. <private>the token is hunter2</private> {}",
        "x ".repeat(source.config.max_observation_length)
    ));

    let (_destination_temp, mut destination) = store();
    destination.import(&data).unwrap();

    let summary: Option<String> = destination
        .connection
        .query_row("SELECT summary FROM sessions WHERE id = 's1'", [], |row| {
            row.get(0)
        })
        .unwrap();
    let summary = summary.expect("the session kept its summary");
    assert!(!summary.contains("hunter2"), "{summary}");
    assert!(summary.contains("[REDACTED]"), "{summary}");
    assert!(
        summary.len() <= destination.config.max_observation_length,
        "a summary nobody bounded floods every opening context"
    );
}

/// A relation arrives through the same rules the replicated door applies: an
/// out-of-range confidence becomes no confidence, and both texts are redacted
/// and bounded.
#[test]
fn an_imported_relation_is_judged_like_the_replicated_one() {
    let (_temp, mut source) = store();
    source.create_session("s1", "leteo", "C:/repo").unwrap();
    let first = source
        .add_observation(observation("s1", "Chose SQLite", "the engine decision"))
        .unwrap()
        .observation;
    let second = source
        .add_observation(observation(
            "s1",
            "Chose Postgres",
            "the engine decision again",
        ))
        .unwrap()
        .observation;
    let relation = source
        .save_relation(crate::memory::model::SaveRelationParams {
            sync_id: crate::memory::normalize::sync_id("rel"),
            source_id: second.sync_id.clone(),
            target_id: first.sync_id.clone(),
        })
        .unwrap();

    let mut data = source.export().unwrap();
    data.relations[0].confidence = Some(7.5);
    data.relations[0].reason =
        Some("Same session. <private>the token is hunter2</private>".to_owned());
    data.relations[0].evidence = Some("x ".repeat(source.config.max_observation_length));

    let (_destination_temp, mut destination) = store();
    destination.import(&data).unwrap();

    let (confidence, reason, evidence): (Option<f64>, Option<String>, Option<String>) = destination
        .connection
        .query_row(
            "SELECT confidence, reason, evidence FROM memory_relations WHERE sync_id = ?1",
            [&relation.sync_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(confidence, None, "7.5 is not a probability");
    let reason = reason.expect("the relation kept its reason");
    assert!(!reason.contains("hunter2"), "{reason}");
    assert!(reason.contains("[REDACTED]"), "{reason}");
    assert!(
        evidence.unwrap_or_default().len() <= destination.config.max_observation_length,
        "evidence nobody bounded is evidence somebody can flood"
    );
}

#[test]
fn merge_projects_updates_all_entities_and_journals_active_rows() {
    let (_temp, mut store) = store();
    for project in ["old-project", "canonical-project"] {
        store.enroll_project(project).unwrap();
    }
    store
        .create_session("old-session", "Old--Project", "C:/old")
        .unwrap();
    let mut old_observation = observation("old-session", "Merge", "merge body");
    old_observation.project = Some("old-project".to_owned());
    let saved = store.add_observation(old_observation).unwrap().observation;
    let prompt = store
        .add_prompt(AddPrompt {
            session_id: "old-session".to_owned(),
            content: "merge prompt".to_owned(),
            project: Some("old-project".to_owned()),
        })
        .unwrap();
    let before: i64 = store
        .connection
        .query_row("SELECT COUNT(*) FROM sync_mutations", [], |row| row.get(0))
        .unwrap();

    let result = store
        .merge_projects(
            &["OLD--PROJECT".to_owned(), "old-project".to_owned()],
            "Canonical--Project",
        )
        .unwrap();
    assert_eq!(result.canonical, "canonical-project");
    // Only the spelling that holds the rows, plus the name the caller asked
    // for, which is the same one here. The other separator spellings hold
    // nothing and were never projects; reporting them would name something no
    // store has ever held.
    assert_eq!(result.sources_merged, ["old-project"]);
    assert_eq!(result.observations_updated, 1);
    assert_eq!(result.sessions_updated, 1);
    assert_eq!(result.prompts_updated, 1);
    assert_eq!(
        store.get_observation(saved.id).unwrap().project.as_deref(),
        Some("canonical-project")
    );
    assert_eq!(
        store.get_session("old-session").unwrap().project,
        "canonical-project"
    );
    assert_eq!(
        store
            .recent_prompts(Some("canonical-project"), None)
            .unwrap()[0]
            .id,
        prompt.id
    );
    // What the journal holds afterwards, rather than how many rows it gained.
    // The merge queues the three moved rows under the canonical name and drops
    // what was still waiting under the old one — every row those described has
    // just been queued again, in its current state — so the net count is zero
    // and says nothing. `before` is kept because it is what makes that
    // sentence checkable.
    let journalled = |project: &str| -> i64 {
        store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM sync_mutations WHERE project = ?1",
                params![project],
                |row| row.get(0),
            )
            .unwrap()
    };
    assert_eq!(before, 3, "the old project had queued its own three rows");
    // Five, not three: the session's unsent upsert stays where it was, ahead of
    // the memory and prompt that point at it, beside the copy the merge queued,
    // and the merge says it was a rename.
    assert_eq!(journalled("canonical-project"), 5);
    assert_eq!(
        journalled("old-project"),
        0,
        "and nothing is still waiting under a name that now holds nothing"
    );
    assert!(
        !store
            .enrolled_projects()
            .unwrap()
            .contains(&"old-project".to_owned()),
        "an emptied project stops being replicated"
    );
    assert!(store.merge_projects(&[], "").is_err());
}

#[test]
fn merging_a_name_leaves_a_live_spelling_of_it_alone() {
    // `my-app` and `my_app` are different projects: listing, deleting and
    // `project_exists` all read the name literally. The merge alone joined
    // them, so the underscore project's memory moved into the canonical one
    // and the underscore project stopped replicating, while the reply named
    // `my-app` as the only source.
    //
    // The two spellings cover the two ways a name is a project of its own:
    // `my_app` holds a live memory without being enroled, and `my app` is
    // enroled while its only memory is soft-deleted. Either check alone would
    // let one of them fold.
    let (_temp, mut store) = store();
    for project in ["my-app", "my app"] {
        store.enroll_project(project).unwrap();
    }
    store
        .create_session("underscore", "my_app", "C:/underscore")
        .unwrap();
    let mut memory = observation("underscore", "The underscore project's memory", "body");
    memory.project = Some("my_app".to_owned());
    let saved = store.add_observation(memory).unwrap().observation;
    let mut deleted = observation("underscore", "The spaced project's memory", "gone");
    deleted.project = Some("my app".to_owned());
    let deleted = store.add_observation(deleted).unwrap().observation;
    store.delete_observation(deleted.id, None, false).unwrap();
    store.create_session("dash", "my-app", "C:/dash").unwrap();

    let result = store.merge_projects(&["my-app".to_owned()], "web").unwrap();

    assert_eq!(
        result.sources_merged,
        ["my-app"],
        "a live spelling was moved or reported"
    );
    assert_eq!(
        store.get_observation(saved.id).unwrap().project.as_deref(),
        Some("my_app"),
        "the memory belongs to the project that is not being merged"
    );
    assert_eq!(store.get_session("underscore").unwrap().project, "my_app");
    let deleted_project: String = store
        .connection
        .query_row(
            "SELECT project FROM observations WHERE id = ?1",
            [deleted.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        deleted_project, "my app",
        "an enroled project is a project even when its only memory is deleted"
    );
    let enrolled = store.enrolled_projects().unwrap();
    assert!(
        enrolled.contains(&"my app".to_owned()),
        "a live project that was not merged stopped replicating: {enrolled:?}"
    );
    assert_eq!(
        store.get_session("dash").unwrap().project,
        "web",
        "and the project the caller did name still moves"
    );
    assert!(!enrolled.contains(&"my-app".to_owned()));
}

#[test]
fn merging_an_enroled_name_that_holds_nothing_changes_nothing() {
    // Nothing to move is not an event. This is the merge's behaviour before it
    // had a scope to report, and moving the enrolment instead would retire a
    // project nobody asked to stop replicating.
    let (_temp, mut store) = store();
    store.enroll_project("empty-project").unwrap();

    let result = store
        .merge_projects(&["empty-project".to_owned()], "canonical")
        .unwrap();

    assert!(result.sources_merged.is_empty(), "{result:?}");
    assert!(!result.canonical_created);
    assert!(
        store
            .enrolled_projects()
            .unwrap()
            .contains(&"empty-project".to_owned()),
        "the enrolment moved for a merge that moved nothing"
    );
}

#[test]
fn a_merge_reports_every_name_it_moves_and_the_preview_asks_the_same_read() {
    let (_temp, mut store) = store();
    store
        .create_session("widget", "widget-app", "C:/widget")
        .unwrap();
    store
        .add_observation(observation("widget", "The widget", "body"))
        .unwrap();
    // A spelling that holds nothing live but still has a row of its own to
    // carry. It is not a project of its own, so the merge folds it, and the
    // preview and the reply both have to say so.
    let mut legacy = observation("widget", "The legacy spelling", "gone");
    legacy.project = Some("widget_app".to_owned());
    let legacy = store.add_observation(legacy).unwrap().observation;
    store.delete_observation(legacy.id, None, false).unwrap();

    let preview = store
        .project_merge_scope(&["widget-app".to_owned()], "widgets")
        .unwrap();
    assert!(
        preview.contains(&"widget-app".to_owned()),
        "the project the caller named is missing from the preview: {preview:?}"
    );
    assert!(
        preview.contains(&"widget_app".to_owned()),
        "the fold the merge performs is missing from the preview: {preview:?}"
    );
    assert!(
        !preview.contains(&"widget app".to_owned()),
        "a spelling that holds nothing was reported as moved: {preview:?}"
    );

    let result = store
        .merge_projects(&["widget-app".to_owned()], "widgets")
        .unwrap();
    assert_eq!(
        result.sources_merged, preview,
        "the reply and the preview must name the same projects"
    );
    let mut sorted = preview.clone();
    sorted.sort();
    assert_eq!(preview, sorted, "the reply is documented in sorted order");
}

#[test]
fn pruning_removes_empty_projects_and_refuses_populated_ones() {
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    store.create_session("s2", "empty", "C:/empty").unwrap();
    store
        .add_observation(observation("s1", "Kept", "kept body"))
        .unwrap();
    store
        .add_prompt(AddPrompt {
            session_id: "s2".to_owned(),
            content: "empty prompt".to_owned(),
            project: Some("empty".to_owned()),
        })
        .unwrap();

    let result = store.prune_project("Empty").unwrap();
    assert_eq!(result.project, "empty");
    assert_eq!(result.sessions_deleted, 1);
    assert_eq!(result.prompts_deleted, 1);
    assert!(store.get_session("s2").is_err());
    assert!(store.prune_project("leteo").is_err());
    assert!(store.prune_project("  ").is_err());
}

#[test]
fn pruning_leaves_a_session_that_another_projects_rows_still_use() {
    // `delete_project --hard` already keeps such a session. Prune counted only
    // the pruned project's own observations, so a session that held nothing of
    // it but did hold another project's memory was deleted from underneath that
    // memory and the whole prune failed with a bare constraint error.
    let (_temp, mut store) = store();
    store.create_session("shared", "empty", "C:/empty").unwrap();
    store.create_session("alone", "empty", "C:/empty").unwrap();
    let kept = store
        .add_observation(observation("shared", "Kept", "kept body"))
        .unwrap()
        .observation;
    store
        .add_prompt(AddPrompt {
            session_id: "shared".to_owned(),
            content: "asked under the other project".to_owned(),
            project: Some("leteo".to_owned()),
        })
        .unwrap();

    let result = store.prune_project("empty").unwrap();

    assert_eq!(result.sessions_deleted, 1);
    assert!(store.get_session("alone").is_err());
    assert_eq!(store.get_session("shared").unwrap().project, "empty");
    assert_eq!(store.get_observation(kept.id).unwrap().session_id, "shared");
}

#[test]
fn scanning_a_project_reports_candidates_and_only_inserts_when_applied() {
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    // Filler, and load-bearing: a candidate has to score past the floor the
    // scan applies, and a bm25 term weight grows with how rare the word is
    // across the store. Two memories score near zero however alike they are.
    for index in 0..40 {
        store
            .add_observation(observation(
                "s1",
                &format!("Unrelated note {index} on deployment windows"),
                &format!("Body {index}: staged rollout, canaries and a rollback plan."),
            ))
            .unwrap();
    }
    store
        .add_observation(observation(
            "s1",
            "Connection pooling limits",
            "the pool caps concurrent writers",
        ))
        .unwrap();
    store
        .add_observation(observation(
            "s1",
            "Connection pooling limits revisited",
            "the pool now caps concurrent readers",
        ))
        .unwrap();

    let dry_run = store
        .scan_project(ScanOptions {
            project: "Leteo".to_owned(),
            ..ScanOptions::default()
        })
        .unwrap();
    assert!(dry_run.dry_run);
    assert_eq!(dry_run.inspected, 42);
    assert!(dry_run.candidates_found >= 2);
    // What the apply would write, not zero because nothing was written.
    //
    // This asserted zero, which is what a dry run used to report because it
    // never looked: it skipped the loop that asks whether a pair is already
    // related, so both that count and this one came back empty whatever the
    // store held. The claim the test is named for is the one below — nothing
    // reaches the table — and that is unchanged.
    assert!(
        dry_run.relations_inserted >= 1,
        "a preview says what applying would do: {dry_run:?}"
    );
    assert!(
        store
            .list_relations(ListRelationsOptions::default())
            .unwrap()
            .is_empty(),
        "and writes nothing while saying it"
    );

    let applied = store
        .scan_project(ScanOptions {
            project: "leteo".to_owned(),
            apply: true,
            ..ScanOptions::default()
        })
        .unwrap();
    assert!(!applied.dry_run);
    assert_eq!(applied.relations_inserted, 1);
    assert_eq!(applied.already_related, 1);
    // The preview and the apply say the same thing, which is the only reason
    // to have a preview. The finder proposes a pair from both ends, so one of
    // the two candidates is the same two memories again — the apply used to
    // notice that only because its own first insert made the second one
    // already-related, and the dry run, writing nothing, counted both.
    assert_eq!(
        dry_run.relations_inserted, applied.relations_inserted,
        "preview {dry_run:?} against apply {applied:?}"
    );
    assert_eq!(dry_run.already_related, applied.already_related);
    let relations = store
        .list_relations(ListRelationsOptions::default())
        .unwrap();
    assert_eq!(relations.len(), 1);
    assert_eq!(relations[0].judgment_status, JUDGMENT_STATUS_PENDING);

    let repeated = store
        .scan_project(ScanOptions {
            project: "leteo".to_owned(),
            apply: true,
            ..ScanOptions::default()
        })
        .unwrap();
    assert_eq!(repeated.relations_inserted, 0);
    assert_eq!(repeated.already_related, 2);
}

#[test]
fn nothing_is_journalled_until_a_project_is_enrolled_and_then_it_catches_up() {
    // Every journal row carries a full JSON copy of what it describes, and
    // nothing removes an unacknowledged one — pruning only reaches rows the
    // cloud confirmed, and a cloud that was never configured confirms
    // nothing. A real store held 9 525 of them against 3 408 memories:
    // 14.5 MB of a 42 MB database, written for a reader that did not exist.
    let (_temp, mut store) = bare_store();
    store.create_session("s1", "Leteo", "C:/repo").unwrap();
    store
        .add_observation(observation("s1", "Before enrolling", "body"))
        .unwrap();
    store
        .add_prompt(AddPrompt {
            session_id: "s1".to_owned(),
            content: "why?".to_owned(),
            project: Some("Leteo".to_owned()),
        })
        .unwrap();
    let journalled = |store: &Store| -> i64 {
        store
            .connection
            .query_row("SELECT COUNT(*) FROM sync_mutations", [], |row| row.get(0))
            .unwrap()
    };
    assert_eq!(journalled(&store), 0, "nobody replicates this project yet");

    // Enrolling has to catch up, or the trade above would cost a project
    // its history: it would replicate whatever came next and silently never
    // send what it already held.
    assert!(store.enroll_project("leteo").unwrap());
    let after = journalled(&store);
    assert!(after >= 3, "session, memory and prompt all queued: {after}");
    for entity in ["observation", "session", "prompt"] {
        let count: i64 = store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM sync_mutations WHERE entity = ?1 AND op = 'upsert'",
                [entity],
                |row| row.get(0),
            )
            .unwrap();
        assert!(count >= 1, "{entity} was not caught up");
    }

    // Enrolling again is not a second copy of everything.
    assert!(!store.enroll_project("leteo").unwrap());
    assert_eq!(journalled(&store), after);

    store
        .add_observation(observation("s1", "After enrolling", "body"))
        .unwrap();
    assert!(journalled(&store) > after);
}

#[test]
fn semantic_compare_is_idempotent_journaled_and_rejects_cross_project() {
    let (_temp, mut store) = store();
    store.enroll_project("other").unwrap();
    store.create_session("same", "leteo", "C:/same").unwrap();
    store.create_session("other", "other", "C:/other").unwrap();
    let first = store
        .add_observation(observation("same", "Auth decision A", "first"))
        .unwrap()
        .observation;
    let second = store
        .add_observation(observation("same", "Auth decision B", "second"))
        .unwrap()
        .observation;
    let unrelated = store
        .add_observation(observation("same", "Database decision", "third"))
        .unwrap()
        .observation;
    let mut other_input = observation("other", "Other project auth", "other");
    other_input.project = Some("other".to_owned());
    let other = store.add_observation(other_input).unwrap().observation;

    let first_sync_id = store
        .judge_by_semantic(JudgeBySemanticParams {
            source_id: first.sync_id.clone(),
            target_id: second.sync_id.clone(),
            relation: RELATION_RELATED.to_owned(),
            confidence: Some(0.7),
            reasoning: Some("initial verdict".to_owned()),
            model: Some("model-v1".to_owned()),
        })
        .unwrap();
    let second_sync_id = store
        .judge_by_semantic(JudgeBySemanticParams {
            source_id: second.sync_id.clone(),
            target_id: first.sync_id.clone(),
            relation: RELATION_COMPATIBLE.to_owned(),
            confidence: Some(0.95),
            reasoning: Some("updated verdict".to_owned()),
            model: Some("model-v2".to_owned()),
        })
        .unwrap();
    assert_eq!(first_sync_id, second_sync_id);
    let relation = store.get_relation(&first_sync_id).unwrap();
    assert_eq!(relation.relation, RELATION_COMPATIBLE);
    assert_eq!(relation.marked_by_actor.as_deref(), Some("leteo"));
    assert_eq!(relation.marked_by_kind.as_deref(), Some("system"));
    assert_eq!(relation.marked_by_model.as_deref(), Some("model-v2"));
    assert_eq!(
        store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM memory_relations
                     WHERE (source_id = ?1 AND target_id = ?2)
                        OR (source_id = ?2 AND target_id = ?1)",
                params![first.sync_id, second.sync_id],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    let journal_before_noop: i64 = store
        .connection
        .query_row(
            "SELECT COUNT(*) FROM sync_mutations WHERE entity = 'relation'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let noop = store
        .judge_by_semantic(JudgeBySemanticParams {
            source_id: first.sync_id.clone(),
            target_id: unrelated.sync_id.clone(),
            relation: RELATION_NOT_CONFLICT.to_owned(),
            confidence: Some(1.0),
            reasoning: Some("unrelated".to_owned()),
            model: None,
        })
        .unwrap();
    assert!(noop.is_empty());
    assert_eq!(
        store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM memory_relations
                     WHERE (source_id = ?1 AND target_id = ?2)
                        OR (source_id = ?2 AND target_id = ?1)",
                params![first.sync_id, unrelated.sync_id],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    assert_eq!(
        store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM sync_mutations WHERE entity = 'relation'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        journal_before_noop
    );
    let payload: serde_json::Value = serde_json::from_str(
        &store
            .connection
            .query_row(
                "SELECT payload FROM sync_mutations
                     WHERE entity = 'relation' ORDER BY seq DESC LIMIT 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
    )
    .unwrap();
    assert_eq!(payload["project"], "leteo");
    assert_eq!(payload["judgment_status"], JUDGMENT_STATUS_JUDGED);

    assert!(matches!(
        store.judge_by_semantic(JudgeBySemanticParams {
            source_id: first.sync_id.clone(),
            target_id: other.sync_id.clone(),
            relation: RELATION_CONFLICTS_WITH.to_owned(),
            confidence: Some(0.9),
            reasoning: Some("cross project".to_owned()),
            model: None,
        }),
        Err(StoreError::CrossProjectRelation { .. })
    ));
    let cross_id = normalize::sync_id("rel");
    store
        .save_relation(SaveRelationParams {
            sync_id: cross_id.clone(),
            source_id: first.sync_id,
            target_id: other.sync_id,
        })
        .unwrap();
    assert!(matches!(
        store.judge_relation(JudgeRelationParams {
            judgment_id: cross_id.clone(),
            relation: RELATION_SUPERSEDES.to_owned(),
            marked_by_actor: "agent".to_owned(),
            marked_by_kind: "agent".to_owned(),
            ..JudgeRelationParams::default()
        }),
        Err(StoreError::CrossProjectRelation { .. })
    ));
    assert_eq!(
        store.get_relation(&cross_id).unwrap().judgment_status,
        JUDGMENT_STATUS_PENDING
    );
}

#[test]
fn a_destructive_operation_refuses_a_project_that_names_nothing() {
    // These three are the only calls in Leteo that remove memories on purpose,
    // and each normalises its argument before using it. An empty name survives
    // normalisation as an empty string, and the queries beneath match on
    // `ifnull(project, '') = ?1` — so an unguarded empty name does not match
    // *nothing*, it matches **every row that has no project**. Deleting or
    // pruning by mistake is not the risk; deleting exactly the rows nobody
    // bothered to file is.
    //
    // All three refusals existed. None was tested: removing the one in
    // `delete_project` left the whole suite green.
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    store
        .add_observation(observation("s1", "A memory with a project", "body"))
        .unwrap();
    // A row that carries no project at all, which is what an empty name reaches.
    store
        .connection
        .execute("UPDATE observations SET project = NULL WHERE id = 1", [])
        .unwrap();

    for empty in ["", "   ", "\t"] {
        assert!(
            store.delete_project(empty, true).is_err(),
            "delete accepted {empty:?}"
        );
        assert!(
            store.prune_project(empty).is_err(),
            "prune accepted {empty:?}"
        );
        assert!(
            store.merge_projects(&["leteo".to_owned()], empty).is_err(),
            "merge accepted {empty:?} as the canonical name"
        );
    }

    assert_eq!(
        store
            .connection
            .query_row("SELECT COUNT(*) FROM observations", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn merging_a_project_into_itself_moves_nothing_and_says_so() {
    // Asking to merge `leteo` into `leteo` is not an error — it is what a
    // consolidation proposes when one of the candidates is already the
    // canonical name, and refusing it would make the caller filter its own
    // list. It has to be a no-op that reports one.
    //
    // Two mechanisms enforce it and only one is load-bearing: the loop skips
    // a source equal to the canonical, and `project_merge_variants` already
    // drops every candidate that normalises to the canonical — so with the
    // skip removed the variant set is empty, nothing is collected, and nothing
    // moves. A mutation deleting the skip survives this test, and the reason
    // is redundancy rather than a hole. Worth knowing instead of rediscovering.
    //
    // What is asserted is therefore the behaviour, so it holds whichever of
    // the two is doing the work: no rows counted as moved, and no sync
    // mutation journalled for a change that is not one.
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    for title in ["First", "Second", "Third"] {
        store
            .add_observation(observation("s1", title, "body"))
            .unwrap();
    }
    let journalled = |store: &Store| {
        store
            .connection
            .query_row("SELECT COUNT(*) FROM sync_mutations", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap()
    };
    let before = journalled(&store);

    let result = store
        .merge_projects(&["leteo".to_owned(), "  LETEO  ".to_owned()], "leteo")
        .unwrap();

    assert_eq!(result.canonical, "leteo");
    assert!(
        result.sources_merged.is_empty(),
        "nothing was merged: {:?}",
        result.sources_merged
    );
    assert_eq!(
        (
            result.observations_updated,
            result.sessions_updated,
            result.prompts_updated
        ),
        (0, 0, 0),
        "a memory that did not move must not be counted as one"
    );
    assert_eq!(
        journalled(&store),
        before,
        "a rewrite onto the same project must not reach a peer as a change"
    );
}

/// Replication follows the memories when they move.
///
/// Nothing is journalled for a project nobody replicates, so merging an
/// enrolled project into a name that is not enrolled queued every moved row
/// into nowhere: the memories arrived under the new name, the peers were never
/// told again, and the enrolment went on naming the old project, which by then
/// held nothing at all. Replication stopped and nothing said so.
#[test]
fn merging_into_an_unenrolled_name_does_not_quietly_stop_replicating() {
    let (_temp, mut store) = store();
    store.enroll_project("old-project").unwrap();
    store
        .create_session("old-session", "old-project", "C:/old")
        .unwrap();
    let mut memory = observation("old-session", "Worth sending", "and worth keeping");
    memory.project = Some("old-project".to_owned());
    store.add_observation(memory).unwrap();

    // `canonical-project` is not enrolled, which is the ordinary case: a name
    // chosen at the moment of merging has never replicated anything.
    let result = store
        .merge_projects(&["old-project".to_owned()], "canonical-project")
        .unwrap();
    assert!(
        result.enrolment_moved,
        "the answer has to say that what leaves this machine changed"
    );

    let enrolled = store.enrolled_projects().unwrap();
    assert!(
        enrolled.contains(&"canonical-project".to_owned()),
        "the memories are still replicated: {enrolled:?}"
    );
    assert!(
        !enrolled.contains(&"old-project".to_owned()),
        "and the name that holds nothing is not: {enrolled:?}"
    );
    let waiting: i64 = store
        .connection
        .query_row(
            "SELECT COUNT(*) FROM sync_mutations WHERE project = 'canonical-project'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        waiting > 0,
        "and the moved rows are queued under the name they now have"
    );
}

/// Moving one directory's sessions queues the rows it moved and leaves the rest
/// of the name replicating.
///
/// The narrower move does the replication arrangement of `merge_projects` for
/// only the rows it touches: each kind is queued again under the new name, and
/// the old name keeps its enrolment and its unsent mutations while another
/// checkout's sessions still hold it.
#[test]
fn moving_one_directorys_sessions_queues_what_moved_and_keeps_the_rest_replicating() {
    let (_temp, mut store) = store();
    store.enroll_project("api").unwrap();
    for (session, directory) in [("here", "/work/api"), ("there", "/other/api")] {
        store.create_session(session, "api", directory).unwrap();
        let mut memory = observation(session, session, "a memory");
        memory.project = Some("api".to_owned());
        store.add_observation(memory).unwrap();
        store
            .add_prompt(crate::memory::model::AddPrompt {
                session_id: session.to_owned(),
                content: format!("a prompt typed in {session}"),
                project: Some("api".to_owned()),
            })
            .unwrap();
    }
    let queued = |store: &Store, project: &str, entity: &str| -> i64 {
        store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM sync_mutations
                  WHERE project = ?1 AND entity = ?2 AND acked_at IS NULL",
                [project, entity],
                |row| row.get(0),
            )
            .unwrap()
    };
    let unsent_before = queued(&store, "api", "observation");

    let (result, left_behind) = store
        .move_directory_sessions("api", "backend", std::path::Path::new("/work/api"))
        .unwrap();

    assert_eq!(left_behind, 1);
    assert_eq!(
        (
            result.observations_updated,
            result.sessions_updated,
            result.prompts_updated
        ),
        (1, 1, 1)
    );
    assert!(result.enrolment_moved);
    for entity in ["observation", "session", "prompt"] {
        assert_eq!(
            queued(&store, "backend", entity),
            1,
            "the moved {entity} is queued under the name it now has"
        );
    }
    let enrolled = store.enrolled_projects().unwrap();
    assert!(
        enrolled.contains(&"api".to_owned()),
        "the other checkout's rows are still replicated: {enrolled:?}"
    );
    assert_eq!(
        queued(&store, "api", "observation"),
        unsent_before,
        "and what was waiting to be sent for them is not thrown away"
    );
}

/// Every unsent mutation of `project`, oldest first, as `(entity, op, payload)`.
fn unsent_under(store: &Store, project: &str) -> Vec<(String, String, serde_json::Value)> {
    store
        .connection
        .prepare(
            "SELECT entity, op, payload FROM sync_mutations
              WHERE project = ?1 AND acked_at IS NULL ORDER BY seq",
        )
        .unwrap()
        .query_map([project], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .unwrap()
        .map(|row| {
            let (entity, op, payload) = row.unwrap();
            (entity, op, serde_json::from_str(&payload).unwrap())
        })
        .collect()
}

fn enrolled_store(projects: &[&str]) -> (TempDir, Store) {
    let (temp, mut store) = store();
    for project in projects {
        store.enroll_project(project).unwrap();
    }
    (temp, store)
}

/// A delete that had not been sent when its project was merged away still goes.
///
/// The merge dropped every unsent mutation of the emptied name because it had
/// queued the live rows again, and a deleted memory is not a live row: the peer
/// went on holding a memory its owner had removed, and nothing reported it.
///
/// The second half is what a peer does with the rewritten delete. It applies it
/// only to a row filed under the delete's project, so the delete has to name the
/// project the peer holds the row under once it has merged too.
#[test]
fn merging_a_project_keeps_the_delete_that_was_waiting_for_a_peer() {
    let (_temp, mut store) = enrolled_store(&["old-project", "canonical-project"]);
    store.create_session("s1", "old-project", "C:/old").unwrap();
    let mut memory = observation("s1", "Removed before sending", "gone");
    memory.project = Some("old-project".to_owned());
    let removed = store.add_observation(memory).unwrap().observation;
    store.delete_observation(removed.id, None, false).unwrap();

    store
        .merge_projects(&["old-project".to_owned()], "canonical-project")
        .unwrap();

    let waiting = unsent_under(&store, "canonical-project");
    let delete = waiting
        .iter()
        .find(|(entity, op, _)| entity == "observation" && op == "delete")
        .unwrap_or_else(|| panic!("the delete was dropped: {waiting:?}"));
    assert_eq!(delete.2["sync_id"], removed.sync_id);
    assert_eq!(
        delete.2["project"], "canonical-project",
        "a peer applies a delete only to the project it holds the row under"
    );
    assert!(
        unsent_under(&store, "old-project").is_empty(),
        "nothing is left waiting under a name that holds nothing"
    );

    let (_peer_temp, mut peer) = bare_store();
    let pending = store
        .list_pending_sync_mutations("cloud", &["canonical-project".to_owned()], 100)
        .unwrap();
    for mutation in &pending {
        peer.apply_pulled(mutation).unwrap();
    }
    let deleted_at: Option<String> = peer
        .connection
        .query_row(
            "SELECT deleted_at FROM observations WHERE sync_id = ?1",
            [&removed.sync_id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        deleted_at.is_some(),
        "the peer that took the whole queue ends with the memory deleted"
    );
}

/// A verdict that had not been sent follows the memories it names.
#[test]
fn merging_a_project_keeps_the_verdict_that_was_waiting_for_a_peer() {
    let (_temp, mut store) = enrolled_store(&["old-project", "canonical-project"]);
    store.create_session("s1", "old-project", "C:/old").unwrap();
    let mut saved = Vec::new();
    for title in ["First decision", "Second decision"] {
        let mut memory = observation("s1", title, title);
        memory.project = Some("old-project".to_owned());
        saved.push(store.add_observation(memory).unwrap().observation);
    }
    let relation = store
        .judge_by_semantic(JudgeBySemanticParams {
            source_id: saved[0].sync_id.clone(),
            target_id: saved[1].sync_id.clone(),
            relation: "supersedes".to_owned(),
            confidence: None,
            reasoning: None,
            model: None,
        })
        .unwrap();

    store
        .merge_projects(&["old-project".to_owned()], "canonical-project")
        .unwrap();

    let waiting = unsent_under(&store, "canonical-project");
    let verdict = waiting
        .iter()
        .find(|(entity, _, _)| entity == "relation")
        .unwrap_or_else(|| panic!("the verdict was dropped: {waiting:?}"));
    assert_eq!(verdict.2["sync_id"], relation);
    assert_eq!(verdict.2["project"], "canonical-project");
}

/// A refusal belongs to a name that no longer replicates once it is merged
/// away, and the status must not go on naming it.
///
/// The merge carries the queue to the canonical name and leaves the retired
/// name's `cloud:<name>` row behind. Nothing under that name can be sent again,
/// so a `refused` verdict on it is a state no batch could clear — it outlived
/// the project it described, on every status and every TUI load.
#[test]
fn merging_a_refused_project_away_stops_naming_it_in_the_status() {
    let (_temp, mut store) = enrolled_store(&["old-project", "canonical-project"]);
    store.create_session("s1", "old-project", "C:/old").unwrap();
    let mut memory = observation("s1", "Waiting under the old name", "body");
    memory.project = Some("old-project".to_owned());
    store.add_observation(memory).unwrap();
    store
        .mark_project_sync_refused(
            "cloud",
            "old-project",
            "sync-paused",
            "sync is paused for project \"old-project\"",
            chrono::Utc::now() + chrono::Duration::hours(1),
        )
        .unwrap();

    store
        .merge_projects(&["old-project".to_owned()], "canonical-project")
        .unwrap();

    let enrolled = store.enrolled_projects().unwrap();
    let statuses = store.project_sync_statuses("cloud", &enrolled).unwrap();
    assert!(
        statuses
            .iter()
            .all(|status| status.project != "old-project"),
        "the retired name outlived the merge: {statuses:?}"
    );
    let canonical = statuses
        .iter()
        .find(|status| status.project == "canonical-project")
        .unwrap_or_else(|| panic!("the name that took the queue is missing: {statuses:?}"));
    assert_eq!(canonical.lifecycle, PROJECT_SYNC_PENDING);
    assert!(canonical.pending_mutations > 0);
}

/// So does the version a content change had replaced.
#[test]
fn merging_a_project_keeps_the_version_that_was_waiting_for_a_peer() {
    let (_temp, mut store) = enrolled_store(&["old-project", "canonical-project"]);
    store.create_session("s1", "old-project", "C:/old").unwrap();
    let mut memory = observation("s1", "Rewritten", "the first text");
    memory.project = Some("old-project".to_owned());
    let saved = store.add_observation(memory).unwrap().observation;
    store
        .update_observation(
            saved.id,
            None,
            UpdateObservation {
                content: Some("the second text".to_owned()),
                ..UpdateObservation::default()
            },
        )
        .unwrap();

    store
        .merge_projects(&["old-project".to_owned()], "canonical-project")
        .unwrap();

    let waiting = unsent_under(&store, "canonical-project");
    let version = waiting
        .iter()
        .find(|(entity, _, _)| entity == "observation_version")
        .unwrap_or_else(|| panic!("the version was dropped: {waiting:?}"));
    assert_eq!(version.2["content"], "the first text");
}

/// A memory that is still live is queued once, not twice.
///
/// The unsent upsert of a row the merge has just queued again is the one thing
/// that is dropped: carrying it too would send every moved memory twice.
#[test]
fn merging_a_project_does_not_send_a_live_memory_twice() {
    let (_temp, mut store) = enrolled_store(&["old-project", "canonical-project"]);
    store.create_session("s1", "old-project", "C:/old").unwrap();
    let mut memory = observation("s1", "Still here", "body");
    memory.project = Some("old-project".to_owned());
    store.add_observation(memory).unwrap();

    store
        .merge_projects(&["old-project".to_owned()], "canonical-project")
        .unwrap();

    let observations = unsent_under(&store, "canonical-project")
        .into_iter()
        .filter(|(entity, _, _)| entity == "observation")
        .count();
    assert_eq!(observations, 1);
}

/// A name nobody replicates has nowhere to send what was waiting, so it goes
/// rather than being filed under a project no peer follows.
#[test]
fn merging_into_a_project_nobody_replicates_drops_what_was_waiting() {
    let (_temp, mut store) = enrolled_store(&["old-project"]);
    store.create_session("s1", "old-project", "C:/old").unwrap();
    let mut memory = observation("s1", "Saved before it was left", "body");
    memory.project = Some("old-project".to_owned());
    store.add_observation(memory).unwrap();
    assert!(
        !unsent_under(&store, "old-project").is_empty(),
        "the fixture needs something waiting"
    );
    store
        .connection
        .execute(
            "DELETE FROM sync_enrolled_projects WHERE project = 'old-project'",
            [],
        )
        .unwrap();

    store
        .merge_projects(&["old-project".to_owned()], "private-project")
        .unwrap();

    assert!(unsent_under(&store, "old-project").is_empty());
    assert!(
        unsent_under(&store, "private-project").is_empty(),
        "what was waiting was filed under a project no peer follows"
    );
}

/// Moving a directory's sessions out of a name that is left empty carries the
/// delete the same way, because it is the same arrangement for fewer rows.
#[test]
fn moving_the_last_directory_of_a_project_keeps_its_unsent_delete() {
    let (_temp, mut store) = enrolled_store(&["api"]);
    store.create_session("here", "api", "/work/api").unwrap();
    let mut memory = observation("here", "Removed", "gone");
    memory.project = Some("api".to_owned());
    let removed = store.add_observation(memory).unwrap().observation;
    store.delete_observation(removed.id, None, false).unwrap();

    store
        .move_directory_sessions("api", "backend", std::path::Path::new("/work/api"))
        .unwrap();

    let waiting = unsent_under(&store, "backend");
    assert!(
        waiting
            .iter()
            .any(|(entity, op, payload)| entity == "observation"
                && op == "delete"
                && payload["project"] == "backend"),
        "the delete was dropped: {waiting:?}"
    );
    assert!(unsent_under(&store, "api").is_empty());
}

/// Enrolling again does not discard a delete that nobody has been told about.
///
/// The catch-up queues what the project holds now, which says nothing about a
/// row that is gone, so the unsent delete was the only record of it.
#[test]
fn enrolling_a_project_again_keeps_the_delete_it_had_not_sent() {
    let (_temp, mut store) = store();
    store.enroll_project("again").unwrap();
    store.create_session("s1", "again", "C:/again").unwrap();
    let mut memory = observation("s1", "Removed", "gone");
    memory.project = Some("again".to_owned());
    let removed = store.add_observation(memory).unwrap().observation;
    store.delete_observation(removed.id, None, false).unwrap();
    store.unenroll_project("again").unwrap();

    assert!(store.enroll_project("again").unwrap());

    let waiting = unsent_under(&store, "again");
    assert!(
        waiting
            .iter()
            .any(|(entity, op, _)| entity == "observation" && op == "delete"),
        "the delete was dropped: {waiting:?}"
    );
}

/// The catch-up carries a memory's history with it.
///
/// A memory edited before its project was enrolled reached the peer with its
/// current text and none of the versions the live journal queues beside each
/// edit.
#[test]
fn enrolling_a_project_queues_the_versions_of_what_it_holds() {
    let (_temp, mut store) = bare_store();
    store.create_session("s1", "late", "C:/late").unwrap();
    let mut memory = observation("s1", "Edited early", "the first text");
    memory.project = Some("late".to_owned());
    let saved = store.add_observation(memory).unwrap().observation;
    store
        .update_observation(
            saved.id,
            None,
            UpdateObservation {
                content: Some("the second text".to_owned()),
                ..UpdateObservation::default()
            },
        )
        .unwrap();

    store.enroll_project("late").unwrap();

    let waiting = unsent_under(&store, "late");
    let positions: Vec<usize> = ["observation", "observation_version"]
        .iter()
        .map(|wanted| {
            waiting
                .iter()
                .position(|(entity, _, _)| entity == wanted)
                .unwrap_or_else(|| panic!("no {wanted} was queued: {waiting:?}"))
        })
        .collect();
    assert!(
        positions[0] < positions[1],
        "the memory goes first: a peer applies the queue in order"
    );
    let version = &waiting[positions[1]].2;
    assert_eq!(version["content"], "the first text");
    assert_eq!(version["sync_id"], saved.sync_id);
}

/// Enrolling a project does not teach a peer about a memory it will never get.
///
/// The backfill queues the graph as well as the memories, and its own comment
/// says what it queues: relations pointing at two live memories *of this
/// project*. The condition only asked about the source. So a relation whose
/// target had moved to another project went out under this one, and the far
/// end — which counts the two memories a relation names before applying it —
/// files it in the deferred table, where it waits for a memory that is never
/// coming, on every retry, for ever.
///
/// Nothing strange is needed to get there: a pair is proposed inside one
/// project, judged there, and one of its memories is moved afterwards.
#[test]
fn a_relation_whose_other_half_left_the_project_is_not_queued_with_it() {
    let (_temp, mut store) = store();
    store.create_session("a", "proyecto-a", "C:/a").unwrap();
    let mut first = observation("a", "The first one", "a memory of project a");
    first.project = Some("proyecto-a".to_owned());
    let first = store.add_observation(first).unwrap().observation;
    let mut second = observation("a", "The second one", "another memory of project a");
    second.project = Some("proyecto-a".to_owned());
    let second = store.add_observation(second).unwrap().observation;
    let mut stays = observation("a", "The third one", "and one that does not move");
    stays.project = Some("proyecto-a".to_owned());
    let stays = store.add_observation(stays).unwrap().observation;

    // Two verdicts inside the project, which is the only place they can be made.
    for target in [&second, &stays] {
        store
            .judge_by_semantic(JudgeBySemanticParams {
                source_id: first.sync_id.clone(),
                target_id: target.sync_id.clone(),
                relation: "related".to_owned(),
                confidence: None,
                reasoning: None,
                model: None,
            })
            .unwrap();
    }
    store.enroll_project("proyecto-b").unwrap();
    store
        .update_observation(
            second.id,
            None,
            UpdateObservation {
                project: Some("proyecto-b".to_owned()),
                ..UpdateObservation::default()
            },
        )
        .unwrap();

    store.enroll_project("proyecto-a").unwrap();
    let queued: Vec<String> = store
        .connection
        .prepare(
            "SELECT payload FROM sync_mutations
             WHERE entity = 'relation' AND project = 'proyecto-a'",
        )
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        queued.len(),
        1,
        "only the pair whose two memories are both here: {queued:?}"
    );
    assert!(
        queued[0].contains(&stays.sync_id),
        "and it is the one that stayed: {}",
        queued[0]
    );
    assert!(
        !queued[0].contains(&second.sync_id),
        "the memory that left is not named to this project's peer"
    );
}

/// An export is this store written down, field for field.
///
/// The existing round trip checks how many rows came back, which is what let
/// pinning disappear: `#[serde(skip)]` kept it out of the wire *and* out of an
/// export, so `leteo export` followed by `leteo import` came back with every
/// pin lost, silently, while the import statement had always had a column ready
/// for it. Counting rows cannot see that. Nor can it see a column added to the
/// table tomorrow and left out of the import's own list of column names, which
/// is the same failure with a different field.
///
/// So this populates every field a memory can carry, sends it through the JSON,
/// and compares the two memories whole. The list of key names is written out
/// deliberately: it is a change-detector, not a second copy of the model, and a
/// field added to `Observation` should make somebody decide here whether it
/// travels.
#[test]
fn a_round_trip_brings_back_every_field_a_memory_carries() {
    let (_source_temp, mut source) = store();
    source.create_session("s1", "leteo", "C:/repo").unwrap();
    source
        .add_prompt(AddPrompt {
            session_id: "s1".to_owned(),
            content: "la pregunta que motivo la memoria".to_owned(),
            project: Some("leteo".to_owned()),
        })
        .unwrap();
    let asked = source.recent_prompts(Some("leteo"), Some(1)).unwrap()[0]
        .sync_id
        .clone();

    let mut input = observation("s1", "Una decision", "el cuerpo de la decision");
    input.kind = "decision".to_owned();
    input.tool_name = Some("una-herramienta".to_owned());
    input.topic_key = Some("decision/exportada".to_owned());
    input.scope = "personal".to_owned();
    input.prompt_sync_id = Some(asked.clone());
    let saved = source.add_observation(input.clone()).unwrap().observation;
    // Revised and duplicated, so both counters are past their defaults, and
    // pinned and reviewed so `pinned` and `review_after` are set.
    source.add_observation(input.clone()).unwrap();
    source.add_observation(input).unwrap();
    source.pin_observation(saved.id, None).unwrap();
    source.mark_reviewed(saved.id, None).unwrap();
    // And one deleted memory, because an export carries those too and a backup
    // that forgets a deletion resurrects it on the next import.
    let removed = source
        .add_observation(observation("s1", "Borrada", "y exportada igual"))
        .unwrap()
        .observation;
    source.delete_observation(removed.id, None, false).unwrap();

    let json = source.export_json(None).unwrap();
    let (_destination_temp, mut destination) = store();
    destination.import_json(&json).unwrap();

    let same = |store: &Store, sync_id: &str| -> crate::memory::model::Observation {
        let found = store
            .search(
                "decision",
                SearchOptions {
                    scope: Some("personal".to_owned()),
                    ..SearchOptions::default()
                },
            )
            .unwrap();
        found
            .into_iter()
            .map(|result| result.observation)
            .find(|observation| observation.sync_id == sync_id)
            .unwrap_or_else(|| panic!("{sync_id} did not survive the round trip"))
    };
    let before = same(&source, &saved.sync_id);
    let after = same(&destination, &saved.sync_id);
    // The row id belongs to the store that issued it; everything else is the
    // memory and has to arrive unchanged.
    assert_eq!(
        crate::memory::model::Observation {
            id: 0,
            ..before.clone()
        },
        crate::memory::model::Observation { id: 0, ..after },
        "a field went missing between export and import"
    );
    // The fixture has to have set them, or the comparison above is two defaults
    // agreeing with each other.
    assert!(before.pinned, "{before:?}");
    assert!(before.review_after.is_some(), "{before:?}");
    assert_eq!(before.prompt_sync_id.as_deref(), Some(asked.as_str()));
    assert_eq!(before.tool_name.as_deref(), Some("una-herramienta"));
    assert_eq!(before.topic_key.as_deref(), Some("decision/exportada"));
    assert_eq!(before.scope, "personal");
    assert!(
        before.revision_count > 1 || before.duplicate_count > 1,
        "{before:?}"
    );

    // The deletion travelled rather than being dropped on the way.
    let raw: Option<String> = destination
        .connection()
        .query_row(
            "SELECT deleted_at FROM observations WHERE sync_id = ?1",
            rusqlite::params![removed.sync_id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(raw.is_some(), "a deleted memory came back alive");

    // The other three entities travel by the same route and have their own
    // hand-written column lists in the import statement. A memory that arrives
    // whole into a session that lost its summary is still a lossy backup.
    source
        .end_session("s1", Some("lo que hizo esta sesion"))
        .unwrap();
    let json = source.export_json(None).unwrap();
    let (_second_temp, mut second) = store();
    second.import_json(&json).unwrap();
    assert_eq!(
        source.get_session("s1").unwrap(),
        second.get_session("s1").unwrap(),
        "the session did not survive the round trip"
    );
    let prompt_before = source.recent_prompts(Some("leteo"), Some(1)).unwrap();
    let prompt_after = second.recent_prompts(Some("leteo"), Some(1)).unwrap();
    assert_eq!(
        crate::memory::model::Prompt {
            id: 0,
            ..prompt_before[0].clone()
        },
        crate::memory::model::Prompt {
            id: 0,
            ..prompt_after
                .first()
                .cloned()
                .expect("the prompt did not survive")
        },
        "the prompt did not survive the round trip"
    );

    // And every field the model serialises is in the JSON, so a comparison
    // cannot pass by both sides skipping the same absent value.
    let exported: serde_json::Value = serde_json::from_str(&json).unwrap();
    let carried = exported["observations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|value| value["sync_id"] == saved.sync_id.as_str())
        .expect("the memory is in the export");
    for key in [
        "id",
        "sync_id",
        "session_id",
        "type",
        "title",
        "content",
        "tool_name",
        "project",
        "scope",
        "topic_key",
        "revision_count",
        "duplicate_count",
        "last_seen_at",
        "review_after",
        "prompt_sync_id",
        "pinned",
        "created_at",
        "updated_at",
    ] {
        assert!(
            carried.get(key).is_some(),
            "the export left out {key}; if that is deliberate, say so here"
        );
    }
}

/// A merge says what it left sharing a name.
///
/// A topic key holds one live memory per project and scope, and revising one
/// finds it by exactly that triple — so saving cannot produce two. A merge can:
/// each project may have had its own memory under one key, legitimately, and
/// afterwards they share it. Nothing is lost, but the revision path takes the
/// most recently updated of the two and the other stops being reachable by its
/// own key for good.
///
/// Reported rather than resolved, because which of the two is the memory and
/// which is the twin is a judgment about their contents, and a merge that threw
/// one away silently would be the worse answer. Found by driving four hundred
/// operations over three project names in an order nobody wrote by hand.
#[test]
fn a_merge_says_how_many_topic_keys_it_left_shared() {
    let (_temp, mut store) = store();
    for (session, project) in [("s1", "leteo"), ("s2", "leteo cloud")] {
        store.create_session(session, project, "C:/repo").unwrap();
        let mut input = observation(session, &format!("Memoria de {project}"), "un cuerpo");
        input.project = Some(project.to_owned());
        input.topic_key = Some("architecture/una-clave".to_owned());
        store.add_observation(input).unwrap();
    }
    let merged = store
        .merge_projects(&["leteo cloud".to_owned()], "leteo")
        .unwrap();
    assert_eq!(merged.observations_updated, 1);
    assert_eq!(
        merged.topic_key_collisions, 1,
        "the merge put two memories under one key and has to say so"
    );

    let held: i64 = store
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM observations WHERE deleted_at IS NULL AND topic_key = ?1",
            rusqlite::params!["architecture/una-clave"],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(held, 2);

    store.create_session("s3", "tercero", "C:/repo").unwrap();
    let mut input = observation("s3", "Sin clave compartida", "otro cuerpo");
    input.project = Some("tercero".to_owned());
    store.add_observation(input).unwrap();
    let quiet = store
        .merge_projects(&["tercero".to_owned()], "leteo")
        .unwrap();
    assert_eq!(
        quiet.topic_key_collisions, 1,
        "the pair from before is still shared"
    );
}

/// Two project lists, two questions, and both are described.
///
/// `stats` answers where anything has been happening: projects holding at
/// least one live memory, most recently written first. `projects list` is the
/// inventory. On a real store they differ by two of nineteen — a project with
/// only a session and one with only a prompt — and a bare `projects` beside
/// three totals reads as the inventory, which is why the field says what it is.
#[test]
fn the_two_project_lists_answer_two_different_questions() {
    let (_temp, mut store) = store();
    store
        .create_session("kept", "con-memoria", "C:/repo")
        .unwrap();
    store
        .add_observation(AddObservation {
            project: Some("con-memoria".to_owned()),
            ..observation("kept", "Una memoria", "un cuerpo")
        })
        .unwrap();
    // A project the store knows and no memory belongs to.
    store
        .create_session("bare", "sin-memoria", "C:/otro")
        .unwrap();

    let stats = store.stats().unwrap();
    assert_eq!(
        stats.projects,
        vec!["con-memoria".to_owned()],
        "stats names where the memories are"
    );
    let inventory: Vec<String> = store
        .list_projects_with_stats()
        .unwrap()
        .into_iter()
        .map(|project| project.name)
        .collect();
    assert!(
        inventory.contains(&"sin-memoria".to_owned()),
        "the inventory names a project with only a session: {inventory:?}"
    );
    assert!(
        inventory.contains(&"con-memoria".to_owned()),
        "{inventory:?}"
    );
}

/// A merge into a name the store never held says so.
///
/// Merging into a new name is a rename, and there is no other way to perform
/// one, so it is allowed — and it is also exactly what a typo in `to` looks
/// like. The two are the same call: a whole project walks into a misspelling,
/// the reply reports success, and the memories are findable only under the
/// mistake. Every other write refuses a project name nobody invented, and
/// `project_exists` was written for that; this path never asked it.
///
/// Reported rather than refused, for the reason the topic-key collisions
/// beside it are: which of the two this was is the caller's to know, and
/// refusing would remove the only way to rename a project.
#[test]
fn a_merge_that_invents_the_canonical_project_says_it_did() {
    let (_temp, mut store) = super::store();
    for proyecto in ["uno", "dos"] {
        store
            .create_session(&format!("s-{proyecto}"), proyecto, "C:/repo")
            .unwrap();
        store
            .add_observation(crate::memory::model::AddObservation {
                session_id: format!("s-{proyecto}"),
                kind: "discovery".to_owned(),
                title: format!("Una memoria de {proyecto} con titulo suficiente"),
                content: "Un cuerpo con texto de sobra.".to_owned(),
                tool_name: None,
                project: Some(proyecto.to_owned()),
                scope: "project".to_owned(),
                topic_key: None,
                prompt_sync_id: None,
            })
            .unwrap();
    }

    let ordinario = store.merge_projects(&["uno".to_owned()], "dos").unwrap();
    assert_eq!(ordinario.sources_merged, vec!["uno".to_owned()]);
    assert!(!ordinario.canonical_created, "{ordinario:?}");

    let renombrado = store
        .merge_projects(&["dos".to_owned()], "proyecto-con-erratta")
        .unwrap();
    assert!(renombrado.canonical_created, "{renombrado:?}");
    assert_eq!(renombrado.observations_updated, 2, "{renombrado:?}");

    // And where nothing moved, the event is not invented: asking to merge a name
    // nobody holds into another name nobody holds creates no project at all.
    let vacio = store
        .merge_projects(&["fantasma-a".to_owned()], "fantasma-b")
        .unwrap();
    assert!(vacio.sources_merged.is_empty(), "{vacio:?}");
    assert!(!vacio.canonical_created, "{vacio:?}");
}

/// Hands everything `store` still has queued for `project` to `peer`, in order,
/// and marks it sent, which is what a sync round does.
fn deliver(store: &mut Store, peer: &mut Store, project: &str) {
    let pending = store
        .list_pending_sync_mutations("cloud", &[project.to_owned()], 100)
        .unwrap();
    for mutation in &pending {
        peer.apply_pulled(mutation).unwrap_or_else(|error| {
            panic!(
                "seq {} ({}) could not be applied at the peer: {error}",
                mutation.seq, mutation.entity
            )
        });
    }
    let sequences: Vec<i64> = pending.iter().map(|mutation| mutation.seq).collect();
    store.ack_sync_mutation_seqs("cloud", &sequences).unwrap();
}

fn project_of_observation(store: &Store, sync_id: &str) -> (String, bool) {
    store
        .connection
        .query_row(
            "SELECT project, deleted_at IS NOT NULL FROM observations WHERE sync_id = ?1",
            [sync_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap()
}

/// A delete that was filed before a merge reaches the peer that has not merged.
///
/// The peer holds the memory under the old name because it received it before
/// the merge. The merge rewrites the unsent delete to the new name, and a peer
/// applies a delete only to a row filed under the delete's project, so the
/// delete was ignored for good and the memory outlived its owner's decision.
/// What fixes it is the merge reaching the peer as a rename, ahead of the
/// delete.
#[test]
fn a_delete_filed_before_a_merge_reaches_a_peer_that_never_merged() {
    let (_temp, mut store) = enrolled_store(&["old-project", "canonical-project"]);
    store.create_session("s1", "old-project", "C:/old").unwrap();
    let mut memory = observation("s1", "Sent and then removed", "gone");
    memory.project = Some("old-project".to_owned());
    let saved = store.add_observation(memory).unwrap().observation;
    let (_peer_temp, mut peer) = enrolled_store(&["old-project"]);
    deliver(&mut store, &mut peer, "old-project");
    assert_eq!(
        project_of_observation(&peer, &saved.sync_id),
        ("old-project".to_owned(), false)
    );

    store.delete_observation(saved.id, None, false).unwrap();
    store
        .merge_projects(&["old-project".to_owned()], "canonical-project")
        .unwrap();
    deliver(&mut store, &mut peer, "canonical-project");

    assert_eq!(
        project_of_observation(&peer, &saved.sync_id),
        ("canonical-project".to_owned(), true),
        "the peer files the memory under the new name and deletes it there"
    );
}

/// A delete made after the merge finds a row the peer has not moved yet.
#[test]
fn a_delete_filed_after_a_merge_finds_the_row_on_a_peer_that_never_merged() {
    let (_temp, mut store) = enrolled_store(&["old-project", "canonical-project"]);
    store.create_session("s1", "old-project", "C:/old").unwrap();
    let mut memory = observation("s1", "Removed later", "gone");
    memory.project = Some("old-project".to_owned());
    let saved = store.add_observation(memory).unwrap().observation;
    let mut kept = observation("s1", "Kept", "stays");
    kept.project = Some("old-project".to_owned());
    let kept = store.add_observation(kept).unwrap().observation;
    let (_peer_temp, mut peer) = enrolled_store(&["old-project"]);
    deliver(&mut store, &mut peer, "old-project");

    store
        .merge_projects(&["old-project".to_owned()], "canonical-project")
        .unwrap();
    store.delete_observation(saved.id, None, false).unwrap();
    // Only the rename and what follows it: a peer that took the merge's own
    // re-queued rows would have been moved by them, and not by the rename.
    let pending = store
        .list_pending_sync_mutations("cloud", &["canonical-project".to_owned()], 100)
        .unwrap();
    let rename = pending
        .iter()
        .find(|mutation| mutation.entity == crate::sync::ENTITY_PROJECT_RENAME)
        .unwrap_or_else(|| panic!("the merge did not queue a rename: {pending:?}"));
    let delete = pending
        .iter()
        .find(|mutation| mutation.entity == "observation" && mutation.op == "delete")
        .unwrap();
    assert!(rename.seq < delete.seq);
    peer.apply_pulled(rename).unwrap();
    peer.apply_pulled(delete).unwrap();

    assert_eq!(
        project_of_observation(&peer, &saved.sync_id),
        ("canonical-project".to_owned(), true)
    );
    assert_eq!(
        project_of_observation(&peer, &kept.sync_id),
        ("canonical-project".to_owned(), false),
        "the rename moved what was not deleted too"
    );
}

/// A rename moves every kind of row a project holds, and says nothing back.
#[test]
fn a_rename_moves_sessions_and_prompts_and_queues_nothing_at_the_peer() {
    let (_temp, mut store) = enrolled_store(&["old-project", "canonical-project"]);
    store.create_session("s1", "old-project", "C:/old").unwrap();
    store
        .add_prompt(AddPrompt {
            session_id: "s1".to_owned(),
            content: "asked before the rename".to_owned(),
            project: Some("old-project".to_owned()),
        })
        .unwrap();
    let (_peer_temp, mut peer) = enrolled_store(&["old-project", "canonical-project"]);
    deliver(&mut store, &mut peer, "old-project");

    store
        .merge_projects(&["old-project".to_owned()], "canonical-project")
        .unwrap();
    let rename = store
        .list_pending_sync_mutations("cloud", &["canonical-project".to_owned()], 100)
        .unwrap()
        .into_iter()
        .find(|mutation| mutation.entity == crate::sync::ENTITY_PROJECT_RENAME)
        .unwrap();
    let queued_before = peer
        .list_pending_sync_mutations("cloud", &["*".to_owned()], 100)
        .unwrap()
        .len();
    peer.apply_pulled(&rename).unwrap();

    for table in ["sessions", "prompts"] {
        let stale: i64 = peer
            .connection
            .query_row(
                &format!("SELECT COUNT(*) FROM {table} WHERE project = 'old-project'"),
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stale, 0, "{table} still carries the old name");
    }
    assert_eq!(
        peer.list_pending_sync_mutations("cloud", &["*".to_owned()], 100)
            .unwrap()
            .len(),
        queued_before,
        "applying a rename is not a write of this store's own, and is not sent back"
    );
    assert!(
        !peer
            .enrolled_projects()
            .unwrap()
            .contains(&"old-project".to_owned()),
        "a name that holds nothing is no longer replicated"
    );
}

/// The store that sent a rename gets it back and is not moved by it again.
#[test]
fn a_store_does_not_apply_the_rename_it_sent() {
    let (_temp, mut store) = enrolled_store(&["old-project", "canonical-project"]);
    store.create_session("s1", "old-project", "C:/old").unwrap();
    let mut memory = observation("s1", "Moved", "body");
    memory.project = Some("old-project".to_owned());
    store.add_observation(memory).unwrap();
    store
        .merge_projects(&["old-project".to_owned()], "canonical-project")
        .unwrap();
    let rename = store
        .list_pending_sync_mutations("cloud", &["canonical-project".to_owned()], 100)
        .unwrap()
        .into_iter()
        .find(|mutation| mutation.entity == crate::sync::ENTITY_PROJECT_RENAME)
        .unwrap();

    // The old name is in use again by the time the push comes back.
    store.create_session("s2", "old-project", "C:/old").unwrap();
    let mut again = observation("s2", "Filed under the old name again", "body");
    again.project = Some("old-project".to_owned());
    let again = store.add_observation(again).unwrap().observation;
    let mut echoed = rename.clone();
    echoed.seq = i64::MAX / 2;
    echoed.source = "remote".to_owned();
    store.apply_pulled(&echoed).unwrap();

    assert_eq!(
        project_of_observation(&store, &again.sync_id).0,
        "old-project"
    );
}

fn rename_from_peer(sources: &[&str], project: &str) -> SyncMutation {
    SyncMutation {
        seq: 1,
        target_key: "cloud".to_owned(),
        entity: crate::sync::ENTITY_PROJECT_RENAME.to_owned(),
        entity_key: "rename-from-a-peer".to_owned(),
        op: crate::sync::OP_UPSERT.to_owned(),
        payload: serde_json::json!({
            "sync_id": "rename-from-a-peer",
            "sources": sources,
            "project": project,
        })
        .to_string(),
        source: "remote".to_owned(),
        project: project.to_owned(),
        occurred_at: String::new(),
        acked_at: None,
    }
}

fn memory_in(store: &mut Store, session: &str, project: &str) -> String {
    store.create_session(session, project, "C:/any").unwrap();
    let mut memory = observation(session, "Held here", "body");
    memory.project = Some(project.to_owned());
    store.add_observation(memory).unwrap().observation.sync_id
}

/// A rename a peer sent moves only the names this store replicates.
///
/// The names in it are the sender's word: a project that exists only on this
/// machine, and that was never shared, would otherwise be filed under the
/// shared one and queued to every peer from then on.
#[test]
fn a_rename_from_a_peer_does_not_move_a_project_this_store_never_shared() {
    let (_temp, mut store) = enrolled_store(&["shared-old"]);
    let shared = memory_in(&mut store, "s1", "shared-old");
    let private = memory_in(&mut store, "s2", "private-notes");

    store
        .apply_pulled(&rename_from_peer(
            &["shared-old", "private-notes"],
            "shared-new",
        ))
        .unwrap();

    assert_eq!(
        project_of_observation(&store, &shared).0,
        "shared-new",
        "the name this store replicates still moves"
    );
    assert_eq!(
        project_of_observation(&store, &private).0,
        "private-notes",
        "a project that was never shared was filed under the shared one"
    );
    let sessions: i64 = store
        .connection
        .query_row(
            "SELECT COUNT(*) FROM sessions WHERE project = 'private-notes'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(sessions, 1);
    assert!(
        !store
            .enrolled_projects()
            .unwrap()
            .contains(&"private-notes".to_owned())
    );
}

/// A rename whose every source was skipped leaves the destination alone.
#[test]
fn a_rename_that_names_only_unshared_projects_enrols_nothing() {
    let (_temp, mut store) = bare_store();
    let private = memory_in(&mut store, "s1", "private-notes");

    store
        .apply_pulled(&rename_from_peer(&["private-notes"], "shared-new"))
        .unwrap();

    assert_eq!(project_of_observation(&store, &private).0, "private-notes");
    assert!(
        store.enrolled_projects().unwrap().is_empty(),
        "a rename that moved nothing enrolled its destination"
    );
}

/// What was waiting under another spelling of the name is carried too.
///
/// The merge folds `old_project` along with `old-project`, and it only carried
/// the unsent mutations of the name it was asked about, so a delete waiting
/// under a spelling stayed filed under a name that now holds nothing.
///
/// The spelling is not a live project here: its only memory is soft-deleted
/// and it was unenroled after the delete was queued. A spelling a store still
/// replicates or still holds a live row under is a project, and the merge
/// leaves it alone.
#[test]
fn merging_a_project_carries_what_was_waiting_under_its_other_spellings() {
    let (_temp, mut store) = enrolled_store(&["old_project", "canonical-project"]);
    store.create_session("s1", "old-project", "C:/old").unwrap();
    let mut memory = observation("s1", "Removed under a spelling", "gone");
    memory.project = Some("old_project".to_owned());
    let removed = store.add_observation(memory).unwrap().observation;
    store.delete_observation(removed.id, None, false).unwrap();
    store.unenroll_project("old_project").unwrap();

    store
        .merge_projects(&["old-project".to_owned()], "canonical-project")
        .unwrap();

    assert!(
        unsent_under(&store, "old_project").is_empty(),
        "nothing is left waiting under a name that holds nothing"
    );
    assert!(
        unsent_under(&store, "canonical-project")
            .iter()
            .any(|(entity, op, _)| entity == "observation" && op == "delete"),
        "the delete follows the memory"
    );
    assert!(
        !store
            .enrolled_projects()
            .unwrap()
            .contains(&"old_project".to_owned()),
        "and the emptied spelling stops being replicated"
    );
}

/// The versions a memory already sent travel with it to its new name.
///
/// A peer that starts replicating the new name reads that name's mutations and
/// nothing else; the versions were queued under the old one, so it received the
/// memories with none of their history. The catch-up of an enrolment queues
/// them, and the merge is the same move.
#[test]
fn merging_a_project_queues_again_the_versions_a_memory_had_already_sent() {
    let (_temp, mut store) = enrolled_store(&["old-project", "canonical-project"]);
    store.create_session("s1", "old-project", "C:/old").unwrap();
    let mut memory = observation("s1", "Rewritten", "the first text");
    memory.project = Some("old-project".to_owned());
    let saved = store.add_observation(memory).unwrap().observation;
    store
        .update_observation(
            saved.id,
            None,
            UpdateObservation {
                content: Some("the second text".to_owned()),
                ..UpdateObservation::default()
            },
        )
        .unwrap();
    let (_peer_temp, mut peer) = bare_store();
    deliver(&mut store, &mut peer, "old-project");
    assert!(unsent_under(&store, "old-project").is_empty());

    store
        .merge_projects(&["old-project".to_owned()], "canonical-project")
        .unwrap();

    let versions: Vec<_> = unsent_under(&store, "canonical-project")
        .into_iter()
        .filter(|(entity, _, _)| entity == "observation_version")
        .collect();
    assert_eq!(versions.len(), 1, "{versions:?}");
    assert_eq!(versions[0].2["content"], "the first text");
}

/// A version still waiting is not sent twice by being carried and queued.
#[test]
fn merging_a_project_does_not_send_a_waiting_version_twice() {
    let (_temp, mut store) = enrolled_store(&["old-project", "canonical-project"]);
    store.create_session("s1", "old-project", "C:/old").unwrap();
    let mut memory = observation("s1", "Rewritten", "the first text");
    memory.project = Some("old-project".to_owned());
    let saved = store.add_observation(memory).unwrap().observation;
    store
        .update_observation(
            saved.id,
            None,
            UpdateObservation {
                content: Some("the second text".to_owned()),
                ..UpdateObservation::default()
            },
        )
        .unwrap();

    store
        .merge_projects(&["old-project".to_owned()], "canonical-project")
        .unwrap();

    let versions = unsent_under(&store, "canonical-project")
        .into_iter()
        .filter(|(entity, _, _)| entity == "observation_version")
        .count();
    assert_eq!(versions, 1);
}

/// Where a session was sent is where its later changes go.
///
/// A session is queued under its own project, and again under the project of a
/// memory filed into it from elsewhere. Ending it, or deleting it, only said so
/// to the first, so the peer that follows the memory's project kept a session
/// that had never ended, and an orphan row after the session was deleted.
#[test]
fn ending_a_session_tells_every_project_it_was_sent_to() {
    let (_temp, mut store) = store();
    store.create_session("s1", "elsewhere", "/tmp/x").unwrap();
    store
        .add_observation(observation("s1", "Filed away from its session", "a body"))
        .unwrap();

    store.end_session("s1", Some("what was done")).unwrap();

    let sessions: Vec<_> = unsent_under(&store, "leteo")
        .into_iter()
        .filter(|(entity, _, _)| entity == "session")
        .collect();
    assert!(
        sessions
            .iter()
            .any(|(_, _, payload)| payload["summary"] == "what was done"),
        "the update never reached the project that follows the memory: {sessions:?}"
    );
}

#[test]
fn deleting_a_session_tells_every_project_it_was_sent_to() {
    let (_temp, mut store) = store();
    store.create_session("s1", "elsewhere", "/tmp/x").unwrap();
    store
        .add_observation(observation("s1", "Filed away from its session", "a body"))
        .unwrap();

    store.delete_session_and_contents("s1", true).unwrap();

    assert!(
        unsent_under(&store, "leteo")
            .iter()
            .any(|(entity, op, _)| entity == "session" && op == "delete"),
        "the peer that follows the memory's project keeps the session row"
    );
}

/// A project holding only soft-deleted memories is not a prune candidate.
#[test]
fn a_project_holding_a_soft_deleted_memory_names_why_it_cannot_be_pruned() {
    let (_temp, mut store) = store();
    store.create_session("s1", "soft", "C:/soft").unwrap();
    let mut memory = observation("s1", "Removed softly", "gone");
    memory.project = Some("soft".to_owned());
    let saved = store.add_observation(memory).unwrap().observation;
    store.delete_observation(saved.id, None, false).unwrap();
    store.create_session("s2", "empty", "C:/empty").unwrap();

    let blocker = store.prune_blocker("soft").unwrap();
    assert!(
        blocker.as_deref().is_some_and(|text| text.contains("soft")),
        "{blocker:?}"
    );
    assert!(store.prune_project("soft").is_err());
    assert_eq!(store.prune_blocker("empty").unwrap(), None);
}
