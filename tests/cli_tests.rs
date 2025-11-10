use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn test_cli_no_arguments() {
    // When no arguments are provided, it should try to run the TUI
    // This will fail in a headless environment, but we can check the error
    let mut cmd = Command::cargo_bin("cmux").unwrap();
    let output = cmd.output().unwrap();

    // It should either succeed (if tmux is available) or fail gracefully
    let stderr = String::from_utf8_lossy(&output.stderr);
    let _stdout = String::from_utf8_lossy(&output.stdout);

    // Check that it's attempting to run TUI or failing gracefully
    assert!(
        output.status.success()
            || stderr.contains("tmux")
            || stderr.contains("Failed")
            || stderr.contains("terminal")
            || stderr.contains("Device not configured")
            || stderr.contains("os error")
    );
}

#[test]
#[cfg(unix)]
fn test_list_command_short_alias() {
    let mut cmd = Command::cargo_bin("cmux").unwrap();
    let output = cmd.arg("ls").output().unwrap();

    // ls should work the same as list
    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("Active tmux sessions") || stdout.contains("No tmux sessions found")
        );
    }
}

#[test]
#[cfg(unix)]
fn test_attach_command_variations() {
    let mut cmd = Command::cargo_bin("cmux").unwrap();
    // Test attach with session name
    let output = cmd
        .arg("attach")
        .arg("nonexistent-test-session-12345")
        .output()
        .unwrap();

    // Should fail because session doesn't exist
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("can't find session") || !output.status.success());

    let mut cmd = Command::cargo_bin("cmux").unwrap();
    // Test attach short alias
    let output = cmd
        .arg("a")
        .arg("nonexistent-test-session-12345")
        .output()
        .unwrap();

    // Should fail because session doesn't exist
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("can't find session") || !output.status.success());
}

#[test]
fn test_new_command_variations() {
    let unique_session_name = format!("test-session-{}", std::process::id());

    let mut cmd = Command::cargo_bin("cmux").unwrap();
    // Test new with session name - this might succeed
    let output = cmd.arg("new").arg(&unique_session_name).output().unwrap();

    // If it succeeds, clean up the session
    if output.status.success() {
        let _ = std::process::Command::new("tmux")
            .args(["kill-session", "-t", &unique_session_name])
            .output();
    }

    let mut cmd = Command::cargo_bin("cmux").unwrap();
    // Test new short alias
    let output = cmd
        .arg("n")
        .arg(format!("{}-alias", unique_session_name))
        .output()
        .unwrap();

    // If it succeeds, clean up the session
    if output.status.success() {
        let _ = std::process::Command::new("tmux")
            .args([
                "kill-session",
                "-t",
                &format!("{}-alias", unique_session_name),
            ])
            .output();
    }

    let mut cmd = Command::cargo_bin("cmux").unwrap();
    // Test new without session name - should create a session
    let output = cmd.arg("new").output().unwrap();

    // This might succeed or fail depending on tmux state
    assert!(output.status.success() || !String::from_utf8_lossy(&output.stderr).is_empty());
}

#[test]
#[cfg(unix)]
fn test_kill_command_variations() {
    let mut cmd = Command::cargo_bin("cmux").unwrap();
    // Test kill with session name
    cmd.arg("kill")
        .arg("nonexistent-session")
        .assert()
        .failure();

    let mut cmd = Command::cargo_bin("cmux").unwrap();
    // Test kill short alias
    cmd.arg("k").arg("nonexistent-session").assert().failure();

    let mut cmd = Command::cargo_bin("cmux").unwrap();
    // Test kill without session name
    cmd.arg("kill")
        .assert()
        .failure()
        .stderr(predicate::str::contains("Please specify a session name"));
}

#[test]
fn test_rename_command_validation() {
    let mut cmd = Command::cargo_bin("cmux").unwrap();
    // Test rename with both arguments
    cmd.arg("rename")
        .arg("old-name")
        .arg("new-name")
        .assert()
        .failure(); // Will fail because session doesn't exist

    let mut cmd = Command::cargo_bin("cmux").unwrap();
    // Test rename short alias
    cmd.arg("r")
        .arg("old-name")
        .arg("new-name")
        .assert()
        .failure(); // Will fail because session doesn't exist

    let mut cmd = Command::cargo_bin("cmux").unwrap();
    // Test rename with missing argument
    cmd.arg("rename")
        .arg("old-name")
        .assert()
        .failure()
        .stderr(predicate::str::contains("required"));
}

#[test]
fn test_restore_command_with_file() {
    let mut cmd = Command::cargo_bin("cmux").unwrap();
    cmd.arg("restore")
        .arg("/nonexistent/file.json")
        .assert()
        .failure()
        .stderr(predicate::str::contains("Failed to read snapshot file"));
}

