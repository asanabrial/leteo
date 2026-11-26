use std::path::{Path, PathBuf};

use tempfile::TempDir;

use super::*;
use crate::setup::SetupOptions;

/// An options set whose every path is inside `directory`.
///
/// The whole point: an uninstall test that resolved real paths would take
/// Leteo off the machine running it.
fn probe_in(directory: &Path) -> SetupOptions {
    SetupOptions {
        home_dir: Some(directory.to_path_buf()),
        config_home: Some(directory.join("config")),
        app_data: Some(directory.join("appdata")),
        dsh_home: Some(directory.join(".dsh")),
        claude_config: Some(directory.join(".claude")),
        pi_agent_dir: Some(directory.join(".pi").join("agent")),
        ..SetupOptions::default()
    }
}

/// A stand-in for the running executable, inside `temp`, so that what is looked
/// for beside it and under `../share` is the test's own directory and not the
/// test binary's.
fn fake_exe(temp: &Path) -> Option<PathBuf> {
    let bin = temp.join("prefix").join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let exe = bin.join("leteo");
    std::fs::write(&exe, b"binary").unwrap();
    Some(exe)
}

#[test]
fn a_dry_run_removes_nothing_at_all() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(data.join("settings.json"), "{}").unwrap();

    let options = SetupOptions {
        dry_run: true,
        ..probe_in(temp.path())
    };
    let removed = uninstall_everything_for(&options, &data, fake_exe(temp.path()));

    assert!(removed.dry_run);
    assert!(!removed.data_dir_removed);
    assert!(data.exists(), "the data directory has to survive a dry run");
    assert!(
        data.join("settings.json").exists(),
        "and so does everything in it"
    );
    assert!(!removed.binary_removed);
}

#[test]
fn every_agent_is_visited_rather_than_only_the_configured_ones() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join("data");
    std::fs::create_dir_all(&data).unwrap();

    let removed = uninstall_everything_for(&probe_in(temp.path()), &data, fake_exe(temp.path()));

    assert_eq!(
        removed.agents.len(),
        crate::setup::agents::REGISTRY.len(),
        "every adapter has to be visited: {:?}",
        removed.agents
    );
    for agent in &removed.agents {
        assert!(!agent.was_configured, "nothing was installed here");
    }
}

#[test]
fn the_data_directory_goes_and_the_count_is_taken_before_it_does() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join("data");
    let mut store =
        crate::store::Store::open(crate::store::StoreConfig::new(data.join("leteo.db"))).unwrap();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    for title in ["one", "two", "three"] {
        store
            .add_observation(crate::memory::model::AddObservation {
                session_id: "s1".to_owned(),
                kind: "decision".to_owned(),
                title: title.to_owned(),
                content: "body".to_owned(),
                tool_name: None,
                project: Some("leteo".to_owned()),
                scope: "project".to_owned(),
                topic_key: None,
                prompt_sync_id: None,
            })
            .unwrap();
    }
    drop(store);

    let removed = uninstall_everything_for(&probe_in(temp.path()), &data, fake_exe(temp.path()));

    assert_eq!(removed.memories, Some(3));
    assert!(removed.data_dir_removed);
    assert!(!data.exists(), "the store has to be gone");
}

#[test]
fn a_store_that_cannot_be_counted_does_not_stop_the_uninstall() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(data.join("leteo.db"), b"this is not a database").unwrap();

    let removed = uninstall_everything_for(&probe_in(temp.path()), &data, fake_exe(temp.path()));

    assert_eq!(removed.memories, None, "unreadable is not zero");
    assert!(removed.data_dir_removed, "and it still goes");
}

#[test]
fn an_absent_data_directory_is_not_a_failure() {
    let temp = TempDir::new().unwrap();
    let removed = uninstall_everything_for(
        &probe_in(temp.path()),
        &temp.path().join("gone"),
        fake_exe(temp.path()),
    );
    assert_eq!(removed.memories, None);
    assert!(removed.remaining.iter().all(|line| !line.contains("gone")));
}

