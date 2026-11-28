use super::*;

pub(super) fn run_projects(store: &mut Store, command: ProjectsCommand) -> Result<()> {
    match command {
        ProjectsCommand::List => print_json(&store.list_projects_with_stats()?),
        ProjectsCommand::Consolidate {
            project,
            all,
            apply,
        } => {
            let stats = store.list_projects_with_stats()?;
            // One grouping answers both flags: `--all` shows every group it
            // found, `--project` shows the group that names this project. It
            // used to be two, and they disagreed about the same store — the
            // flag made the project named on the command line the canonical
            // whatever the memories said, so running the remedy from the
            // directory that needed it folded the project that held the work
            // into the name that held an empty session.
            let groups = if all {
                similar_project_groups(&stats)
            } else {
                let named = resolve_project(store, project)?;
                groups_naming(&stats, &named)
            };
            let mut merges = Vec::new();
            for group in &groups {
                // The preview and the merge read the same scope, so what the
                // dry run lists is what the answer will name. Passing the
                // scope back to the merge also keeps the fold closed: asking
                // to merge the names the preview showed moves the same set
                // again and no more.
                let sources = store.project_merge_scope(&group.sources, &group.canonical)?;
                let mut entry = serde_json::json!({
                    "canonical": group.canonical,
                    "sources": sources,
                });
                if !group.skipped.is_empty() {
                    // One entry per source rather than one for the group: a
                    // source refused here is the only thing that does not
                    // move, and the source the remedy exists to fold still
                    // goes with the others.
                    entry["skipped"] = serde_json::Value::Array(
                        group
                            .skipped
                            .iter()
                            .map(|refused| {
                                serde_json::json!({
                                    "project": refused.project,
                                    "reason": refused.reason,
                                })
                            })
                            .collect(),
                    );
                }
                if apply {
                    entry["result"] =
                        serde_json::to_value(store.merge_projects(&sources, &group.canonical)?)?;
                }
                merges.push(entry);
            }
            print_json(&serde_json::json!({
                "dry_run": !apply,
                "groups": merges,
            }))
        }
        ProjectsCommand::Prune { apply } => {
            let candidates = store
                .list_projects_with_stats()?
                .into_iter()
                .filter(|stats| stats.observation_count == 0)
                .collect::<Vec<_>>();
            let mut pruned = Vec::new();
            for candidate in &candidates {
                let mut entry = serde_json::json!({
                    "project": candidate.name,
                    "sessions": candidate.session_count,
                    "prompts": candidate.prompt_count,
                });
                // Listed by live count, which a soft-deleted memory does not
                // appear in, so the list can name a project the prune would
                // refuse and abort on, leaving the ones after it untouched.
                // Said in the preview as well, so what it shows is what the
                // apply does.
                if let Some(reason) = store.prune_blocker(&candidate.name)? {
                    entry["skipped"] = serde_json::Value::String(reason);
                } else if apply {
                    entry["result"] = serde_json::to_value(store.prune_project(&candidate.name)?)?;
                }
                pruned.push(entry);
            }
            print_json(&serde_json::json!({
                "dry_run": !apply,
                "projects": pruned,
            }))
        }
    }
}

/// One fold the store's own evidence proposes: the name that keeps the work and
/// the names that would move into it.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct ProjectGroup {
    pub(super) canonical: String,
    /// The sources this run would move, after the refusals below.
    pub(super) sources: Vec<String>,
    /// Sources that would be moved out of a folder they belong to, with the
    /// reason. Nothing in this list moves, and the preview and the apply both
    /// say so.
    pub(super) skipped: Vec<RefusedSource>,
}

/// A source a fold leaves where it is, because folding it would move a project
/// out of the folder that folder belongs to.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct RefusedSource {
    pub(super) project: String,
    pub(super) reason: String,
}
/// The group a named project belongs to: the same one `--all` reports for it.
///
/// `--project` used to name the canonical itself, so the two forms of the
/// preview could disagree about one store: `--all` chose the name holding the
/// memories and the flag chose the argument. Naming a project that turns out to
/// be a source therefore means its group, whose canonical keeps the work. A
/// project in no group has nothing to fold, which is what an empty answer says.
///
/// A name the refusal leaves in place answers with the group and no sources:
/// the person is told which fold was considered and why their project is not in
/// it, and `--apply` changes nothing. Naming the canonical or a source that
/// does move applies the fold whole, which is what naming a member of it asks
/// for.
fn groups_naming(stats: &[ProjectStats], named: &str) -> Vec<ProjectGroup> {
    let named = crate::memory::normalize::project(named);
    let names = |project: &str| crate::memory::normalize::project(project) == named;
    similar_project_groups(stats)
        .into_iter()
        .filter_map(|group| {
            if names(&group.canonical) || group.sources.iter().any(|source| names(source)) {
                return Some(group);
            }
            group
                .skipped
                .iter()
                .any(|refused| names(&refused.project))
                .then(|| ProjectGroup {
                    sources: Vec::new(),
                    ..group
                })
        })
        .collect()
}

