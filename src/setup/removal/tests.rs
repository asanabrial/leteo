use std::path::Path;

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
        ..SetupOptions::default()
    }
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
    let removed = uninstall_everything(&options, &data);

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

    let removed = uninstall_everything(&probe_in(temp.path()), &data);

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

    let removed = uninstall_everything(&probe_in(temp.path()), &data);

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

    let removed = uninstall_everything(&probe_in(temp.path()), &data);

    assert_eq!(removed.memories, None, "unreadable is not zero");
    assert!(removed.data_dir_removed, "and it still goes");
}

#[test]
fn an_absent_data_directory_is_not_a_failure() {
    let temp = TempDir::new().unwrap();
    let removed = uninstall_everything(&probe_in(temp.path()), &temp.path().join("gone"));
    assert_eq!(removed.memories, None);
    assert!(removed.remaining.iter().all(|line| !line.contains("gone")));
}

#[cfg(windows)]
#[test]
fn windows_reports_the_binary_instead_of_pretending_it_removed_it() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join("data");
    std::fs::create_dir_all(&data).unwrap();

    let removed = uninstall_everything(&probe_in(temp.path()), &data);

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

    let removed = uninstall_everything(&probe_in(temp.path()), &data);

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

    let removed = uninstall_everything(&probe_in(temp.path()), &data);

    assert!(removed.data_dir_removed, "{removed:?}");
    assert!(removed.data_removed);
    assert!(!data.exists(), "nothing of ours may be left behind");
}