#[cfg(windows)]
#[test]
fn windows_reports_the_binary_instead_of_pretending_it_removed_it() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join("data");
    std::fs::create_dir_all(&data).unwrap();

    let removed = uninstall_everything_for(&probe_in(temp.path()), &data, fake_exe(temp.path()));

    assert!(
        !removed.binary_removed,
        "claiming to have deleted a running .exe would be a lie"
    );
    assert!(
        removed
            .remaining
            .iter()
            .any(|line| line.contains("uninstall.ps1")),
        "and it has to say what finishes the job: {:?}",
        removed.remaining
    );
}

/// The binary a package manager installed is the manager's to remove.
///
/// `uninstall --yes` took every executable it found, a Homebrew or npm install
/// included. The manager's record then disagreed with the disk, and the next
/// upgrade or reinstall acted on a file that was gone.
#[test]
fn a_package_managed_binary_is_left_and_its_removal_named() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join("data");
    std::fs::create_dir_all(&data).unwrap();

    let cases = [
        (
            temp.path().join("Cellar/leteo/1.2.3/bin/leteo"),
            "Homebrew",
            "brew uninstall leteo",
        ),
        (
            temp.path().join("mise/installs/leteo/1.2.3/bin/leteo"),
            "mise",
            "mise uninstall leteo",
        ),
        (
            temp.path().join(".asdf/installs/leteo/1.2.3/bin/leteo"),
            "asdf",
            "asdf uninstall leteo",
        ),
        (
            temp.path()
                .join("_npx/1a19a25e/node_modules/@asanabrial/leteo/vendor/leteo"),
            "npm",
            "npm cache clean --force",
        ),
        (
            temp.path()
                .join("lib/node_modules/@asanabrial/leteo/vendor/leteo"),
            "npm",
            "npm uninstall -g @asanabrial/leteo",
        ),
    ];

    for (exe, manager, command) in cases {
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, b"binary").unwrap();

        let removed = uninstall_everything_for(&probe_in(temp.path()), &data, Some(exe.clone()));

        assert!(
            exe.exists(),
            "{manager} installed this, so uninstall must not take it"
        );
        assert!(!removed.binary_removed, "{manager}");
        let owner = removed
            .binary_managed_by
            .as_ref()
            .unwrap_or_else(|| panic!("{manager}'s ownership has to be reported"));
        assert_eq!(owner.manager, manager);
        assert_eq!(owner.command, command);
        // `print_json` writes exactly this pretty string, so this is the output
        // a person or a script is given.
        let report = serde_json::to_string_pretty(&removed).unwrap();
        assert!(
            report.contains(&format!("\"command\": \"{command}\"")),
            "the report has to say how it goes: {report}"
        );

        // A dry run names the owner too: it is the run that is supposed to say
        // what the real one would do.
        let dry = uninstall_everything_for(
            &SetupOptions {
                dry_run: true,
                ..probe_in(temp.path())
            },
            &data,
            Some(exe.clone()),
        );
        assert!(exe.exists());
        assert!(!dry.binary_removed);
        assert_eq!(
            dry.binary_managed_by
                .as_ref()
                .map(|owner| owner.command.as_str()),
            Some(command),
            "a dry run says whose it is and how it goes"
        );
    }
}

/// The manager keeps a link in front of the versioned file, and a run handed
/// the link is still holding the manager's binary.
#[cfg(unix)]
#[test]
fn a_link_in_front_of_a_managed_binary_is_left_too() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join("data");
    std::fs::create_dir_all(&data).unwrap();

    let target = temp.path().join("Cellar/leteo/1.2.3/bin/leteo");
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(&target, b"binary").unwrap();
    let link = temp.path().join("bin/leteo");
    std::fs::create_dir_all(link.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&target, &link).unwrap();

    let removed = uninstall_everything_for(&probe_in(temp.path()), &data, Some(link.clone()));

    assert!(link.exists(), "the link is the manager's too");
    assert!(target.exists());
    assert!(!removed.binary_removed);
    assert_eq!(
        removed.binary_managed_by.map(|owner| owner.command),
        Some("brew uninstall leteo".to_owned())
    );
}