/// Groups projects around the one they each resemble, biggest first.
///
/// Resembling is not transitive, and treating it as though it were merged two
/// projects that have nothing to do with each other. On a real store this
/// proposed folding `nas.archive` — 46 memories — into `almanac`, which has
/// 690 and is a different piece of work entirely. The chain was five links
/// long: `almanac` and `repo` had both been worked on in `H:\REPO`; `repo`
/// resembles `h:\repo`; `h:\repo` resembles `h:\repo\nas.archive`; and that
/// resembles `nas.archive`. A union-find welds all five into one component
/// and then elects the largest as canonical, so the biggest project in the
/// chain swallows everything the chain reaches.
///
/// So a group is now every project that resembles *the canonical itself*,
/// which is what the single-project form has always done, and each project is
/// claimed once — the largest claimant first, so a small project cannot pull a
/// large one into its own group.
///
/// A source the refusal below leaves in place is claimed like any other. It is
/// reported once, in the group that asked, and it is not made the canonical of
/// a later group — which is what keeps a project that was left alone from then
/// swallowing a larger one. Two projects that share a folder and nothing else
/// are related, so leaving the source unclaimed would let the lighter of the
/// two become the name that survives, and that is the direction error this
/// command exists to stop.
///
/// A directory shared by more than two projects is also no longer evidence of
/// anything. `H:\REPO` on that store is the parent folder of every repository
/// on the machine and three projects had a session there; a folder people work
/// in says nothing about what a project is, while a directory two projects
/// share is the ordinary shape of one project renamed.
fn similar_project_groups(stats: &[ProjectStats]) -> Vec<ProjectGroup> {
    let names = stats
        .iter()
        .map(|entry| entry.name.clone())
        .collect::<Vec<_>>();
    let mut related: Vec<std::collections::BTreeSet<usize>> =
        vec![std::collections::BTreeSet::new(); stats.len()];
    let relate =
        |left: usize, right: usize, related: &mut Vec<std::collections::BTreeSet<usize>>| {
            if left != right {
                related[left].insert(right);
                related[right].insert(left);
            }
        };

    for (index, entry) in stats.iter().enumerate() {
        for matched in crate::project::find_similar(&entry.name, &names, 3) {
            if let Some(other) = names.iter().position(|name| *name == matched.name) {
                relate(index, other, &mut related);
            }
        }
    }
    let mut by_directory: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (index, entry) in stats.iter().enumerate() {
        for directory in &entry.directories {
            // A relative directory is every directory at once, so two projects
            // that both recorded `.` are not thereby the same project. The
            // single-project form has always refused that evidence; `--all`
            // did not read it the same way, and folded the two.
            if !crate::project::is_absolute_spelling(directory) {
                continue;
            }
            by_directory
                .entry(crate::project::comparable(directory))
                .or_default()
                .push(index);
        }
    }
    for indexes in by_directory.values_mut() {
        // One project that recorded the same folder under two spellings is one
        // project. Without this a bucket of one project and one other counts
        // three and the rule below drops the evidence, which is the rename it
        // was meant to find.
        indexes.sort_unstable();
        indexes.dedup();
    }
    for indexes in by_directory.values() {
        if indexes.len() != 2 {
            continue;
        }
        relate(indexes[0], indexes[1], &mut related);
    }

    // Biggest first, so the project a group is named after is the one that
    // holds the work rather than whichever the ordering happened to reach.
    let mut order = (0..stats.len()).collect::<Vec<_>>();
    order.sort_by(|left, right| {
        stats[*right]
            .observation_count
            .cmp(&stats[*left].observation_count)
            .then_with(|| stats[*left].name.cmp(&stats[*right].name))
    });

    let mut claimed = vec![false; stats.len()];
    let mut groups = Vec::new();
    for index in order {
        if claimed[index] {
            continue;
        }
        let taken = related[index]
            .iter()
            .copied()
            .filter(|other| !claimed[*other])
            .collect::<Vec<_>>();
        if taken.is_empty() {
            continue;
        }
        let mut sources = Vec::with_capacity(taken.len());
        let mut skipped = Vec::new();
        for other in taken {
            // Judged one source at a time: a source that is a project of its
            // own is not folded, and the empty name the remedy exists for still
            // goes with the others.
            claimed[other] = true;
            match two_projects(&stats[index], &stats[other]) {
                Some(reason) => skipped.push(RefusedSource {
                    project: stats[other].name.clone(),
                    reason,
                }),
                None => sources.push(stats[other].name.clone()),
            }
        }
        claimed[index] = true;
        sources.sort();
        skipped.sort_by(|left, right| left.project.cmp(&right.project));
        groups.push(ProjectGroup {
            canonical: stats[index].name.clone(),
            sources,
            skipped,
        });
    }
    groups.sort_by(|left, right| left.canonical.cmp(&right.canonical));
    groups
}

