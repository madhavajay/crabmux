# Testing Documentation for Crabmux

This document describes the comprehensive test suite for the crabmux project.

## Test Structure

The test suite is organized into several categories:

### 1. Unit Tests (`src/main.rs`)

Located in the `#[cfg(test)]` module within `main.rs`, these test the core functionality:

#### Parsing Tests
- `test_parse_tmux_sessions()` - Tests parsing of valid tmux session output
- `test_parse_tmux_sessions_empty()` - Tests handling of empty output
- `test_parse_tmux_sessions_invalid_format()` - Tests handling of malformed session data

#### Mock Executor Tests
- `test_get_tmux_sessions_with_mock()` - Tests session retrieval with mocked tmux commands
- `test_get_tmux_sessions_no_server()` - Tests behavior when tmux server is not running

#### Application Logic Tests
- `test_app_navigation()` - Tests navigation through session list
- `test_app_navigation_empty()` - Tests navigation with no sessions
- `test_toggle_help()` - Tests help display toggle functionality

#### Data Structure Tests
- `test_tmux_session_struct()` - Tests TmuxSession struct creation and properties
- `test_session_snapshot_serialization()` - Tests JSON serialization/deserialization
- `test_input_result_variants()` - Tests input handling enum variants

### 2. Integration Tests (`tests/` directory)

#### CLI Command Tests (`tests/cli_tests.rs`)
- Tests all CLI commands and their aliases
- Validates argument parsing and validation
- Tests help and version commands
- Tests error handling for invalid commands

#### Error Handling Tests (`tests/error_handling_tests.rs`)
- Tests graceful handling of various error conditions
- Tests file permission errors
- Tests malformed JSON handling
- Tests behavior when tmux is not available
- Tests concurrent operations

#### File Operations Tests (`tests/file_operations_tests.rs`)
- Tests alias file creation and management
- Tests snapshot file operations
- Tests Unicode and special character handling
- Tests permission and file system error scenarios

#### Parsing Edge Cases (`tests/parsing_tests.rs`)
- Comprehensive tests for session parsing logic
- Tests various edge cases and malformed input
- Tests Unicode session names
- Tests extremely long session names and data
- Tests different line endings and whitespace handling

#### Integration Tests (`tests/integration_tests.rs`)
- End-to-end command testing
- Tests complete command workflows
- Tests file operations integration
- Tests error propagation through the entire stack

## Mock System

The test suite includes a comprehensive mocking system:

### TmuxExecutor Trait
A trait that abstracts tmux command execution to allow for testing without requiring tmux to be installed:

```rust
trait TmuxExecutor {
    fn execute_command(&self, args: &[&str]) -> Result<Output>;
}
```

### MockTmuxExecutor
A test implementation that allows simulating various tmux command responses:
- Success responses with custom stdout/stderr
- Error responses
- Simulation of "no server running" conditions

## Running Tests

### All Tests
```bash
cargo test
```

### Unit Tests Only
```bash
cargo test --bin cmux
```

### Specific Test Categories
```bash
# Integration tests
cargo test --test integration_tests

# CLI tests
cargo test --test cli_tests

# Error handling tests
cargo test --test error_handling_tests

# File operations tests
cargo test --test file_operations_tests

# Parsing tests
cargo test --test parsing_tests
```

### Specific Test Function
```bash
cargo test test_parse_tmux_sessions
```

## Test Dependencies

The test suite uses the following dependencies:

- `assert_cmd` - For testing CLI applications
- `predicates` - For flexible assertions on command output
- `tempfile` - For creating temporary files and directories in tests

## Test Coverage

The test suite covers:

1. **Command Execution**: All CLI commands and their variations
2. **Error Handling**: Various error conditions and edge cases
3. **File Operations**: Reading/writing alias and snapshot files
4. **Session Parsing**: All variations of tmux session output format
5. **UI Navigation**: Application state management and navigation
6. **Data Serialization**: JSON handling for snapshots and aliases
7. **Permission Handling**: File system permission scenarios
8. **Unicode Support**: International characters in session names
9. **Concurrent Operations**: Basic concurrency testing
10. **Environment Handling**: HOME directory and environment variable handling

## Test Philosophy

The tests are designed to:

1. **Work without tmux**: Most tests use mocking to avoid requiring tmux installation
2. **Test edge cases**: Comprehensive coverage of unusual inputs and error conditions
3. **Be isolated**: Each test is independent and doesn't affect others
4. **Be deterministic**: Tests produce consistent results across runs
5. **Be maintainable**: Clear naming and organization for easy maintenance

## Continuous Integration

The test suite is designed to work in CI environments where:
- tmux may not be installed
- Terminal interaction is not available
- File permissions may be restricted
- Network access may be limited

Tests that require tmux will gracefully handle its absence and still validate the error handling paths.

## Adding New Tests

When adding new functionality, ensure you:

1. Add unit tests for the core logic
2. Add integration tests for end-to-end workflows
3. Test error conditions and edge cases
4. Update mocks if new tmux commands are used
5. Add tests that work without tmux installed
6. Test Unicode and special character handling where applicable