#[test]
fn a_file_nobody_here_created_is_not_taken_with_the_rest() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(data.join("leteo.db"), b"store").unwrap();
    std::fs::write(data.join("leteo.db-wal"), b"wal").unwrap();
    std::fs::write(data.join("settings.json"), b"{}").unwrap();
    std::fs::create_dir_all(data.join("hooks")).unwrap();
    std::fs::write(data.join("hooks").join("s1.nudge"), b"stamp").unwrap();
    std::fs::write(data.join("my-notes.md"), b"mine").unwrap();
    std::fs::create_dir_all(data.join("scratch")).unwrap();
    std::fs::write(data.join("scratch").join("thing.txt"), b"also mine").unwrap();

    let removed = uninstall_everything_for(&probe_in(temp.path()), &data, fake_exe(temp.path()));

    assert!(!data.join("leteo.db").exists(), "the store has to go");
    assert!(!data.join("leteo.db-wal").exists(), "and its sidecars");
    assert!(!data.join("settings.json").exists());
    assert!(!data.join("hooks").exists(), "and the reminder clocks");

    assert!(
        data.join("my-notes.md").exists(),
        "a file Leteo never created is not Leteo's to delete"
    );
    assert!(
        data.join("scratch").join("thing.txt").exists(),
        "nor is a directory somebody else made"
    );
    assert!(
        !removed.data_dir_removed,
        "the directory stays while it still holds somebody's things"
    );
    assert!(
        removed.data_removed,
        "and that is a success with a leftover, not a partial removal"
    );
    assert!(removed.complete(), "{removed:?}");
    assert!(
        removed
            .remaining
            .iter()
            .any(|line| line.contains("my-notes.md")),
        "and it says what it kept: {:?}",
        removed.remaining
    );
}

#[test]
fn a_data_directory_of_only_leteos_own_files_goes_entirely() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(data.join("leteo.db"), b"store").unwrap();
    std::fs::write(data.join("settings.json"), b"{}").unwrap();
    std::fs::write(data.join("cloud.json"), b"{}").unwrap();
    std::fs::write(data.join("backup-20260802.db"), b"copy").unwrap();

    let removed = uninstall_everything_for(&probe_in(temp.path()), &data, fake_exe(temp.path()));

    assert!(removed.data_dir_removed, "{removed:?}");
    assert!(removed.data_removed);
    assert!(!data.exists(), "nothing of ours may be left behind");
}

#[test]
fn a_hand_made_backup_is_not_leteos_to_take() {
    // The release notes used to ask for exactly this name, and a bare `leteo.db`
    // prefix matched it: the one file a person was told to make was deleted by
    // the command that deletes the store. A migration copy went the same way,
    // and it is the only way back from a one-way upgrade.
    let temp = TempDir::new().unwrap();
    let data = temp.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(data.join("leteo.db"), b"store").unwrap();
    std::fs::write(data.join("leteo.db-wal"), b"wal").unwrap();
    std::fs::write(data.join("leteo.db.bak-before-migrate"), b"mine").unwrap();
    std::fs::write(data.join("leteo.db.pre-schema-21"), b"the way back").unwrap();

    let removed = uninstall_everything_for(&probe_in(temp.path()), &data, fake_exe(temp.path()));

    assert!(!data.join("leteo.db").exists(), "the store goes");
    assert!(!data.join("leteo.db-wal").exists(), "and its sidecars");
    assert!(
        data.join("leteo.db.bak-before-migrate").exists(),
        "a copy a person made is theirs, whatever it is called: {:?}",
        removed.remaining
    );
    assert!(
        data.join("leteo.db.pre-schema-21").exists(),
        "and a migration copy stays, because it is the way back"
    );
    assert!(removed.data_removed, "{removed:?}");
}

