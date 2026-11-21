//! The uninstall scripts decide that the binary started by looking for the line
//! `leteo uninstall --yes` prints first, and a shell script cannot read the
//! constant that holds it. This keeps their copies from drifting: a script
//! looking for stale text would count every binary as one that never started and
//! delete a model file the binary had chosen to keep.
//!
//! The scripts matching the constant is only half of it. Nothing here ran the
//! binary, so the `eprintln!` could be deleted and every test would still pass —
//! the flows in `check_install.sh` accept the report's `model_removed` line too —
//! and the marker could just as easily start being printed by the dry-run
//! preview. `the_binary_prints_the_marker_when_it_removes_and_not_when_it_previews`
//! runs both, which is the only thing that holds the marker to what it promises.

use std::path::Path;
use std::process::Command;

fn marker_lines_in(script: &str) -> Vec<String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("scripts")
        .join(script);
    let text = std::fs::read_to_string(&path).expect("read the script");
    text.lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .filter(|line| line.contains("uninstall: started"))
        .map(str::to_owned)
        .collect()
}

#[test]
fn the_uninstallers_look_for_exactly_the_line_the_binary_prints() {
    for script in ["uninstall.sh", "uninstall.ps1"] {
        let lines = marker_lines_in(script);
        assert_eq!(
            lines.len(),
            1,
            "{script} has to name the marker on exactly one line: {lines:?}"
        );
        assert!(
            lines[0].contains(&format!("'{}'", leteo::setup::UNINSTALL_STARTED))
                || lines[0].contains(&format!("\"{}\"", leteo::setup::UNINSTALL_STARTED)),
            "{script} looks for a different line than the binary prints: {}",
            lines[0]
        );
    }
}

/// A binary built before the marker existed is recognised by its report, and the
/// scripts match one line of it. The report is what `print_json` makes of a
/// `Removal`, so the line is taken from the real serialisation.
#[test]
fn the_uninstallers_recognise_a_line_of_the_report_the_binary_prints() {
    let removal = leteo::setup::Removal {
        dry_run: false,
        agents: Vec::new(),
        data_dir: std::path::PathBuf::new(),
        data_dir_removed: false,
        data_removed: false,
        data_kept: false,
        memories: None,
        binary: None,
        binary_removed: false,
        model_files: Vec::new(),
        model_removed: true,
        remaining: Vec::new(),
    };
    let report = serde_json::to_string_pretty(&removal).expect("serialise the report");
    assert!(
        report
            .lines()
            .any(|line| line == "  \"model_removed\": true,"),
        "the report no longer has the line the scripts look for:\n{report}"
    );
    for script in ["uninstall.sh", "uninstall.ps1"] {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("scripts")
            .join(script);
        let text = std::fs::read_to_string(&path).expect("read the script");
        let named = text
            .lines()
            .filter(|line| !line.trim_start().starts_with('#'))
            .any(|line| line.contains("model_removed") && line.contains("(true|false),?"));
        assert!(
            named,
            "{script} does not match the report's model_removed line"
        );
    }
}

/// `uninstall --yes` prints the marker and the preview does not.
///
/// The line has to be the first thing the run says: it is what a script reads to
/// tell a binary that started from one that failed to exec, and a binary that
/// printed it after judging a model file could have deleted one the run would
/// have kept. The two runs together are the point — a test that only ran `--yes`
/// would not notice the marker creeping into the preview, and one that only
/// compared the scripts with the constant would not notice the `eprintln!` going
/// away.
#[test]
fn the_binary_prints_the_marker_when_it_removes_and_not_when_it_previews() {
    let temp = tempfile::tempdir().expect("a temporary directory");
    let home = temp.path().join("home");
    let data = temp.path().join("data");
    std::fs::create_dir_all(&home).expect("create the temporary home");
    std::fs::create_dir_all(&data).expect("create the temporary data directory");

    // A copy, because `uninstall --yes` removes the program that is running:
    // pointed at the built binary it would take `target/debug/leteo` out from
    // under every other test in the same run.
    let binary = temp
        .path()
        .join(format!("leteo{}", std::env::consts::EXE_SUFFIX));
    std::fs::copy(env!("CARGO_BIN_EXE_leteo"), &binary).expect("copy the binary");

    let run = |yes: bool| {
        let mut command = Command::new(&binary);
        command.arg("uninstall");
        if yes {
            command.arg("--yes");
        }
        // The home is temporary as well, and the variables that can point the
        // agent paths or the model somewhere else are taken out: the removal
        // walks every agent's configuration and judges the model files, and a
        // test must not edit the ones this machine is using.
        command.env("HOME", &home);
        command.env("LETEO_DATA_DIR", &data);
        command.env_remove("LETEO_DATABASE");
        command.env_remove("LETEO_MODEL_DIR");
        command.env_remove("XDG_CONFIG_HOME");
        command.env_remove("CLAUDE_CONFIG_DIR");
        command.env_remove("DSH_HOME");
        command.output().expect("run the binary")
    };

    let preview = run(false);
    // The positive control comes before the negative one: the preview prints no
    // marker, and a run that failed before printing anything would satisfy that
    // on its own. Its report is what says the run reached the end.
    let report: serde_json::Value =
        serde_json::from_slice(&preview.stdout).expect("the preview prints its report");
    assert_eq!(
        report["dry_run"],
        serde_json::json!(true),
        "the preview did not report a dry run: {report}"
    );
    let stderr = String::from_utf8_lossy(&preview.stderr);
    assert!(
        !stderr.contains(leteo::setup::UNINSTALL_STARTED),
        "the dry run printed the line the scripts read as a started binary: {stderr}"
    );

    let removal = run(true);
    let stderr = String::from_utf8_lossy(&removal.stderr);
    assert_eq!(
        stderr.lines().next(),
        Some(leteo::setup::UNINSTALL_STARTED),
        "the marker is not the first line the uninstall printed: {stderr}"
    );
}
