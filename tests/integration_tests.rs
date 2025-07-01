use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

#[test]
fn test_cmux_help() {
    let mut cmd = Command::cargo_bin("cmux").unwrap();
    cmd.arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("A mobile-friendly tmux wrapper"))
        .stdout(predicate::str::contains("Commands:"))
        .stdout(predicate::str::contains("list"))
        .stdout(predicate::str::contains("attach"))
        .stdout(predicate::str::contains("new"))
        .stdout(predicate::str::contains("kill"));
}

#[test]
fn test_cmux_version() {
    let mut cmd = Command::cargo_bin("cmux").unwrap();
    cmd.arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("cmux"));
}

#[test]
fn test_list_command() {
    let mut cmd = Command::cargo_bin("cmux").unwrap();
    // This will fail if tmux is not installed, but that's expected in CI
    let output = cmd.arg("list").output().unwrap();

    // Check that the command runs without crashing
    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        // If successful, it should either show sessions or say "No tmux sessions found"
        assert!(
            stdout.contains("Active tmux sessions") || stdout.contains("No tmux sessions found")
        );
    } else {
        // If tmux is not available, the command should fail gracefully
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("tmux") || stderr.contains("Failed"));
    }
}

#[test]
fn test_alias_command_without_args() {
    let mut cmd = Command::cargo_bin("cmux").unwrap();
    cmd.arg("alias").assert().success();
}

#[test]
fn test_info_command_without_session() {
    let mut cmd = Command::cargo_bin("cmux").unwrap();
    // This should fail gracefully when no sessions exist
    let output = cmd.arg("info").output().unwrap();

    // The command might succeed or fail depending on whether tmux is running
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Either it shows session info or reports no sessions/error
    assert!(
        stdout.contains("Session Information")
            || stderr.contains("No tmux sessions found")
            || stderr.contains("tmux")
            || stderr.contains("Failed")
    );
}

#[test]
fn test_restore_with_invalid_file() {
    let temp_dir = TempDir::new().unwrap();
    let invalid_file = temp_dir.path().join("nonexistent.json");

    let mut cmd = Command::cargo_bin("cmux").unwrap();
    cmd.arg("restore")
        .arg(invalid_file.to_str().unwrap())
        .assert()
        .failure()
        .stderr(predicate::str::contains("Failed to read snapshot file"));
}

#[test]
fn test_restore_with_valid_snapshot_file() {
    let temp_dir = TempDir::new().unwrap();
    let snapshot_file = temp_dir.path().join("test_snapshot.json");

    // Create a valid snapshot file
    let snapshot_content = r#"{
        "sessions": [
            {
                "name": "test-session",
                "windows": 1,
                "attached": false,
                "created": "1234567890",
                "activity": "1234567890"
            }
        ],
        "timestamp": "2024-01-01T00:00:00"
    }"#;

    fs::write(&snapshot_file, snapshot_content).unwrap();

    let mut cmd = Command::cargo_bin("cmux").unwrap();
    let output = cmd
        .arg("restore")
        .arg(snapshot_file.to_str().unwrap())
        .output()
        .unwrap();

    // The command might succeed or fail depending on tmux availability
    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("Restoring 1 sessions from snapshot"));
    } else {
        // If tmux is not available, it should fail gracefully
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("tmux") || stderr.contains("Failed"));
    }
}

#[test]
fn test_rename_command_validation() {
    let mut cmd = Command::cargo_bin("cmux").unwrap();
    // Test that rename requires both arguments
    cmd.arg("rename").arg("old-name").assert().failure();
}

#[test]
fn test_kill_without_session_name() {
    let mut cmd = Command::cargo_bin("cmux").unwrap();
    cmd.arg("kill")
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "Please specify a session name to kill",
        ));
}

#[test]
fn test_subcommand_aliases() {
    // Test that aliases work
    let mut cmd = Command::cargo_bin("cmux").unwrap();
    cmd.arg("ls") // alias for list
        .assert()
        .code(predicate::eq(0).or(predicate::eq(1))); // Either success or tmux not found

    let mut cmd = Command::cargo_bin("cmux").unwrap();
    cmd.arg("a") // alias for attach
        .assert()
        .code(predicate::eq(0).or(predicate::eq(1)));
}

#[test]
fn test_invalid_command() {
    let mut cmd = Command::cargo_bin("cmux").unwrap();
    cmd.arg("invalid-command")
        .assert()
        .failure()
        .stderr(predicate::str::contains("unrecognized subcommand"));
}