#[test]
fn keep_data_leaves_the_store_where_it_is() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join("data");
    let mut store =
        crate::store::Store::open(crate::store::StoreConfig::new(data.join("leteo.db"))).unwrap();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    drop(store);

    let mut options = probe_in(temp.path());
    options.keep_data = true;
    let removed = uninstall_everything_for(&options, &data, fake_exe(temp.path()));

    assert!(data.join("leteo.db").exists(), "the store is what was kept");
    assert!(
        removed.memories.is_some(),
        "and it was counted before the choice"
    );
    assert!(removed.data_kept, "{removed:?}");
    assert!(
        removed.complete(),
        "a kept store is a success, not a partial removal: {removed:?}"
    );
    assert!(
        removed
            .remaining
            .iter()
            .any(|line| line.contains("--keep-data")),
        "and it says why: {:?}",
        removed.remaining
    );
}

#[test]
fn keep_data_in_a_preview_says_the_store_would_stay() {
    // `--keep-data` is answered before the `dry_run` branch, so a preview with
    // it reports the store as kept — which is what `complete()` needs to read
    // the run as a success rather than a partial removal — and still touches
    // nothing.
    let temp = TempDir::new().unwrap();
    let data = temp.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(data.join("leteo.db"), b"store").unwrap();

    let mut options = probe_in(temp.path());
    options.dry_run = true;
    options.keep_data = true;
    let removed = uninstall_everything_for(&options, &data, fake_exe(temp.path()));

    assert!(data.join("leteo.db").exists(), "a preview touches nothing");
    assert!(
        removed.data_kept,
        "and still says the store was asked for and kept: {removed:?}"
    );
    assert!(removed.complete(), "{removed:?}");
    assert!(
        removed
            .remaining
            .iter()
            .any(|line| line.contains("--keep-data")),
        "and it says why: {:?}",
        removed.remaining
    );
}

#[test]
fn the_other_store_name_and_its_sidecars_go_too() {
    // `store.db` is the name the store carried before `leteo.db`, and the
    // sidecars are named from whichever store they belong to. All of these are
    // Leteo's own; a bare `leteo.db` prefix used to be what took the rest.
    let temp = TempDir::new().unwrap();
    let data = temp.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    let names = [
        "leteo.db",
        "leteo.db-wal",
        "leteo.db-shm",
        "leteo.db-journal",
        "store.db",
        "store.db-wal",
        "store.db-shm",
        "store.db-journal",
    ];
    for name in names {
        std::fs::write(data.join(name), b"old").unwrap();
    }

    let removed = uninstall_everything_for(&probe_in(temp.path()), &data, fake_exe(temp.path()));

    for name in names {
        assert!(
            !data.join(name).exists(),
            "{name} is the store's own name or a sidecar of it, and it goes"
        );
    }
    assert!(removed.data_dir_removed, "{removed:?}");
    assert!(removed.data_removed);
}

/// One model file with its real content, hard-linked from the checkout where
/// the filesystem allows and copied where it does not. Deleting the link leaves
/// the checkout's file alone. The hash check means a stand-in cannot be junk.
fn put_model_file(from: &Path, into: &Path, name: &str) {
    let target = into.join(name);
    if std::fs::hard_link(from.join(name), &target).is_err() {
        std::fs::copy(from.join(name), &target).unwrap();
    }
}

fn put_model(from: &Path, directory: &Path) {
    std::fs::create_dir_all(directory).unwrap();
    for (name, _) in crate::semantic::MODEL_FILES {
        put_model_file(from, directory, name);
    }
}