/// Why folding `source` into `canonical` would move a project out of its home.
///
/// A name is a spelling of another when the folder or the memories are shared:
/// a repository that gained a remote, or a project renamed without moving,
/// keeps one of the two. A source that holds memories *and* records a directory
/// the canonical does not is neither — it is a second project that happens to
/// resemble the first. A merge has no undo, and the issue that produced this
/// rule is a session-start warning whose advice emptied a project in four runs
/// out of four. `mem_merge_projects` remains the deliberate door: it names both
/// sides and the direction, which is what a person who means it can answer.
fn two_projects(canonical: &ProjectStats, source: &ProjectStats) -> Option<String> {
    if source.observation_count == 0 {
        return None;
    }
    let own = source.directories.iter().find(|directory| {
        crate::project::is_absolute_spelling(directory)
            && !canonical.directories.iter().any(|shared| {
                crate::project::comparable(shared) == crate::project::comparable(directory)
            })
    })?;
    Some(format!(
        "{:?} holds {} memories and records {:?}, which {:?} does not: these are two projects, \
         not one name twice, and folding them would move work out of the folder it belongs to. \
         It is left where it is; use `mem_merge_projects` with both names if they really are one",
        source.name, source.observation_count, own, canonical.name
    ))
}

/// Which project a listing answers about: the one named, the one this
/// directory belongs to, or every one of them.
///
/// `search`, `recent` and `context` passed `--project` straight through, so
/// with nothing named they answered from the whole store while `mem_search`
/// standing in the same directory answered from one project. Measured over the
/// 114 real questions somebody asked from inside this repo, 107 of which found
/// something: 82% came back with at least one memory from another project and
/// 72% led with one. Of those 77, exactly two would have found nothing had the
/// search been narrowed — the other 75 had a memory from this project waiting
/// underneath. And 77 of the 88 arrived through a relaxed stage rather than by
/// matching: an exact hit in another project can be worth having, the nearest
/// thing in the whole store is not.
///
/// `conflicts` already narrowed this way through `resolve_project`, so the
/// three that did not were drift rather than a decision. This is the softer
/// form the three of them need: `resolve_project` refuses when it cannot tell
/// which project this is, which is right before a write and wrong before a
/// search — somebody asking a question from a directory Leteo knows nothing
/// about should get the whole store, not an error.
pub(super) struct ReadScope {
    pub(super) project: Option<String>,
    /// True when the directory decided this rather than the caller. It is the
    /// difference between an empty answer somebody asked for and one they did
    /// not know they were asking for.
    pub(super) inferred: bool,
}

pub(super) fn read_scope(store: &Store, explicit: Option<String>, all_projects: bool) -> ReadScope {
    if all_projects {
        return ReadScope {
            project: None,
            inferred: false,
        };
    }
    if let Some(project) = explicit {
        return ReadScope {
            project: Some(project),
            inferred: false,
        };
    }
    let detection = crate::project::detect_current_project();
    if detection.error_hint.is_some() || detection.project.is_empty() {
        return ReadScope {
            project: None,
            inferred: false,
        };
    }
    // The block this scopes is the one a save from here would join, so a name
    // another repository already recorded is settled first, and the person is
    // told why the name is not the folder's.
    let project = match store.settle_detected_project(&detection) {
        Ok(settled) => {
            if let Some(warning) = &settled.warning {
                eprintln!("leteo: {warning}");
            }
            settled.project
        }
        // The read this scopes is about to fail on the same connection and say
        // so; the detected name is the answer that is still true.
        Err(_) => crate::memory::normalize::project(&detection.project),
    };
    ReadScope {
        inferred: true,
        project: Some(project),
    }
}