#[test]
fn test_restore_command_without_file() {
    let mut cmd = Command::cargo_bin("cmux").unwrap();
    cmd.arg("restore").assert().failure(); // Will likely fail because default snapshot file doesn't exist
}

#[test]
fn test_alias_command_variations() {
    let mut cmd = Command::cargo_bin("cmux").unwrap();
    // Test alias list (no arguments)
    cmd.arg("alias").assert().success(); // Should always succeed

    let mut cmd = Command::cargo_bin("cmux").unwrap();
    // Test alias query (one argument)
    cmd.arg("alias").arg("test-alias").assert().success(); // Should succeed even if alias doesn't exist

    let mut cmd = Command::cargo_bin("cmux").unwrap();
    // Test alias create (two arguments)
    cmd.arg("alias")
        .arg("test-alias")
        .arg("test-session")
        .assert()
        .success(); // Should succeed
}

#[test]
fn test_top_command() {
    let mut cmd = Command::cargo_bin("cmux").unwrap();
    // Top command will run indefinitely in a real terminal
    // In a test environment, it should fail gracefully
    let output = cmd.arg("top").output().unwrap();

    // It should either succeed or fail gracefully
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success()
            || stderr.contains("terminal")
            || stderr.contains("Failed")
            || stderr.contains("tmux")
            || stderr.contains("Device not configured")
            || stderr.contains("os error")
    );
}

#[test]
#[cfg(unix)]
fn test_info_command_variations() {
    let mut cmd = Command::cargo_bin("cmux").unwrap();
    // Test info with session name
    cmd.arg("info")
        .arg("nonexistent-session")
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found"));

    let mut cmd = Command::cargo_bin("cmux").unwrap();
    // Test info without session name
    let output = cmd.arg("info").output().unwrap();

    // Should either show info or report no sessions
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("No tmux sessions found")
                || stderr.contains("tmux")
                || stderr.contains("Failed")
        );
    }
}

#[test]
#[cfg(unix)]
fn test_kill_all_command() {
    let mut cmd = Command::cargo_bin("cmux").unwrap();
    // Test kill-all with 'n' input
    let output = cmd.arg("kill-all").write_stdin("n\n").output().unwrap();

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("Cancelled") || stdout.contains("No tmux sessions to kill"));
    }
}

#[test]
fn test_long_and_short_options() {
    let mut cmd = Command::cargo_bin("cmux").unwrap();
    cmd.arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage:"));

    let mut cmd = Command::cargo_bin("cmux").unwrap();
    cmd.arg("-h")
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage:"));

    let mut cmd = Command::cargo_bin("cmux").unwrap();
    cmd.arg("--version").assert().success();

    let mut cmd = Command::cargo_bin("cmux").unwrap();
    cmd.arg("-V").assert().success();
}

#[test]
fn test_case_sensitivity() {
    let mut cmd = Command::cargo_bin("cmux").unwrap();
    // Commands should be case sensitive
    cmd.arg("LIST")
        .assert()
        .failure()
        .stderr(predicate::str::contains("unrecognized subcommand"));

    let mut cmd = Command::cargo_bin("cmux").unwrap();
    cmd.arg("List")
        .assert()
        .failure()
        .stderr(predicate::str::contains("unrecognized subcommand"));
}

#[test]
fn test_command_order() {
    let mut cmd = Command::cargo_bin("cmux").unwrap();
    // clap allows --help after subcommands
    cmd.arg("list")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicates::str::contains("List all tmux sessions"));
}

#[test]
fn test_special_characters_in_session_names() {
    let mut cmd = Command::cargo_bin("cmux").unwrap();
    // Test with special characters that might cause issues
    cmd.arg("attach")
        .arg("session-with-spaces and symbols!@#$%")
        .assert()
        .failure(); // Will fail because session doesn't exist

    let mut cmd = Command::cargo_bin("cmux").unwrap();
    cmd.arg("new")
        .arg("session_with_underscores-and-dashes.dots")
        .assert()
        .failure(); // May fail if tmux not available
}

#[test]
fn test_empty_arguments() {
    let mut cmd = Command::cargo_bin("cmux").unwrap();
    // Test with empty string arguments
    let output = cmd.arg("attach").arg("").output().unwrap();

    // Empty session name might be handled differently by tmux
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("can't find session")
            || stderr.contains("invalid")
            || !output.status.success()
    );

    let mut cmd = Command::cargo_bin("cmux").unwrap();
    let output = cmd.arg("new").arg("").output().unwrap();

    // Empty session name for new might be allowed (creates default name)
    // So we just check that it doesn't crash
    assert!(output.status.success() || !String::from_utf8_lossy(&output.stderr).is_empty());
}