/// A prefix as `install.sh` lays it out, plus the other two places a model can
/// be: `bin/leteo`, `bin/model/`, `share/leteo/model/` and `data/model/`, every
/// one holding the three files.
fn installed_prefix(temp: &Path, model: &Path) -> (PathBuf, PathBuf) {
    let exe = fake_exe(temp).unwrap();
    let data = temp.join("data");
    for directory in [
        exe.parent().unwrap().join("model"),
        temp.join("prefix/share/leteo/model"),
        data.join("model"),
    ] {
        put_model(model, &directory);
    }
    (exe, data)
}

fn remaining_mentions(removed: &Removal, needles: &[&str]) -> bool {
    removed
        .remaining
        .iter()
        .any(|line| needles.iter().all(|needle| line.contains(needle)))
}

#[test]
fn uninstall_takes_the_model_from_every_place_the_binary_looks() {
    let Some(model) = crate::semantic::tests::needs_model!() else {
        return;
    };
    let temp = TempDir::new().unwrap();
    let (exe, data) = installed_prefix(temp.path(), &model);
    std::fs::write(temp.path().join("prefix/share/other"), b"not ours").unwrap();

    let removed = uninstall_everything_for(&probe_in(temp.path()), &data, Some(exe.clone()));

    for gone in [
        exe.parent().unwrap().join("model"),
        temp.path().join("prefix/share/leteo"),
        data.join("model"),
    ] {
        assert!(!gone.exists(), "{} should be gone", gone.display());
    }
    assert!(
        temp.path().join("prefix/share/other").exists(),
        "share/ and what is in it are not Leteo's"
    );
    assert!(
        !data.exists(),
        "and a data directory whose only other tenant was the model goes whole"
    );
    assert!(removed.model_removed);
    assert!(removed.complete(), "{removed:?}");
    assert_eq!(
        removed.model_files.len(),
        3 * crate::semantic::MODEL_FILES.len(),
        "{:?}",
        removed.model_files
    );
    assert!(
        model.join("model.safetensors").exists(),
        "removing a link to the checkout's file is not removing the file"
    );
}

#[test]
fn a_dry_run_lists_the_model_and_leaves_it_where_it_is() {
    let Some(model) = crate::semantic::tests::needs_model!() else {
        return;
    };
    let temp = TempDir::new().unwrap();
    let (exe, data) = installed_prefix(temp.path(), &model);
    let options = SetupOptions {
        dry_run: true,
        ..probe_in(temp.path())
    };

    let removed = uninstall_everything_for(&options, &data, Some(exe));

    assert_eq!(
        removed.model_files.len(),
        3 * crate::semantic::MODEL_FILES.len(),
        "each file once, whatever the spellings of its directory: {:?}",
        removed.model_files
    );
    assert!(data.join("model/config.json").exists());
    assert!(
        temp.path()
            .join("prefix/share/leteo/model/config.json")
            .exists()
    );
    assert!(
        temp.path()
            .join("prefix/bin/model/model.safetensors")
            .exists(),
        "and nothing was deleted"
    );
}

#[test]
fn a_model_directory_with_a_stranger_in_it_keeps_the_stranger_and_says_so() {
    let Some(model) = crate::semantic::tests::needs_model!() else {
        return;
    };
    let temp = TempDir::new().unwrap();
    let (exe, data) = installed_prefix(temp.path(), &model);
    let beside = exe.parent().unwrap().join("model");
    std::fs::write(beside.join("notes.txt"), b"mine").unwrap();

    let removed = uninstall_everything_for(&probe_in(temp.path()), &data, Some(exe.clone()));

    assert!(beside.join("notes.txt").exists());
    assert!(!beside.join("config.json").exists(), "the named files go");
    assert!(
        exe.parent().unwrap().exists(),
        "and the binary's directory stays"
    );
    assert!(
        remaining_mentions(&removed, &["model", "was kept", "notes.txt"]),
        "{:?}",
        removed.remaining
    );
}