/// Resolves the project a read-only command applies to, falling back to
/// detection from the current directory.
pub(super) fn resolve_project(store: &Store, explicit: Option<String>) -> Result<String> {
    if let Some(project) = explicit {
        let project = crate::memory::normalize::project(&project);
        if project.is_empty() {
            anyhow::bail!("{}", crate::project::EMPTY_NAME);
        }
        return Ok(project);
    }
    let detection = crate::project::detect_current_project();
    let project = crate::memory::normalize::project(&detection.project);
    if project.is_empty() {
        anyhow::bail!(
            "{}; pass --project PROJECT",
            detection
                .error_hint
                .as_deref()
                .unwrap_or("cannot determine the current project")
        );
    }
    // The same settlement the write doors apply, so a command that reads one
    // project does not read another repository's memories of the same name.
    let settled = store.settle_detected_project(&detection)?;
    if let Some(warning) = &settled.warning {
        eprintln!("leteo: {warning}");
    }
    Ok(settled.project)
}

pub(super) struct WriteSession {
    pub(super) id: String,
    pub(super) project: String,
    /// Whether the caller chose the session, which decides how far a save may
    /// reach for the question behind it. See `Store::prompt_behind_a_save`.
    pub(super) named: bool,
    /// The directory the memory is recorded as written from: this process's for
    /// a save that joins a session itself, the session's own for one the caller
    /// named — the rule `mem_save` follows, so the two doors agree.
    pub(super) directory: Option<String>,
}

/// Whether a write files under the project detection answers with.
///
/// Only then does this directory's recorded history matter: a directory whose
/// sessions were recorded under another name would file this write on the other
/// side of a split. A consolidation files where its sources are, so where it is
/// run from decides nothing and the walk through the sources owns the project.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Filing {
    Detection,
    Elsewhere,
}

/// Resolves the session a CLI write belongs to: the one named, else the
/// project's live session as `mem_save` joins it, else the stable manual
/// session.
pub(super) fn resolve_write_session(
    store: &mut Store,
    session_id: Option<String>,
    explicit_project: Option<String>,
    filing: Filing,
) -> Result<WriteSession> {
    if let Some(id) = session_id.filter(|id| !id.trim().is_empty()) {
        let session = store.get_session(&id)?;
        // The same refusal the MCP write path makes: a write naming an ended
        // session is a misattribution, and the two doors must not disagree
        // about which sessions accept content.
        if session.ended_at.is_some() {
            anyhow::bail!("session {id:?} has ended; omit --session or start a new one");
        }
        let session_project = crate::memory::normalize::project(&session.project);
        if let Some(project) = explicit_project {
            let project = crate::memory::normalize::project(&project);
            if project.is_empty() {
                anyhow::bail!("{}", crate::project::EMPTY_NAME);
            }
            if project != session_project {
                anyhow::bail!(
                    "session {id:?} belongs to project {session_project:?}, not {project:?}"
                );
            }
        }
        return Ok(WriteSession {
            id,
            project: session_project,
            named: true,
            directory: Some(session.directory).filter(|path| !path.is_empty()),
        });
    }

    join_or_open_manual_session(
        store,
        explicit_project,
        filing,
        crate::project::detect_current_project(),
    )
}

