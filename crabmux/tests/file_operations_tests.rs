use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;
use std::fs;
use std::env;

fn setup_temp_home(temp_dir: &TempDir) {
    env::set_var("HOME", temp_dir.path().to_str().unwrap());
}

#[test]
fn test_alias_file_operations() {
    let temp_dir = TempDir::new().unwrap();
    setup_temp_home(&temp_dir);
    
    // Test creating an alias
    let mut cmd = Command::cargo_bin("cmux").unwrap();
    cmd.arg("alias")
        .arg("test-alias")
        .arg("test-session")
        .env("HOME", temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Created alias 'test-alias' for session 'test-session'"));
    
    // Verify alias file was created
    let alias_file = temp_dir.path().join(".cmux_aliases.json");
    assert!(alias_file.exists());
    
    // Verify file contents
    let content = fs::read_to_string(&alias_file).unwrap();
    assert!(content.contains("test-alias"));
    assert!(content.contains("test-session"));
    
    // Test querying the alias
    let mut cmd = Command::cargo_bin("cmux").unwrap();
    cmd.arg("alias")
        .arg("test-alias")
        .env("HOME", temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("test-alias -> test-session"));
    
    // Test listing all aliases
    let mut cmd = Command::cargo_bin("cmux").unwrap();
    cmd.arg("alias")
        .env("HOME", temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Current aliases:"))
        .stdout(predicate::str::contains("test-alias -> test-session"));
}

#[test]
fn test_alias_query_nonexistent() {
    let temp_dir = TempDir::new().unwrap();
    
    let mut cmd = Command::cargo_bin("cmux").unwrap();
    cmd.arg("alias")
        .arg("nonexistent-alias")
        .env("HOME", temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Alias 'nonexistent-alias' not found"));
}

#[test]
fn test_alias_list_empty() {
    let temp_dir = TempDir::new().unwrap();
    
    let mut cmd = Command::cargo_bin("cmux").unwrap();
    cmd.arg("alias")
        .env("HOME", temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("No aliases defined"));
}

#[test]
fn test_alias_overwrite_existing() {
    let temp_dir = TempDir::new().unwrap();
    
    // Create first alias
    let mut cmd = Command::cargo_bin("cmux").unwrap();
    cmd.arg("alias")
        .arg("test-alias")
        .arg("session1")
        .env("HOME", temp_dir.path())
        .assert()
        .success();
    
    // Overwrite with new alias
    let mut cmd = Command::cargo_bin("cmux").unwrap();
    cmd.arg("alias")
        .arg("test-alias")
        .arg("session2")
        .env("HOME", temp_dir.path())
        .assert()
        .success();
    
    // Verify the alias was updated
    let mut cmd = Command::cargo_bin("cmux").unwrap();
    cmd.arg("alias")
        .arg("test-alias")
        .env("HOME", temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("test-alias -> session2"));
}

#[test]
fn test_snapshot_restore_with_valid_file() {
    let temp_dir = TempDir::new().unwrap();
    let snapshot_file = temp_dir.path().join("test_snapshot.json");
    
    // Create a valid snapshot file
    let snapshot_content = r#"{
        "sessions": [
            {
                "name": "restored-session1",
                "windows": 2,
                "attached": false,
                "created": "1640995200",
                "activity": "1640995200"
            },
            {
                "name": "restored-session2",
                "windows": 1,
                "attached": true,
                "created": "1640995210",
                "activity": "1640995210"
            }
        ],
        "timestamp": "2024-01-01T00:00:00Z"
    }"#;
    
    fs::write(&snapshot_file, snapshot_content).unwrap();
    
    let mut cmd = Command::cargo_bin("cmux").unwrap();
    let output = cmd.arg("restore")
        .arg(snapshot_file.to_str().unwrap())
        .env("HOME", temp_dir.path())
        .output()
        .unwrap();
    
    // Command may succeed or fail depending on tmux availability
    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("Restoring 2 sessions from snapshot"));
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("tmux") || stderr.contains("Failed"));
    }
}

#[test]
fn test_restore_default_location() {
    let temp_dir = TempDir::new().unwrap();
    let default_snapshot = temp_dir.path().join(".cmux_snapshot.json");
    
    // Create default snapshot file
    let snapshot_content = r#"{
        "sessions": [
            {
                "name": "default-session",
                "windows": 1,
                "attached": false,
                "created": "1640995200",
                "activity": "1640995200"
            }
        ],
        "timestamp": "2024-01-01T00:00:00Z"
    }"#;
    
    fs::write(&default_snapshot, snapshot_content).unwrap();
    
    let mut cmd = Command::cargo_bin("cmux").unwrap();
    let output = cmd.arg("restore")
        .env("HOME", temp_dir.path())
        .output()
        .unwrap();
    
    // Should attempt to read from default location
    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("Restoring 1 sessions from snapshot"));
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("tmux") || stderr.contains("Failed"));
    }
}