#[test]
fn a_file_with_a_model_name_and_other_content_is_kept_and_named() {
    let Some(model) = crate::semantic::tests::needs_model!() else {
        return;
    };
    let temp = TempDir::new().unwrap();
    let (exe, data) = installed_prefix(temp.path(), &model);
    let theirs = data.join("model").join("config.json");
    std::fs::remove_file(&theirs).unwrap();
    std::fs::write(&theirs, b"{\"somebody\": \"else\"}").unwrap();

    let removed = uninstall_everything_for(&probe_in(temp.path()), &data, Some(exe));

    assert_eq!(
        std::fs::read(&theirs).unwrap(),
        b"{\"somebody\": \"else\"}",
        "a name is not a reason to delete"
    );
    assert!(
        remaining_mentions(
            &removed,
            &["config.json", "not the model this build installs"]
        ),
        "{:?}",
        removed.remaining
    );
    assert!(
        !data.join("model/model.safetensors").exists(),
        "the files that are the model still go"
    );
    assert!(
        removed.complete(),
        "kept on purpose is not a failed removal: {removed:?}"
    );
}

#[cfg(unix)]
#[test]
fn a_model_directory_that_is_a_symbolic_link_is_not_followed() {
    let Some(model) = crate::semantic::tests::needs_model!() else {
        return;
    };
    let temp = TempDir::new().unwrap();
    let (exe, data) = installed_prefix(temp.path(), &model);
    let beside = exe.parent().unwrap().join("model");
    let elsewhere = temp.path().join("somebody-elses-model");
    std::fs::remove_dir_all(&beside).unwrap();
    put_model(&model, &elsewhere);
    std::os::unix::fs::symlink(&elsewhere, &beside).unwrap();

    let removed = uninstall_everything_for(&probe_in(temp.path()), &data, Some(exe));

    for (name, _) in crate::semantic::MODEL_FILES {
        assert!(
            elsewhere.join(name).exists(),
            "{name} behind the link must survive"
        );
    }
    assert!(beside.exists(), "and so must the link");
    assert!(
        remaining_mentions(&removed, &["symbolic link"]),
        "{:?}",
        removed.remaining
    );
}

#[cfg(unix)]
#[test]
fn a_file_that_cannot_be_deleted_makes_the_removal_incomplete() {
    use std::os::unix::fs::PermissionsExt;
    let Some(model) = crate::semantic::tests::needs_model!() else {
        return;
    };
    let temp = TempDir::new().unwrap();
    let (exe, data) = installed_prefix(temp.path(), &model);
    let locked = data.join("model");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
    // root deletes out of a read-only directory, so there is nothing to provoke.
    let provokable = std::fs::write(locked.join("probe"), b"").is_err();

    let removed = uninstall_everything_for(&probe_in(temp.path()), &data, Some(exe));

    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    if !provokable {
        eprintln!("skipped: this user can write into a read-only directory");
        return;
    }
    assert!(!removed.model_removed, "{removed:?}");
    assert!(
        !removed.complete(),
        "a model that could not be taken is not a clean uninstall"
    );
    assert!(
        remaining_mentions(&removed, &["data/model/config.json"]),
        "{:?}",
        removed.remaining
    );
}

#[cfg(unix)]
#[test]
fn a_model_file_that_is_a_symbolic_link_is_not_a_regular_file_and_stays() {
    let Some(model) = crate::semantic::tests::needs_model!() else {
        return;
    };
    let temp = TempDir::new().unwrap();
    let (exe, data) = installed_prefix(temp.path(), &model);
    let link = data.join("model").join("model.safetensors");
    std::fs::remove_file(&link).unwrap();
    std::os::unix::fs::symlink(model.join("model.safetensors"), &link).unwrap();

    let removed = uninstall_everything_for(&probe_in(temp.path()), &data, Some(exe));

    assert!(
        std::fs::symlink_metadata(&link).is_ok(),
        "only a regular file is Leteo's to delete"
    );
    assert!(
        remaining_mentions(&removed, &["model.safetensors", "not a regular file"]),
        "{:?}",
        removed.remaining
    );
}

