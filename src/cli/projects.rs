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
            let groups = if all {
                similar_project_groups(&stats)
            } else {
                let canonical = resolve_project(project)?;
                let sources = similar_projects(&canonical, &stats);
                if sources.is_empty() {
                    Vec::new()
                } else {
                    vec![(canonical, sources)]
                }
            };
            let mut merges = Vec::new();
            for (canonical, sources) in &groups {
                let mut entry = serde_json::json!({
                    "canonical": canonical,
                    "sources": sources,
                });
                if apply {
                    entry["result"] =
                        serde_json::to_value(store.merge_projects(sources, canonical)?)?;
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
                if apply {
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

/// Returns the projects that should be consolidated into `canonical`: those
/// with similar names and those recorded under one of its directories.
fn similar_projects(canonical: &str, stats: &[ProjectStats]) -> Vec<String> {
    let names = stats
        .iter()
        .map(|entry| entry.name.clone())
        .collect::<Vec<_>>();
    let mut sources = crate::project::find_similar(canonical, &names, 3)
        .into_iter()
        .map(|matched| matched.name)
        .collect::<Vec<_>>();
    let directories = stats
        .iter()
        .find(|entry| entry.name == canonical)
        .map(|entry| entry.directories.clone())
        .unwrap_or_default();
    for entry in stats {
        if entry.name == canonical || sources.contains(&entry.name) {
            continue;
        }
        if entry
            .directories
            .iter()
            .any(|directory| directories.contains(directory))
        {
            sources.push(entry.name.clone());
        }
    }
    sources
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
/// A directory shared by more than two projects is also no longer evidence of
/// anything. `H:\REPO` on that store is the parent folder of every repository
/// on the machine and three projects had a session there; a folder people work
/// in says nothing about what a project is, while a directory two projects
/// share is the ordinary shape of one project renamed.
fn similar_project_groups(stats: &[ProjectStats]) -> Vec<(String, Vec<String>)> {
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
    let mut by_directory: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (index, entry) in stats.iter().enumerate() {
        for directory in &entry.directories {
            by_directory.entry(directory).or_default().push(index);
        }
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
        let mut sources = Vec::with_capacity(taken.len());
        for other in taken {
            claimed[other] = true;
            sources.push(stats[other].name.clone());
        }
        if sources.is_empty() {
            continue;
        }
        claimed[index] = true;
        sources.sort();
        groups.push((stats[index].name.clone(), sources));
    }
    groups.sort_by(|left, right| left.0.cmp(&right.0));
    groups
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

pub(super) fn read_scope(explicit: Option<String>, all_projects: bool) -> ReadScope {
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
    let project = (detection.error_hint.is_none() && !detection.project.is_empty())
        .then_some(detection.project);
    ReadScope {
        inferred: project.is_some(),
        project,
    }
}

/// Resolves the project a read-only command applies to, falling back to
/// detection from the current directory.
pub(super) fn resolve_project(explicit: Option<String>) -> Result<String> {
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
    Ok(project)
}

pub(super) struct WriteSession {
    pub(super) id: String,
    pub(super) project: String,
    /// Whether the caller chose the session, which decides how far a save may
    /// reach for the question behind it. See `Store::prompt_behind_a_save`.
    pub(super) named: bool,
}

/// Resolves the session a CLI write belongs to, creating the stable manual
/// session used by `mem_save` when `--session` is omitted.
pub(super) fn resolve_write_session(
    store: &mut Store,
    session_id: Option<String>,
    explicit_project: Option<String>,
) -> Result<WriteSession> {
    if let Some(id) = session_id.filter(|id| !id.trim().is_empty()) {
        let session = store.get_session(&id)?;
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
        });
    }

    let detection = crate::project::detect_current_project();
    let project = match explicit_project {
        Some(project) => {
            let project = crate::memory::normalize::project(&project);
            if project.is_empty() {
                anyhow::bail!("{}", crate::project::EMPTY_NAME);
            }
            project
        }
        None => {
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
            project
        }
    };
    let id = crate::mcp::manual_session_id(&project);
    let directory = if detection.path.is_empty() {
        std::env::current_dir()?.to_string_lossy().into_owned()
    } else {
        detection.path
    };
    store.create_session(&id, &project, &directory)?;
    Ok(WriteSession {
        id,
        project,
        named: false,
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
                .find(|(_, sources)| sources.iter().any(|source| source == name))
                .map(|(canonical, _)| canonical.clone())
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
            !groups.iter().any(|(canonical, _)| canonical == "almanac"),
            "nothing is folded into a project that only shares a parent folder: {groups:?}"
        );
        assert!(
            canonical_of("nas.archive").is_none(),
            "a project with 46 memories is not somebody else's source: {groups:?}"
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

        // Two is the ordinary shape of one project renamed, and still counts.
        let renamed = vec![
            project("nombre-nuevo", 10, &[r"H:\REPO\cosa"]),
            project("nombre-viejo", 2, &[r"H:\REPO\cosa"]),
        ];
        assert_eq!(
            similar_project_groups(&renamed),
            vec![("nombre-nuevo".to_owned(), vec!["nombre-viejo".to_owned()])]
        );
    }
}