#[test]
fn test_malformed_alias_file() {
    let temp_dir = TempDir::new().unwrap();
    let alias_file = temp_dir.path().join(".cmux_aliases.json");
    
    // Create malformed alias file
    fs::write(&alias_file, "{invalid json}").unwrap();
    
    let mut cmd = Command::cargo_bin("cmux").unwrap();
    cmd.arg("alias")
        .env("HOME", temp_dir.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("key must be a string").or(predicate::str::contains("Failed")));
}

#[test]
fn test_permission_denied_alias_file() {
    let temp_dir = TempDir::new().unwrap();
    let alias_file = temp_dir.path().join(".cmux_aliases.json");
    
    // Create a valid alias file first
    fs::write(&alias_file, r#"{"test": "session"}"#).unwrap();
    
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        
        // Make the file unreadable
        let mut perms = fs::metadata(&alias_file).unwrap().permissions();
        perms.set_mode(0o000);
        fs::set_permissions(&alias_file, perms).unwrap();
        
        let mut cmd = Command::cargo_bin("cmux").unwrap();
        cmd.arg("alias")
            .env("HOME", temp_dir.path())
            .assert()
            .failure()
            .stderr(predicate::str::contains("Permission denied").or(predicate::str::contains("Failed")));
        
        // Restore permissions for cleanup
        let mut restore_perms = fs::metadata(&alias_file).unwrap().permissions();
        restore_perms.set_mode(0o644);
        fs::set_permissions(&alias_file, restore_perms).unwrap();
    }
}

#[test]
fn test_alias_with_special_characters() {
    let temp_dir = TempDir::new().unwrap();
    
    // Test alias with special characters
    let mut cmd = Command::cargo_bin("cmux").unwrap();
    cmd.arg("alias")
        .arg("test-alias_with.special@chars")
        .arg("session-with-special_chars.too")
        .env("HOME", temp_dir.path())
        .assert()
        .success();
    
    // Verify the alias was created correctly
    let mut cmd = Command::cargo_bin("cmux").unwrap();
    cmd.arg("alias")
        .arg("test-alias_with.special@chars")
        .env("HOME", temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("test-alias_with.special@chars -> session-with-special_chars.too"));
}

#[test]
fn test_unicode_alias_names() {
    let temp_dir = TempDir::new().unwrap();
    
    // Test alias with Unicode characters
    let mut cmd = Command::cargo_bin("cmux").unwrap();
    cmd.arg("alias")
        .arg("🚀rocket")
        .arg("测试session")
        .env("HOME", temp_dir.path())
        .assert()
        .success();
    
    // Verify the Unicode alias works
    let mut cmd = Command::cargo_bin("cmux").unwrap();
    cmd.arg("alias")
        .arg("🚀rocket")
        .env("HOME", temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("🚀rocket -> 测试session"));
}

#[test]
fn test_very_long_alias_names() {
    let temp_dir = TempDir::new().unwrap();
    
    let long_alias = "a".repeat(1000);
    let long_session = "b".repeat(1000);
    
    let mut cmd = Command::cargo_bin("cmux").unwrap();
    cmd.arg("alias")
        .arg(&long_alias)
        .arg(&long_session)
        .env("HOME", temp_dir.path())
        .assert()
        .success();
    
    // Verify the long alias works
    let mut cmd = Command::cargo_bin("cmux").unwrap();
    cmd.arg("alias")
        .arg(&long_alias)
        .env("HOME", temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains(&long_session));
}

#[test]
fn test_concurrent_alias_operations() {
    let temp_dir = TempDir::new().unwrap();
    
    // Create multiple aliases concurrently (simulate concurrent access)
    let mut cmd1 = Command::cargo_bin("cmux").unwrap();
    let mut cmd2 = Command::cargo_bin("cmux").unwrap();
    
    let output1 = cmd1.arg("alias")
        .arg("alias1")
        .arg("session1")
        .env("HOME", temp_dir.path())
        .output()
        .unwrap();
    
    let output2 = cmd2.arg("alias")
        .arg("alias2")
        .arg("session2")
        .env("HOME", temp_dir.path())
        .output()
        .unwrap();
    
    // Both operations should complete successfully
    assert!(output1.status.success());
    assert!(output2.status.success());
}

#[test]
fn test_home_directory_fallback() {
    // Test behavior when HOME environment variable is not set
    let mut cmd = Command::cargo_bin("cmux").unwrap();
    cmd.arg("alias")
        .arg("test")
        .arg("session")
        .env_remove("HOME")
        .assert()
        .success(); // Should use "." as fallback
}