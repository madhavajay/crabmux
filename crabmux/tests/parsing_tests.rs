// Tests for session parsing logic and edge cases
// These tests don't require tmux to be installed

#[cfg(test)]
mod parsing_tests {
    // We need to duplicate the parsing function here since it's not exported
    // In a real implementation, you'd want to make this function public for testing
    use serde::{Deserialize, Serialize};
    
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    struct TmuxSession {
        name: String,
        windows: usize,
        attached: bool,
        created: String,
        activity: String,
    }
    
    fn parse_tmux_sessions(output: &str) -> Vec<TmuxSession> {
        output
            .lines()
            .filter_map(|line| {
                let parts: Vec<&str> = line.split(':').collect();
                if parts.len() >= 5 {
                    Some(TmuxSession {
                        name: parts[0].to_string(),
                        windows: parts[1].parse().unwrap_or(0),
                        attached: parts[2] == "1",
                        created: parts[3].to_string(),
                        activity: parts[4].to_string(),
                    })
                } else {
                    None
                }
            })
            .collect()
    }

    #[test]
    fn test_parse_single_session() {
        let output = "main:3:1:1640995200:1640995200";
        let sessions = parse_tmux_sessions(output);
        
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].name, "main");
        assert_eq!(sessions[0].windows, 3);
        assert_eq!(sessions[0].attached, true);
        assert_eq!(sessions[0].created, "1640995200");
        assert_eq!(sessions[0].activity, "1640995200");
    }

    #[test]
    fn test_parse_multiple_sessions() {
        let output = "main:3:1:1640995200:1640995200\ndev:1:0:1640995210:1640995210\ntest:2:0:1640995220:1640995220";
        let sessions = parse_tmux_sessions(output);
        
        assert_eq!(sessions.len(), 3);
        
        assert_eq!(sessions[0].name, "main");
        assert_eq!(sessions[0].attached, true);
        
        assert_eq!(sessions[1].name, "dev");
        assert_eq!(sessions[1].attached, false);
        
        assert_eq!(sessions[2].name, "test");
        assert_eq!(sessions[2].attached, false);
    }

    #[test]
    fn test_parse_session_with_special_characters() {
        let output = "session-with-dashes:1:0:123:456\nsession_with_underscores:2:1:789:012";
        let sessions = parse_tmux_sessions(output);
        
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].name, "session-with-dashes");
        assert_eq!(sessions[1].name, "session_with_underscores");
    }

    #[test]
    fn test_parse_session_with_numeric_names() {
        let output = "123:1:0:456:789\n0:2:1:111:222";
        let sessions = parse_tmux_sessions(output);
        
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].name, "123");
        assert_eq!(sessions[1].name, "0");
    }

    #[test]
    fn test_parse_invalid_window_count() {
        let output = "main:invalid:1:123:456";
        let sessions = parse_tmux_sessions(output);
        
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].windows, 0); // Should default to 0 for invalid numbers
    }

    #[test]
    fn test_parse_attached_status_variations() {
        let output = "attached:1:1:123:456\ndetached:1:0:123:456\ninvalid:1:2:123:456";
        let sessions = parse_tmux_sessions(output);
        
        assert_eq!(sessions.len(), 3);
        assert_eq!(sessions[0].attached, true);   // "1" = attached
        assert_eq!(sessions[1].attached, false);  // "0" = detached
        assert_eq!(sessions[2].attached, false);  // anything else = detached
    }

    #[test]
    fn test_parse_empty_lines() {
        let output = "main:1:0:123:456\n\ndev:2:1:789:012\n";
        let sessions = parse_tmux_sessions(output);
        
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].name, "main");
        assert_eq!(sessions[1].name, "dev");
    }

    #[test]
    fn test_parse_incomplete_lines() {
        let output = "complete:1:0:123:456\nincomplete:data\nanother:complete:line:1:0:789:012";
        let sessions = parse_tmux_sessions(output);
        
        // Should only parse the complete lines
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].name, "complete");
        assert_eq!(sessions[1].name, "another");
    }

    #[test]
    fn test_parse_sessions_with_colons_in_names() {
        // This is an edge case - session names with colons would break parsing
        // This test documents the current behavior
        let output = "name:with:colons:1:0:123:456";
        let sessions = parse_tmux_sessions(output);
        
        // The parser will take the first part as the name
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].name, "name");
    }

    #[test]
    fn test_parse_sessions_with_extra_fields() {
        let output = "main:1:0:123:456:extra:field";
        let sessions = parse_tmux_sessions(output);
        
        // Should still parse correctly with extra fields
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].name, "main");
        assert_eq!(sessions[0].windows, 1);
        assert_eq!(sessions[0].attached, false);
        assert_eq!(sessions[0].created, "123");
        assert_eq!(sessions[0].activity, "456");
    }

    #[test]
    fn test_parse_very_large_window_count() {
        let output = "main:999999:1:123:456";
        let sessions = parse_tmux_sessions(output);
        
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].windows, 999999);
    }

    #[test]
    fn test_parse_negative_window_count() {
        let output = "main:-1:1:123:456";
        let sessions = parse_tmux_sessions(output);
        
        // Negative numbers should default to 0
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].windows, 0);
    }

    #[test]
    fn test_parse_unicode_session_names() {
        let output = "🚀session:1:0:123:456\n测试:2:1:789:012";
        let sessions = parse_tmux_sessions(output);
        
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].name, "🚀session");
        assert_eq!(sessions[1].name, "测试");
    }

    #[test]
    fn test_parse_whitespace_handling() {
        let output = " main:1:0:123:456 \n\t dev:2:1:789:012\t";
        let sessions = parse_tmux_sessions(output);
        
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].name, " main"); // Leading space preserved
        assert_eq!(sessions[1].name, "\t dev"); // Leading tab preserved
    }

    #[test]
    fn test_parse_windows_with_decimal() {
        let output = "main:1.5:1:123:456";
        let sessions = parse_tmux_sessions(output);
        
        // Decimal numbers should not parse correctly and default to 0
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].windows, 0);
    }

    #[test]
    fn test_parse_extremely_long_session_name() {
        let long_name = "a".repeat(1000);
        let output = format!("{}:1:0:123:456", long_name);
        let sessions = parse_tmux_sessions(&output);
        
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].name, long_name);
    }

    #[test]
    fn test_parse_mixed_line_endings() {
        let output = "unix:1:0:123:456\nwindows:2:1:789:012\r\nmixed:3:0:345:678\r\n";
        let sessions = parse_tmux_sessions(output);
        
        // Should handle different line endings gracefully
        assert_eq!(sessions.len(), 3);
        assert_eq!(sessions[0].name, "unix");
        assert_eq!(sessions[1].name, "windows");
        assert_eq!(sessions[2].name, "mixed");
    }
}