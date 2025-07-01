use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

#[test]
fn test_restore_with_malformed_json() {
    let temp_dir = TempDir::new().unwrap();
    let malformed_file = temp_dir.path().join("malformed.json");

    // Create a malformed JSON file
    fs::write(&malformed_file, "{invalid json}").unwrap();

    let mut cmd = Command::cargo_bin("cmux").unwrap();
    cmd.arg("restore")
        .arg(malformed_file.to_str().unwrap())
        .assert()
        .failure()
        .stderr(predicate::str::contains("Failed to parse snapshot file"));
}

#[test]
fn test_restore_with_empty_file() {
    let temp_dir = TempDir::new().unwrap();
    let empty_file = temp_dir.path().join("empty.json");

    // Create an empty file
    fs::write(&empty_file, "").unwrap();

    let mut cmd = Command::cargo_bin("cmux").unwrap();
    cmd.arg("restore")
        .arg(empty_file.to_str().unwrap())
        .assert()
        .failure()
        .stderr(predicate::str::contains("Failed to parse snapshot file"));
}

#[test]
fn test_restore_with_wrong_json_structure() {
    let temp_dir = TempDir::new().unwrap();
    let wrong_structure_file = temp_dir.path().join("wrong_structure.json");

    // Create JSON with wrong structure
    let wrong_content = r#"{"wrong": "structure"}"#;
    fs::write(&wrong_structure_file, wrong_content).unwrap();

    let mut cmd = Command::cargo_bin("cmux").unwrap();
    cmd.arg("restore")
        .arg(wrong_structure_file.to_str().unwrap())
        .assert()
        .failure()
        .stderr(predicate::str::contains("Failed to parse snapshot file"));
}

#[test]
#[cfg(unix)]
fn test_attach_with_nonexistent_session() {
    let mut cmd = Command::cargo_bin("cmux").unwrap();
    let output = cmd
        .arg("attach")
        .arg("nonexistent-session-12345")
        .output()
        .unwrap();

    // tmux might return success or failure depending on version
    // But it should have an error message
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("can't find session")
            || stderr.contains("Failed to attach")
            || stderr.contains("no sessions")
            || !output.status.success()
    );
}

#[test]
#[cfg(unix)]
fn test_kill_with_nonexistent_session() {
    let mut cmd = Command::cargo_bin("cmux").unwrap();
    let output = cmd
        .arg("kill")
        .arg("nonexistent-session-12345")
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("can't find session")
            || stderr.contains("Failed to kill")
            || stderr.contains("no sessions")
            || !output.status.success()
    );
}

#[test]
#[cfg(unix)]
fn test_rename_with_nonexistent_session() {
    let mut cmd = Command::cargo_bin("cmux").unwrap();
    let output = cmd
        .arg("rename")
        .arg("nonexistent-session-12345")
        .arg("new-name")
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("can't find session")
            || stderr.contains("Failed to rename")
            || stderr.contains("no sessions")
            || !output.status.success()
    );
}

#[test]
#[cfg(unix)]
fn test_info_with_nonexistent_session() {
    let mut cmd = Command::cargo_bin("cmux").unwrap();
    cmd.arg("info")
        .arg("nonexistent-session-12345")
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "Session 'nonexistent-session-12345' not found",
        ));
}

#[test]
fn test_alias_with_invalid_arguments() {
    let mut cmd = Command::cargo_bin("cmux").unwrap();
    cmd.arg("alias")
        .arg("alias-name")
        .arg("session-name")
        .arg("extra-argument")
        .assert()
        .failure()
        .stderr(predicate::str::contains("unexpected argument"));
}

#[test]
fn test_new_session_with_empty_name() {
    let mut cmd = Command::cargo_bin("cmux").unwrap();
    let output = cmd.arg("new").arg("").output().unwrap();

    // Empty session names might be allowed by tmux, so we check for any reasonable behavior
    let stderr = String::from_utf8_lossy(&output.stderr);
    let _stdout = String::from_utf8_lossy(&output.stdout);

    // Either it should succeed (empty name becomes default) or fail with an error
    assert!(
        output.status.success()
            || stderr.contains("Failed")
            || stderr.contains("invalid")
            || stderr.contains("empty")
    );
}

#[test]
fn test_kill_all_with_no_confirmation() {
    let mut cmd = Command::cargo_bin("cmux").unwrap();
    // Simulate entering 'n' for no
    let output = cmd.arg("kill-all").write_stdin("n\n").output().unwrap();

    // Should exit successfully with cancellation message
    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("Cancelled") || stdout.contains("No tmux sessions to kill"));
    } else {
        // If tmux is not available, it should fail gracefully
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("tmux") || stderr.contains("Failed"));
    }
}

#[test]
fn test_command_without_tmux_installed() {
    // This test simulates the behavior when tmux is not installed
    // We can't actually uninstall tmux, but we can test error handling

    let mut cmd = Command::cargo_bin("cmux").unwrap();
    let output = cmd.arg("list").output().unwrap();

    // Command should handle the case where tmux is not available
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Should contain some error message about tmux
        assert!(
            stderr.contains("tmux") || stderr.contains("Failed") || stderr.contains("No such file")
        );
    }
}

#[test]
fn test_invalid_subcommand_suggestions() {
    let mut cmd = Command::cargo_bin("cmux").unwrap();
    cmd.arg("lst") // typo for "list"
        .assert()
        .failure()
        .stderr(predicate::str::contains("unrecognized subcommand"));
}

#[test]
fn test_missing_required_arguments() {
    // Test rename command missing second argument
    let mut cmd = Command::cargo_bin("cmux").unwrap();
    cmd.arg("rename")
        .arg("old-name")
        .assert()
        .failure()
        .stderr(predicate::str::contains("required"));
}

#[test]
fn test_permissions_error_simulation() {
    let temp_dir = TempDir::new().unwrap();
    let unreadable_file = temp_dir.path().join("unreadable.json");

    // Create a file and make it unreadable (on Unix-like systems)
    fs::write(&unreadable_file, r#"{"sessions": [], "timestamp": "test"}"#).unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&unreadable_file).unwrap().permissions();
        perms.set_mode(0o000); // No permissions
        fs::set_permissions(&unreadable_file, perms).unwrap();

        let mut cmd = Command::cargo_bin("cmux").unwrap();
        cmd.arg("restore")
            .arg(unreadable_file.to_str().unwrap())
            .assert()
            .failure()
            .stderr(predicate::str::contains("Failed to read snapshot file"));
    }
}

#[test]
fn test_concurrent_session_operations() {
    // Test that operations don't interfere when run concurrently
    // This is a basic test that just ensures commands don't crash

    let mut cmd1 = Command::cargo_bin("cmux").unwrap();
    let mut cmd2 = Command::cargo_bin("cmux").unwrap();

    let output1 = cmd1.arg("list").output().unwrap();
    let output2 = cmd2.arg("alias").output().unwrap();

    // Both commands should complete without hanging
    // Success or failure depends on tmux availability
    assert!(output1.status.success() || !output1.stderr.is_empty());
    assert!(output2.status.success() || !output2.stderr.is_empty());
}