/// The tail of [`resolve_write_session`] for a write that names no session,
/// with the detection handed in so a test can say where the command was run.
fn join_or_open_manual_session(
    store: &mut Store,
    explicit_project: Option<String>,
    filing: Filing,
    detection: crate::project::ProjectDetection,
) -> Result<WriteSession> {
    let names_another_project = explicit_project.is_some();
    let mut settled = crate::project::SettledProject::plain(&detection);
    let project = if let Some(project) = explicit_project {
        let project = crate::memory::normalize::project(&project);
        if project.is_empty() {
            anyhow::bail!("{}", crate::project::EMPTY_NAME);
        }
        project
    } else {
        let project = crate::memory::normalize::project(&detection.project);
        if project.is_empty() {
            anyhow::bail!(
                "{}; pass --project PROJECT or --session SESSION",
                detection
                    .error_hint
                    .as_deref()
                    .unwrap_or("cannot determine the current project")
            );
        }
        // What the MCP door refuses, so the same directory cannot take a
        // memory through one door and reject it through the other: the
        // sessions already recorded here are evidence this directory used to
        // resolve to another name, and filing under the new one splits the
        // project in two.
        if filing == Filing::Detection {
            let recorded = store.drifted_projects(&detection, crate::mcp::is_manual_session_id)?;
            if !recorded.is_empty() {
                anyhow::bail!(
                    "this directory's sessions were recorded under {}, but it now resolves \
                     to {project:?}; pass --project PROJECT (one of the recorded names) to \
                     save under that name, or run `leteo projects consolidate` if the two \
                     names are one project",
                    recorded.join(", ")
                );
            }
            settled = store.settle_detected_project(&detection)?;
            if let Some(warning) = &settled.warning {
                eprintln!("leteo: {warning}");
            }
        }
        settled.project.clone()
    };
    // The repository this command is standing in is only this project's when
    // the project came from here; a `--project` naming somewhere else would
    // claim this checkout for it.
    let remote = (!names_another_project && filing == Filing::Detection)
        .then(|| crate::project::detected_remote(&detection).map(|remote| remote.key))
        .flatten();
    let directory = if detection.path.is_empty() {
        // An explicit project can still be written when the working directory
        // cannot be read; the session then carries no directory, not a guess.
        std::env::current_dir()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default()
    } else {
        detection.path.clone()
    };
    // The same lookup, window and definition of a synthetic session that
    // `write_session` in the MCP server uses, so a memory typed at a terminal
    // lands in the conversation a hook opened rather than in a session nobody
    // ever reads next to it.
    let live = store.latest_active_session(
        &project,
        crate::store::PROMPT_ATTRIBUTION_MINUTES,
        crate::mcp::is_manual_session_id,
    )?;
    let id = match live {
        Some(session) => session.id,
        None => {
            let id = crate::mcp::manual_session_id(&project);
            let recorded =
                crate::project::session_directory(&project, &settled.detection, &directory);
            match remote.as_deref() {
                Some(remote) => {
                    store.create_session_with_remote(&id, &project, recorded, remote)?
                }
                None => store.create_session(&id, &project, recorded)?,
            };
            id
        }
    };
    Ok(WriteSession {
        id,
        project,
        named: false,
        directory: Some(directory).filter(|path| !path.is_empty()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(name: &str, observations: i64, directories: &[&str]) -> ProjectStats {
        ProjectStats {
            name: name.to_owned(),
            observation_count: observations,
            session_count: 1,
            prompt_count: 0,
            directories: directories
                .iter()
                .map(|entry| (*entry).to_owned())
                .collect(),
            last_activity: None,
        }
    }

    /// A chain of resemblances does not make two projects one.
    ///
    /// The fixture is a real store, in the shape it was found in. `almanac`,
    /// `repo` and `h:\repo` had all been worked on in `H:\REPO`, which is the
    /// folder every repository on that machine sits in; `h:\repo` resembles
    /// `h:\repo\nas.archive`, and that resembles `nas.archive`. Joining
    /// every link into one component and electing the largest as canonical
    /// proposed folding 46 memories of `nas.archive` into `almanac`, which
    /// is 690 memories of something else. `--apply` would have done it.
    #[test]
    fn a_chain_of_resemblances_does_not_merge_two_unrelated_projects() {
        let stats = vec![
            project("almanac", 690, &[r"H:\REPO"]),
            project("nas.archive", 46, &[r"H:\REPO\nas.archive"]),
            project("repo", 5, &[r"H:\REPO"]),
            project(r"h:\repo", 0, &[r"H:\REPO"]),
            project(r"h:\repo\nas.archive", 0, &[r"H:\REPO\nas.archive"]),
        ];
        let groups = similar_project_groups(&stats);

        let canonical_of = |name: &str| {
            groups
                .iter()
                .find(|group| group.sources.iter().any(|source| source == name))
                .map(|group| group.canonical.clone())
        };
        assert_eq!(
            canonical_of(r"h:\repo\nas.archive").as_deref(),
            Some("nas.archive"),
            "a fragment goes back to the project it is a fragment of"
        );
        assert_eq!(
            canonical_of(r"h:\repo").as_deref(),
            Some("repo"),
            "and so does the other one"
        );
        assert!(
            !groups.iter().any(|group| group.canonical == "almanac"),
            "nothing is folded into a project that only shares a parent folder: {groups:?}"
        );
        assert!(
            canonical_of("nas.archive").is_none(),
            "a project with 46 memories is not somebody else's source: {groups:?}"
        );
    }

    /// The memories decide which name survives, whatever was asked for.
    ///
    /// The shape the issue reports, from the store after `mem_merge_projects`
    /// moved `alpha` into `beta`: `beta` holds all the work and records the
    /// folder it was moved from, and `alpha` holds one session and no memories.
    /// The remedy run from that folder used to name `alpha` the canonical,
    /// because that is what the directory resolves to, and `--all` named
    /// `beta`. Both forms now read the same group.
    #[test]
    fn the_name_holding_the_work_is_the_one_that_survives() {
        let stats = vec![
            project("beta", 11, &["/work/alpha", "/work/beta"]),
            project("alpha", 0, &["/work/alpha"]),
        ];
        let expected = vec![ProjectGroup {
            canonical: "beta".to_owned(),
            sources: vec!["alpha".to_owned()],
            skipped: Vec::new(),
        }];
        assert_eq!(similar_project_groups(&stats), expected, "under `--all`");
        assert_eq!(
            groups_naming(&stats, "alpha"),
            expected,
            "and when the directory's own name is the one named"
        );
        assert_eq!(
            groups_naming(&stats, "beta"),
            expected,
            "naming the other side answers the same group"
        );
        assert_eq!(
            groups_naming(&stats, " ALPHA "),
            expected,
            "a name reaches its group however the store spelled it"
        );
        assert!(
            groups_naming(&stats, "something-else").is_empty(),
            "a project in no group has nothing to fold"
        );
    }

    /// Two projects that each hold memories in a folder of their own are not
    /// one project spelled twice.
    ///
    /// `leteo` and `leteo-old` resemble each other, and folding either would
    /// move real work out of the folder it belongs to with no undo. The group
    /// is still reported, so the preview says why nothing happens instead of
    /// looking like a store with nothing to fold.
    #[test]
    fn a_source_with_memories_in_a_folder_of_its_own_is_refused() {
        let stats = vec![
            project("leteo", 9, &["/work/leteo"]),
            project("leteo-old", 3, &["/work/leteo-old"]),
        ];
        let groups = similar_project_groups(&stats);
        assert_eq!(groups.len(), 1, "{groups:?}");
        assert_eq!(groups[0].canonical, "leteo");
        assert!(
            groups[0].sources.is_empty(),
            "nothing moves: {:?}",
            groups[0].sources
        );
        let refusal = groups[0]
            .skipped
            .first()
            .expect("a source with a home of its own is refused");
        assert_eq!(refusal.project, "leteo-old");
        assert!(refusal.reason.contains("/work/leteo-old"), "{refusal:?}");
        assert!(refusal.reason.contains("mem_merge_projects"), "{refusal:?}");

        // Naming the refused side reaches the same group, so the person who
        // asked to fold it is told why nothing happened.
        assert_eq!(groups_naming(&stats, "leteo-old"), groups);
    }

    /// A fold never makes the lighter project the name that survives.
    ///
    /// The chain the panel built: `almanac` resembles `almanac-old` and refuses
    /// it, because `almanac-old` holds memories in a folder `almanac` does not
    /// record; and `almanac-old` shares that folder with a larger project. Leave
    /// the refused source free to be a canonical and the larger project is
    /// taken by the smaller one, which is the direction error this command
    /// exists to stop.
    #[test]
    fn a_refused_source_never_becomes_the_canonical_of_a_larger_project() {
        let stats = vec![
            project("almanac", 11, &["/work/almanac"]),
            project("delta", 7, &["/work/c"]),
            project("almanac-old", 3, &["/work/c"]),
        ];
        let groups = similar_project_groups(&stats);
        assert_eq!(groups.len(), 1, "{groups:?}");
        assert_eq!(groups[0].canonical, "almanac");
        assert!(groups[0].sources.is_empty());
        assert_eq!(groups[0].skipped[0].project, "almanac-old");

        // The invariant behind that: a source never holds more than the name it
        // moves into.
        for group in &groups {
            let canonical = stats
                .iter()
                .find(|stats| stats.name == group.canonical)
                .expect("a canonical from the stats");
            for source in &group.sources {
                let source = stats
                    .iter()
                    .find(|stats| stats.name == *source)
                    .expect("a source from the stats");
                assert!(
                    canonical.observation_count >= source.observation_count,
                    "{:?} holds more than {:?} and would move into it: {groups:?}",
                    source.name,
                    canonical.name
                );
            }
        }
    }

    /// One refused source does not stop the fold the remedy exists for.
    ///
    /// A group can hold the empty name a merge left behind and a second project
    /// that only resembles the canonical. Judged together, the second one would
    /// call the fold off and leave the drift in place; judged one at a time,
    /// only it stays.
    #[test]
    fn a_refused_source_does_not_call_off_the_rest_of_the_group() {
        let stats = vec![
            project("beta", 11, &["/work/alpha", "/work/beta"]),
            project("beta-old", 4, &["/work/beta-old"]),
            project("alpha", 0, &["/work/alpha"]),
        ];
        let groups = similar_project_groups(&stats);
        assert_eq!(groups.len(), 1, "{groups:?}");
        assert_eq!(groups[0].canonical, "beta");
        assert_eq!(groups[0].sources, vec!["alpha".to_owned()]);
        assert_eq!(groups[0].skipped.len(), 1);
        assert_eq!(groups[0].skipped[0].project, "beta-old");

        // Naming the source that moves answers the whole fold.
        assert_eq!(groups_naming(&stats, "alpha"), groups);

        // Naming the one left in place answers the same group with nothing to
        // move, so a run scoped to it changes nothing.
        let scoped = groups_naming(&stats, "beta-old");
        assert_eq!(scoped.len(), 1, "{scoped:?}");
        assert_eq!(scoped[0].canonical, "beta");
        assert!(
            scoped[0].sources.is_empty(),
            "a run scoped to a project the fold leaves alone applies nothing: {scoped:?}"
        );
        assert_eq!(scoped[0].skipped[0].project, "beta-old");
    }

    /// A source with memories but no recorded folder is a name variant.
    ///
    /// The refusal needs a home to protect. A project whose sessions recorded
    /// no absolute directory — or only `.` — has no folder to be moved out of,
    /// and folding it by its name is the ordinary rename.
    #[test]
    fn a_source_with_memories_and_no_folder_of_its_own_is_folded() {
        let stats = vec![
            project("leteo", 9, &["/work/leteo"]),
            project("leteo-old", 3, &["."]),
        ];
        let groups = similar_project_groups(&stats);
        assert_eq!(
            groups,
            vec![ProjectGroup {
                canonical: "leteo".to_owned(),
                sources: vec!["leteo-old".to_owned()],
                skipped: Vec::new(),
            }]
        );
    }

    /// Two spellings of one folder are one folder.
    ///
    /// The session matcher has always folded a trailing separator and, on
    /// Windows, case. The consolidation reads the same fold, so a project whose
    /// folder arrived with a trailing slash is still the project that already
    /// records that folder rather than a second one to refuse.
    #[test]
    fn two_spellings_of_one_folder_are_one_folder() {
        let stats = vec![
            project("alpha-service", 10, &["/work/shared"]),
            project("omega-worker", 5, &["/work/shared/"]),
        ];
        assert_eq!(
            similar_project_groups(&stats),
            vec![ProjectGroup {
                canonical: "alpha-service".to_owned(),
                sources: vec!["omega-worker".to_owned()],
                skipped: Vec::new(),
            }]
        );

        // And the same fold decides the refusal: a source whose only folder is
        // the canonical's, spelled differently, has no home of its own.
        let stats = vec![
            project("leteo", 9, &["/work/leteo"]),
            project("leteo-old", 3, &["/work/leteo/"]),
        ];
        assert_eq!(
            similar_project_groups(&stats)[0].sources,
            vec!["leteo-old".to_owned()],
            "one folder under two spellings is not a second home"
        );

        // One project that recorded the folder under both spellings is still
        // one project, so the shared-directory rule counts two, not three.
        let stats = vec![
            project("alpha-service", 10, &["/work/shared", "/work/shared/"]),
            project("omega-worker", 5, &["/work/shared"]),
        ];
        assert_eq!(
            similar_project_groups(&stats),
            vec![ProjectGroup {
                canonical: "alpha-service".to_owned(),
                sources: vec!["omega-worker".to_owned()],
                skipped: Vec::new(),
            }]
        );
    }

    /// `.` is not a place. Two projects that both recorded it share no
    /// directory, and grouping them folded one into the other on the strength
    /// of a spelling that means "wherever the reader is".
    #[test]
    fn a_relative_directory_does_not_link_two_projects() {
        let stats = vec![
            project("alpha-service", 10, &["."]),
            project("omega-worker", 5, &["."]),
        ];
        assert!(
            similar_project_groups(&stats).is_empty(),
            "the grouped preview reads the same rule as the single one"
        );
        let stats = vec![
            project("alpha-service", 10, &["/work/shared"]),
            project("omega-worker", 5, &["/work/shared"]),
        ];
        assert_eq!(
            similar_project_groups(&stats),
            vec![ProjectGroup {
                canonical: "alpha-service".to_owned(),
                sources: vec!["omega-worker".to_owned()],
                skipped: Vec::new(),
            }],
            "an absolute shared directory still does"
        );
    }

    /// A folder several projects were worked on in says nothing about them.
    #[test]
    fn a_directory_three_projects_share_is_not_evidence() {
        let stats = vec![
            project("uno", 10, &[r"H:\REPO"]),
            project("dos", 5, &[r"H:\REPO"]),
            project("tres", 1, &[r"H:\REPO"]),
        ];
        assert!(
            similar_project_groups(&stats).is_empty(),
            "three projects in one folder are three projects"
        );

        // Equal memories: the name decides, so the same store groups the same
        // way twice.
        let tied = vec![
            project("nombre-b", 2, &[r"H:\REPO\cosa"]),
            project("nombre-a", 2, &[r"H:\REPO\cosa"]),
        ];
        assert_eq!(similar_project_groups(&tied)[0].canonical, "nombre-a");

        // Two is the ordinary shape of one project renamed, and still counts.
        let renamed = vec![
            project("nombre-nuevo", 10, &[r"H:\REPO\cosa"]),
            project("nombre-viejo", 2, &[r"H:\REPO\cosa"]),
        ];
        assert_eq!(
            similar_project_groups(&renamed),
            vec![ProjectGroup {
                canonical: "nombre-nuevo".to_owned(),
                sources: vec!["nombre-viejo".to_owned()],
                skipped: Vec::new(),
            }]
        );
    }

    /// The command line's twin of the MCP guard: `leteo save --project other`
    /// from inside a repository opens the other project's manual session
    /// without recording the repository on it.
    #[test]
    fn a_save_for_another_project_does_not_record_this_directory() {
        let temp = tempfile::tempdir().unwrap();
        let mut store =
            Store::open(crate::store::StoreConfig::new(temp.path().join("cli.db"))).unwrap();
        let standing_in = |project: &str| crate::project::ProjectDetection {
            project: project.to_owned(),
            source: crate::project::SOURCE_GIT_REMOTE.to_owned(),
            path: "C:/here".to_owned(),
            available_projects: Vec::new(),
            warning: None,
            error_hint: None,
        };

        let crossed = join_or_open_manual_session(
            &mut store,
            Some("other".to_owned()),
            Filing::Detection,
            standing_in("here"),
        )
        .unwrap();
        assert_eq!(store.get_session(&crossed.id).unwrap().directory, "");

        let own =
            join_or_open_manual_session(&mut store, None, Filing::Detection, standing_in("here"))
                .unwrap();
        assert_eq!(own.project, "here");
        assert_eq!(store.get_session(&own.id).unwrap().directory, "C:/here");
    }

    /// The command line asks what the MCP door asks.
    ///
    /// `leteo save` filed the memory under the name detection reads now, while
    /// `mem_save` refused the same write with the recorded projects named.
    /// Adding a remote, renaming it, or pointing the repository at a fork
    /// changes what the directory resolves to, and the memories already there
    /// stay under the old name — so a silent save splits the project in two.
    #[test]
    fn a_save_in_a_drifted_directory_is_refused_with_the_recorded_projects_named() {
        let temp = tempfile::tempdir().unwrap();
        let mut store =
            Store::open(crate::store::StoreConfig::new(temp.path().join("cli.db"))).unwrap();
        store
            .create_session("old", "old-remote", "C:/here")
            .unwrap();
        let standing_in = |project: &str| crate::project::ProjectDetection {
            project: project.to_owned(),
            source: crate::project::SOURCE_GIT_REMOTE.to_owned(),
            path: "C:/here".to_owned(),
            available_projects: Vec::new(),
            warning: None,
            error_hint: None,
        };

        let refusal = match join_or_open_manual_session(
            &mut store,
            None,
            Filing::Detection,
            standing_in("new-remote"),
        ) {
            Ok(session) => panic!(
                "saved under {:?} instead of asking which project this is",
                session.project
            ),
            Err(error) => error,
        };
        let refusal = refusal.to_string();
        assert!(refusal.contains("old-remote"), "{refusal}");
        assert!(refusal.contains("new-remote"), "{refusal}");
        assert!(
            refusal.contains("--project"),
            "there is no recovery token here, so the refusal names the flag: {refusal}"
        );

        for chosen in ["old-remote", "new-remote"] {
            let session = join_or_open_manual_session(
                &mut store,
                Some(chosen.to_owned()),
                Filing::Detection,
                standing_in("new-remote"),
            )
            .unwrap_or_else(|error| panic!("--project {chosen} was refused: {error}"));
            assert_eq!(session.project, chosen);
        }
    }

    /// A consolidation files where its sources are, so the directory's recorded
    /// names decide nothing: refusing here would block a merge that mis-files
    /// nothing, and `leteo consolidate` has no `--project` to get past it.
    #[test]
    fn a_consolidation_in_a_drifted_directory_is_not_refused_for_the_detected_name() {
        let temp = tempfile::tempdir().unwrap();
        let mut store =
            Store::open(crate::store::StoreConfig::new(temp.path().join("cli.db"))).unwrap();
        store
            .create_session("old", "old-remote", "C:/here")
            .unwrap();
        let detection = crate::project::ProjectDetection {
            project: "new-remote".to_owned(),
            source: crate::project::SOURCE_GIT_REMOTE.to_owned(),
            path: "C:/here".to_owned(),
            available_projects: Vec::new(),
            warning: None,
            error_hint: None,
        };

        let session =
            join_or_open_manual_session(&mut store, None, Filing::Elsewhere, detection).unwrap();
        assert_eq!(session.project, "new-remote");
    }
}