#[cfg(unix)]
#[test]
fn an_empty_model_directory_that_cannot_be_removed_is_not_blamed_on_a_stranger() {
    use std::os::unix::fs::PermissionsExt;
    let Some(model) = crate::semantic::tests::needs_model!() else {
        return;
    };
    let temp = TempDir::new().unwrap();
    let (exe, data) = installed_prefix(temp.path(), &model);
    // The files can go (their directory is writable) but the directory cannot,
    // because its parent is not.
    std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o555)).unwrap();
    let provokable = std::fs::create_dir(data.join("probe")).is_err();

    let removed = uninstall_everything_for(&probe_in(temp.path()), &data, Some(exe));

    std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o755)).unwrap();
    if !provokable {
        eprintln!("skipped: this user can write into a read-only directory");
        return;
    }
    let line = removed
        .remaining
        .iter()
        .find(|line| line.starts_with(&data.join("model").display().to_string()))
        .unwrap_or_else(|| panic!("the model directory is named: {:?}", removed.remaining));
    assert!(
        !line.contains("did not put there"),
        "nothing foreign is in it, and the report must not say so: {line}"
    );
}

#[test]
fn a_share_leteo_with_something_else_in_it_is_kept_and_named() {
    let Some(model) = crate::semantic::tests::needs_model!() else {
        return;
    };
    let temp = TempDir::new().unwrap();
    let (exe, data) = installed_prefix(temp.path(), &model);
    let theirs = temp.path().join("prefix/share/leteo/docs");
    std::fs::write(&theirs, b"mine").unwrap();

    let removed = uninstall_everything_for(&probe_in(temp.path()), &data, Some(exe));

    assert!(theirs.exists());
    assert!(
        !temp.path().join("prefix/share/leteo/model").exists(),
        "the model directory it emptied still goes"
    );
    assert!(
        remaining_mentions(&removed, &["share", "leteo", "was kept", "docs"]),
        "{:?}",
        removed.remaining
    );
}

#[test]
fn a_data_directory_named_like_the_installers_share_leteo_is_still_the_data_directory() {
    let Some(model) = crate::semantic::tests::needs_model!() else {
        return;
    };
    let temp = TempDir::new().unwrap();
    let exe = fake_exe(temp.path());
    let data = temp.path().join("share/leteo");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(data.join("leteo.db"), b"store").unwrap();
    put_model(&model, &data.join("model"));

    let removed = uninstall_everything_for(&probe_in(temp.path()), &data, exe);

    assert!(
        !remaining_mentions(&removed, &["was kept"]),
        "{:?}",
        removed.remaining
    );
    assert!(removed.complete(), "{removed:?}");
    assert!(removed.data_removed && removed.data_dir_removed);
    assert!(!data.exists());
}

#[test]
fn a_model_that_was_never_installed_is_not_a_failure_or_a_mention() {
    let temp = TempDir::new().unwrap();
    let exe = fake_exe(temp.path());
    let data = temp.path().join("data");
    std::fs::create_dir_all(&data).unwrap();

    let removed = uninstall_everything_for(&probe_in(temp.path()), &data, exe);

    assert!(removed.model_removed && removed.model_files.is_empty());
    assert!(
        removed.remaining.iter().all(|line| !line.contains("model")),
        "{:?}",
        removed.remaining
    );
}

#[cfg(unix)]
#[test]
fn a_missing_path_under_a_link_resolves_like_the_one_that_exists() {
    let temp = TempDir::new().unwrap();
    let real = temp.path().join("real");
    std::fs::create_dir_all(&real).unwrap();
    let link = temp.path().join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();

    assert_eq!(
        resolved(&link.join("not-yet").join("deeper")),
        real.canonicalize().unwrap().join("not-yet").join("deeper")
    );
}
