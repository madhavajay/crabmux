#![allow(clippy::uninlined_format_args)]

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyModifiers,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
    Frame, Terminal,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    io::{self, IsTerminal, Write},
    path::PathBuf,
    process::{Command, Output},
    time::Duration,
};
use sysinfo::System;

// Trait for executing tmux commands - allows for mocking in tests
trait TmuxExecutor {
    fn execute_command(&self, args: &[&str]) -> Result<Output>;
}

// Default implementation that executes real tmux commands
struct DefaultTmuxExecutor;

impl TmuxExecutor for DefaultTmuxExecutor {
    fn execute_command(&self, args: &[&str]) -> Result<Output> {
        Command::new("tmux")
            .args(args)
            .output()
            .context("Failed to execute tmux command")
    }
}

#[derive(Parser)]
#[command(name = "cmux")]
#[command(about = "A mobile-friendly tmux wrapper", long_about = None)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// List all tmux sessions
    #[command(visible_alias = "ls")]
    List,

    /// Attach to a tmux session
    #[command(visible_alias = "a")]
    Attach {
        /// Session name to attach to
        session: Option<String>,
    },

    /// Create a new tmux session
    #[command(visible_alias = "n")]
    New {
        /// Session name for the new session
        name: Option<String>,
    },

    /// Kill a tmux session
    #[command(visible_alias = "k")]
    Kill {
        /// Session name to kill
        session: Option<String>,
    },

    /// Rename a tmux session
    #[command(visible_alias = "r")]
    Rename {
        /// Current session name
        old_name: String,
        /// New session name
        new_name: String,
    },

    /// Restore sessions from snapshot
    Restore {
        /// Snapshot file path
        file: Option<PathBuf>,
    },

    /// Create or manage session aliases
    Alias {
        /// Alias name
        name: Option<String>,
        /// Session name to alias
        session: Option<String>,
    },

    /// Show live session overview
    Top,

    /// Show detailed session information
    Info {
        /// Session name
        session: Option<String>,
    },

    /// Kill all sessions with confirmation
    #[command(visible_alias = "ka")]
    KillAll,

    /// Show version information
    #[command(visible_alias = "v")]
    Version,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TmuxSession {
    name: String,
    windows: usize,
    attached: bool,
    created: String,
    activity: String,
    process_info: Option<ProcessInfo>,
    resource_info: Option<ResourceInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProcessInfo {
    pid: Option<u32>,
    command: String,
    user: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ResourceInfo {
    memory_mb: f64,
    cpu_percent: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SessionSnapshot {
    sessions: Vec<TmuxSession>,
    timestamp: String,
}

struct App {
    sessions: Vec<TmuxSession>,
    selected: usize,
    show_help: bool,
    #[allow(dead_code)]
    aliases: HashMap<String, String>,
    show_new_session_popup: bool,
    new_session_input: String,
    system: System,
}

impl App {
    fn new() -> Result<Self> {
        let sessions = get_tmux_sessions()?;
        let aliases = load_aliases()?;
        let mut system = System::new_all();
        system.refresh_all();
        Ok(App {
            sessions,
            selected: 0,
            show_help: false,
            aliases,
            show_new_session_popup: false,
            new_session_input: String::new(),
            system,
        })
    }

    /// Get the appropriate highlight style based on terminal capabilities
    fn get_highlight_style(&self) -> Style {
        // Check terminal environment for better compatibility
        let term = std::env::var("TERM").unwrap_or_else(|_| "unknown".to_string());
        let term_program = std::env::var("TERM_PROGRAM").unwrap_or_else(|_| "unknown".to_string());
        let colorterm = std::env::var("COLORTERM").unwrap_or_else(|_| "unknown".to_string());

        // For Warp terminal and other terminals that may have issues with background colors
        if term_program.contains("WarpTerminal") || term_program.contains("Warp") {
            // Warp-specific styling - use bright colors and modifiers without RGB for better compatibility
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
                .add_modifier(Modifier::UNDERLINED)
        } else if term.contains("screen") || term.contains("tmux") {
            // Screen/tmux compatibility
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
                .add_modifier(Modifier::REVERSED)
        } else if colorterm.contains("truecolor") || term.contains("256color") {
            // High color support terminals
            Style::default()
                .bg(Color::Rgb(0, 100, 200))
                .fg(Color::White)
                .add_modifier(Modifier::BOLD)
        } else {
            // Fallback for basic terminals
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
                .add_modifier(Modifier::REVERSED)
        }
    }

    /// Get selection symbol based on terminal capabilities
    fn get_selection_symbol(&self) -> &'static str {
        let term_program = std::env::var("TERM_PROGRAM").unwrap_or_else(|_| "unknown".to_string());
        let term = std::env::var("TERM").unwrap_or_else(|_| "unknown".to_string());

        // Use different symbols for different terminals for better visibility
        if term_program.contains("WarpTerminal") || term_program.contains("Warp") {
            "===> "
        } else if term_program.contains("iTerm") {
            "▶ "
        } else if term.contains("screen") || term.contains("tmux") {
            "-> "
        } else {
            "► "
        }
    }

    /// Get fallback selection indicators for terminals with limited symbol support
    fn get_selection_prefix(&self, selected: bool) -> String {
        if selected {
            ">".to_string()
        } else {
            " ".to_string()
        }
    }

    /// Debug function to show terminal detection information
    fn get_terminal_info(&self) -> String {
        let term = std::env::var("TERM").unwrap_or_else(|_| "unknown".to_string());
        let term_program = std::env::var("TERM_PROGRAM").unwrap_or_else(|_| "unknown".to_string());
        let colorterm = std::env::var("COLORTERM").unwrap_or_else(|_| "unknown".to_string());

        format!(
            "Terminal Detection:\n  TERM: {}\n  TERM_PROGRAM: {}\n  COLORTERM: {}\n  Selection Symbol: '{}'\n",
            term, term_program, colorterm, self.get_selection_symbol()
        )
    }

    fn refresh(&mut self) -> Result<()> {
        self.sessions = get_tmux_sessions_with_system(&mut self.system)?;
        Ok(())
    }

    fn next(&mut self) {
        if !self.sessions.is_empty() {
            self.selected = (self.selected + 1) % self.sessions.len();
        }
    }

    fn previous(&mut self) {
        if !self.sessions.is_empty() {
            self.selected = if self.selected == 0 {
                self.sessions.len() - 1
            } else {
                self.selected - 1
            };
        }
    }

    fn toggle_help(&mut self) {
        self.show_help = !self.show_help;
    }

    fn show_new_session_popup(&mut self) {
        self.show_new_session_popup = true;
        self.new_session_input.clear();
    }

    fn hide_new_session_popup(&mut self) {
        self.show_new_session_popup = false;
        self.new_session_input.clear();
    }

    fn handle_new_session_input(&mut self, c: char) {
        self.new_session_input.push(c);
    }

    fn backspace_new_session_input(&mut self) {
        self.new_session_input.pop();
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        None => run_tui()?,
        Some(Commands::List) => list_sessions()?,
        Some(Commands::Attach { session }) => attach_session(session)?,
        Some(Commands::New { name }) => new_session(name)?,
        Some(Commands::Kill { session }) => kill_session(session)?,
        Some(Commands::Rename { old_name, new_name }) => rename_session(&old_name, &new_name)?,
        Some(Commands::Restore { file }) => restore_sessions(file)?,
        Some(Commands::Alias { name, session }) => manage_alias(name, session)?,
        Some(Commands::Top) => run_top_mode()?,
        Some(Commands::Info { session }) => show_session_info(session)?,
        Some(Commands::KillAll) => kill_all_sessions()?,
        Some(Commands::Version) => {
            println!("cmux {}", env!("CARGO_PKG_VERSION"));
            println!("A mobile-friendly tmux session manager");
        }
    }

    Ok(())
}

fn get_tmux_sessions() -> Result<Vec<TmuxSession>> {
    let mut system = System::new_all();
    system.refresh_all();
    get_tmux_sessions_with_system(&mut system)
}

fn get_tmux_sessions_with_system(system: &mut System) -> Result<Vec<TmuxSession>> {
    get_tmux_sessions_with_executor_and_system(&DefaultTmuxExecutor, system)
}

#[allow(dead_code)]
fn get_tmux_sessions_with_executor(executor: &dyn TmuxExecutor) -> Result<Vec<TmuxSession>> {
    let mut system = System::new_all();
    system.refresh_all();
    get_tmux_sessions_with_executor_and_system(executor, &mut system)
}

fn get_tmux_sessions_with_executor_and_system(
    executor: &dyn TmuxExecutor,
    system: &mut System,
) -> Result<Vec<TmuxSession>> {
    let output = executor.execute_command(&["list-sessions", "-F", "#{session_name}:#{session_windows}:#{session_attached}:#{session_created}:#{session_activity}"])?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Handle various tmux error messages for no server
        if stderr.contains("no server running") || 
           stderr.contains("no sessions") || 
           stderr.contains("no current client") ||
           stderr.contains("can't find session") ||
           stderr.contains("server not found") {
            return Ok(Vec::new());
        }
        return Err(anyhow::anyhow!("tmux command failed: {}", stderr.trim()));
    }

    let mut sessions = parse_tmux_sessions(&String::from_utf8_lossy(&output.stdout));

    // Enrich sessions with process and resource information
    for session in &mut sessions {
        enrich_session_info(session, executor, system);
    }

    Ok(sessions)
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
                    process_info: None,
                    resource_info: None,
                })
            } else {
                None
            }
        })
        .collect()
}

fn enrich_session_info(
    session: &mut TmuxSession,
    executor: &dyn TmuxExecutor,
    system: &mut System,
) {
    // Get tmux server PID
    if let Ok(output) =
        executor.execute_command(&["list-sessions", "-t", &session.name, "-F", "#{session_id}"])
    {
        if output.status.success() {
            let _session_id = String::from_utf8_lossy(&output.stdout).trim().to_string();

            // Try to find the tmux process for this session
            system.refresh_processes();
            let mut total_memory = 0.0;
            let mut total_cpu = 0.0;
            let mut process_count = 0;

            for (pid, process) in system.processes() {
                let cmd = process.cmd();
                if cmd
                    .iter()
                    .any(|arg| arg.contains("tmux") || arg.contains(&session.name))
                {
                    total_memory += process.memory() as f64 / 1024.0 / 1024.0; // Convert to MB
                    total_cpu += process.cpu_usage();
                    process_count += 1;

                    if session.process_info.is_none() {
                        session.process_info = Some(ProcessInfo {
                            pid: Some(pid.as_u32()),
                            command: cmd.join(" "),
                            user: process
                                .user_id()
                                .map(|u| u.to_string())
                                .unwrap_or_else(|| "unknown".to_string()),
                        });
                    }
                }
            }

            if process_count > 0 {
                session.resource_info = Some(ResourceInfo {
                    memory_mb: total_memory,
                    cpu_percent: total_cpu,
                });
            }
        }
    }

    // Fallback process info if not found
    if session.process_info.is_none() {
        session.process_info = Some(ProcessInfo {
            pid: None,
            command: "tmux".to_string(),
            user: std::env::var("USER").unwrap_or_else(|_| "unknown".to_string()),
        });
    }

    // Fallback resource info if not found
    if session.resource_info.is_none() {
        session.resource_info = Some(ResourceInfo {
            memory_mb: 0.0,
            cpu_percent: 0.0,
        });
    }
}

fn list_sessions() -> Result<()> {
    let sessions = get_tmux_sessions()?;

    if sessions.is_empty() {
        println!("No tmux sessions found.");
        return Ok(());
    }

    println!("Active tmux sessions:");
    println!("{:<20} {:<10} {:<10}", "Name", "Windows", "Status");
    println!("{}", "-".repeat(40));

    for session in sessions {
        let status = if session.attached {
            "attached"
        } else {
            "detached"
        };
        println!(
            "{:<20} {:<10} {:<10}",
            session.name, session.windows, status
        );
    }

    Ok(())
}

fn attach_session(session_name: Option<String>) -> Result<()> {
    let sessions = get_tmux_sessions()?;

    let target_session = match session_name {
        Some(name) => name,
        None => {
            if sessions.is_empty() {
                return Err(anyhow::anyhow!("No tmux sessions found"));
            }
            sessions[0].name.clone()
        }
    };

    let status = Command::new("tmux")
        .args(["attach-session", "-t", &target_session])
        .status()
        .context("Failed to execute tmux attach command")?;

    if !status.success() {
        return Err(anyhow::anyhow!(
            "Failed to attach to session '{}'. Session may not exist.",
            target_session
        ));
    }

    Ok(())
}

fn new_session(name: Option<String>) -> Result<()> {
    let mut cmd = Command::new("tmux");
    cmd.arg("new-session");

    if let Some(session_name) = name {
        cmd.args(["-s", &session_name]);
    }

    let status = cmd
        .status()
        .context("Failed to execute tmux new-session command")?;

    if !status.success() {
        return Err(anyhow::anyhow!(
            "Failed to create new tmux session. Session name may already exist."
        ));
    }

    Ok(())
}

fn kill_session(session_name: Option<String>) -> Result<()> {
    let sessions = get_tmux_sessions()?;

    let target_session = match session_name {
        Some(name) => name,
        None => {
            if sessions.is_empty() {
                return Err(anyhow::anyhow!("No tmux sessions found"));
            }
            // In interactive mode, we'd select, but in CLI mode, refuse to kill without name
            return Err(anyhow::anyhow!("Please specify a session name to kill"));
        }
    };

    let status = Command::new("tmux")
        .args(["kill-session", "-t", &target_session])
        .status()
        .context("Failed to execute tmux kill-session command")?;

    if !status.success() {
        return Err(anyhow::anyhow!(
            "Failed to kill session '{}'. Session may not exist.",
            target_session
        ));
    }

    println!("Killed session: {}", target_session);
    Ok(())
}

fn rename_session(old_name: &str, new_name: &str) -> Result<()> {
    let status = Command::new("tmux")
        .args(["rename-session", "-t", old_name, new_name])
        .status()
        .context("Failed to execute tmux rename command")?;

    if !status.success() {
        return Err(anyhow::anyhow!(
            "Failed to rename session '{}' to '{}'. Session may not exist.",
            old_name,
            new_name
        ));
    }

    println!("Renamed session '{}' to '{}'", old_name, new_name);
    Ok(())
}

fn restore_sessions(file: Option<PathBuf>) -> Result<()> {
    let snapshot_path = file.unwrap_or_else(|| {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home).join(".cmux_snapshot.json")
    });

    let content = fs::read_to_string(&snapshot_path).context("Failed to read snapshot file")?;

    let snapshot: SessionSnapshot =
        serde_json::from_str(&content).context("Failed to parse snapshot file")?;

    println!(
        "Restoring {} sessions from snapshot...",
        snapshot.sessions.len()
    );

    for session in snapshot.sessions {
        if get_tmux_sessions()?.iter().any(|s| s.name == session.name) {
            println!("Session '{}' already exists, skipping...", session.name);
            continue;
        }

        Command::new("tmux")
            .args(["new-session", "-d", "-s", &session.name])
            .status()
            .context("Failed to create session")?;

        println!("Restored session: {}", session.name);
    }

    Ok(())
}

fn save_snapshot() -> Result<PathBuf> {
    let sessions = get_tmux_sessions()?;
    let snapshot = SessionSnapshot {
        sessions,
        timestamp: chrono::Local::now().to_rfc3339(),
    };

    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let snapshot_path = PathBuf::from(home).join(".cmux_snapshot.json");

    let json = serde_json::to_string_pretty(&snapshot)?;
    fs::write(&snapshot_path, json)?;

    Ok(snapshot_path)
}

fn load_aliases() -> Result<HashMap<String, String>> {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let alias_path = PathBuf::from(home).join(".cmux_aliases.json");

    if !alias_path.exists() {
        return Ok(HashMap::new());
    }

    let content = fs::read_to_string(&alias_path)?;
    let aliases: HashMap<String, String> = serde_json::from_str(&content)?;
    Ok(aliases)
}

fn save_aliases(aliases: &HashMap<String, String>) -> Result<()> {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let alias_path = PathBuf::from(home).join(".cmux_aliases.json");

    let json = serde_json::to_string_pretty(aliases)?;
    fs::write(&alias_path, json)?;
    Ok(())
}

fn manage_alias(name: Option<String>, session: Option<String>) -> Result<()> {
    let mut aliases = load_aliases()?;

    match (name, session) {
        (Some(alias_name), Some(session_name)) => {
            aliases.insert(alias_name.clone(), session_name.clone());
            save_aliases(&aliases)?;
            println!(
                "Created alias '{}' for session '{}'",
                alias_name, session_name
            );
        }
        (Some(alias_name), None) => {
            if let Some(session_name) = aliases.get(&alias_name) {
                println!("{} -> {}", alias_name, session_name);
            } else {
                println!("Alias '{}' not found", alias_name);
            }
        }
        (None, None) => {
            if aliases.is_empty() {
                println!("No aliases defined");
            } else {
                println!("Current aliases:");
                for (alias, session) in aliases {
                    println!("  {} -> {}", alias, session);
                }
            }
        }
        _ => {
            return Err(anyhow::anyhow!("Invalid alias command"));
        }
    }

    Ok(())
}

fn show_session_info(session_name: Option<String>) -> Result<()> {
    let sessions = get_tmux_sessions()?;

    let target_session = match session_name {
        Some(name) => sessions
            .into_iter()
            .find(|s| s.name == name)
            .ok_or_else(|| anyhow::anyhow!("Session '{}' not found", name))?,
        None => {
            if sessions.is_empty() {
                return Err(anyhow::anyhow!("No tmux sessions found"));
            }
            sessions.into_iter().next().unwrap()
        }
    };

    println!("Session Information:");
    println!("  Name: {}", target_session.name);
    println!("  Windows: {}", target_session.windows);
    println!(
        "  Status: {}",
        if target_session.attached {
            "attached"
        } else {
            "detached"
        }
    );
    println!("  Created: {}", target_session.created);
    println!("  Last Activity: {}", target_session.activity);

    // Get window details
    let output = Command::new("tmux")
        .args([
            "list-windows",
            "-t",
            &target_session.name,
            "-F",
            "#{window_index}: #{window_name} (#{window_panes} panes)",
        ])
        .output()?;

    if output.status.success() {
        println!("\nWindows:");
        let windows = String::from_utf8_lossy(&output.stdout);
        for window in windows.lines() {
            println!("  {}", window);
        }
    }

    Ok(())
}

fn kill_all_sessions() -> Result<()> {
    let sessions = get_tmux_sessions()?;

    if sessions.is_empty() {
        println!("No tmux sessions to kill.");
        return Ok(());
    }

    println!("This will kill {} sessions:", sessions.len());
    for session in &sessions {
        println!("  - {}", session.name);
    }

    print!("\nAre you sure? (y/N): ");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    if input.trim().to_lowercase() != "y" {
        println!("Cancelled.");
        return Ok(());
    }

    for session in sessions {
        Command::new("tmux")
            .args(["kill-session", "-t", &session.name])
            .status()?;
        println!("Killed: {}", session.name);
    }

    println!("All sessions killed.");
    Ok(())
}

fn run_top_mode() -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new()?;
    let mut last_refresh = std::time::Instant::now();

    loop {
        // Auto-refresh every 2 seconds
        if last_refresh.elapsed() > Duration::from_secs(2) {
            app.refresh()?;
            last_refresh = std::time::Instant::now();
        }

        terminal.draw(|f| draw_top_ui(f, &app))?;

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => break,
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => break,
                    KeyCode::Char('r') => {
                        app.refresh()?;
                        last_refresh = std::time::Instant::now();
                    }
                    _ => {}
                }
            }
        }
    }

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    Ok(())
}

fn draw_top_ui(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(3),
        ])
        .split(f.size());

    // Header with system info
    let total_sessions = app.sessions.len();
    let active_sessions = app.sessions.iter().filter(|s| s.attached).count();
    let header_text = format!(
        "crabmux - Live Overview | {} total, {} active | {}",
        total_sessions,
        active_sessions,
        chrono::Local::now().format("%H:%M:%S")
    );
    let header = Paragraph::new(header_text)
        .style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(header, chunks[0]);

    // Session list with detailed info
    let sessions: Vec<ListItem> = app
        .sessions
        .iter()
        .map(|s| {
            let status = if s.attached { "●" } else { "○" };
            let (memory_info, cpu_info) = if let Some(ref resource) = s.resource_info {
                (
                    format!("{:.1}MB", resource.memory_mb),
                    format!("{:.1}%", resource.cpu_percent),
                )
            } else {
                ("N/A".to_string(), "N/A".to_string())
            };

            let user = if let Some(ref process) = s.process_info {
                &process.user
            } else {
                "unknown"
            };

            let content = Line::from(vec![
                Span::styled(
                    "▶ ",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    status,
                    Style::default().fg(if s.attached { Color::Green } else { Color::Red }),
                ),
                Span::raw(" "),
                Span::styled(
                    format!("{:<12}", s.name),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" "),
                Span::styled(
                    format!("{}W", s.windows),
                    Style::default().fg(Color::Yellow),
                ),
                Span::raw(" "),
                Span::styled(
                    format!("{:<8}", memory_info),
                    Style::default().fg(Color::Cyan),
                ),
                Span::raw(" "),
                Span::styled(
                    format!("{:<6}", cpu_info),
                    Style::default().fg(Color::Magenta),
                ),
                Span::raw(" "),
                Span::styled(format!("{:<8}", user), Style::default().fg(Color::Gray)),
            ]);
            ListItem::new(content)
        })
        .collect();

    let title = " │ Name             │Win │  Memory │   CPU │ User    ";
    // Helper function to get terminal-appropriate styles
    fn get_top_ui_highlight_style() -> Style {
        let term = std::env::var("TERM").unwrap_or_else(|_| "unknown".to_string());
        let term_program = std::env::var("TERM_PROGRAM").unwrap_or_else(|_| "unknown".to_string());
        let colorterm = std::env::var("COLORTERM").unwrap_or_else(|_| "unknown".to_string());

        // For Warp terminal and other terminals that may have issues with background colors
        if term_program.contains("WarpTerminal") || term_program.contains("Warp") {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
                .add_modifier(Modifier::UNDERLINED)
        } else if term.contains("screen") || term.contains("tmux") {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
                .add_modifier(Modifier::REVERSED)
        } else if colorterm.contains("truecolor") || term.contains("256color") {
            Style::default()
                .bg(Color::Rgb(0, 100, 200))
                .fg(Color::White)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
                .add_modifier(Modifier::REVERSED)
        }
    }

    fn get_top_ui_selection_symbol() -> &'static str {
        let term_program = std::env::var("TERM_PROGRAM").unwrap_or_else(|_| "unknown".to_string());
        let term = std::env::var("TERM").unwrap_or_else(|_| "unknown".to_string());

        if term_program.contains("WarpTerminal") || term_program.contains("Warp") {
            "===> "
        } else if term_program.contains("iTerm") {
            "▶ "
        } else if term.contains("screen") || term.contains("tmux") {
            "-> "
        } else {
            "► "
        }
    }

    let sessions_list = List::new(sessions)
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(get_top_ui_highlight_style())
        .highlight_symbol(get_top_ui_selection_symbol());

    f.render_widget(sessions_list, chunks[1]);

    // Help
    let help_text = "Press 'q' to quit, 'r' to refresh, Ctrl+C to exit";
    let help = Paragraph::new(help_text)
        .style(Style::default().fg(Color::Gray))
        .alignment(Alignment::Left)
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(help, chunks[2]);
}

fn run_tui() -> Result<()> {
    // Check if we're in a proper terminal
    if !std::io::stdout().is_terminal() {
        return Err(anyhow::anyhow!("cmux requires an interactive terminal. Try running a specific command like 'cmux ls' or 'cmux --help'"));
    }

    enable_raw_mode()
        .context("Failed to enable raw mode. Make sure you're running in a supported terminal.")?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new()?;
    let mut list_state = ListState::default();
    list_state.select(Some(0));

    loop {
        terminal.draw(|f| draw_ui(f, &mut app, &mut list_state))?;

        if let Event::Key(key) = event::read()? {
            match handle_input(&mut app, key)? {
                InputResult::Continue => {}
                InputResult::Quit => break,
                InputResult::AttachSession(name) => {
                    // Clean up terminal before attaching
                    disable_raw_mode()?;
                    execute!(
                        terminal.backend_mut(),
                        LeaveAlternateScreen,
                        DisableMouseCapture
                    )?;
                    terminal.show_cursor()?;

                    // Attach to session
                    attach_session(Some(name))?;

                    // Re-enter TUI mode after detaching
                    enable_raw_mode()?;
                    let mut new_stdout = io::stdout();
                    execute!(new_stdout, EnterAlternateScreen, EnableMouseCapture)?;

                    // Clear the screen and refresh the terminal
                    let backend = CrosstermBackend::new(new_stdout);
                    terminal = Terminal::new(backend)?;
                    terminal.clear()?;
                    app.refresh()?;
                }
            }
        }
    }

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    Ok(())
}

enum InputResult {
    Continue,
    Quit,
    AttachSession(String),
}

fn handle_input(app: &mut App, key: KeyEvent) -> Result<InputResult> {
    // Handle Ctrl+C for exit
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return Ok(InputResult::Quit);
    }

    // Handle popup input if showing new session popup
    if app.show_new_session_popup {
        match key.code {
            KeyCode::Enter => {
                let session_name = if app.new_session_input.trim().is_empty() {
                    format!("session-{}", chrono::Local::now().format("%H%M%S"))
                } else {
                    app.new_session_input.clone()
                };
                new_session(Some(session_name))?;
                app.hide_new_session_popup();
                app.refresh()?;
            }
            KeyCode::Esc => {
                app.hide_new_session_popup();
            }
            KeyCode::Backspace => {
                app.backspace_new_session_input();
            }
            KeyCode::Char(c) => {
                app.handle_new_session_input(c);
            }
            _ => {}
        }
        return Ok(InputResult::Continue);
    }

    // Normal input handling
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => return Ok(InputResult::Quit),
        KeyCode::Char('?') | KeyCode::Char('h') => app.toggle_help(),
        KeyCode::Down | KeyCode::Char('j') => app.next(),
        KeyCode::Up | KeyCode::Char('k') => app.previous(),
        KeyCode::Enter => {
            if !app.sessions.is_empty() {
                let session_name = app.sessions[app.selected].name.clone();
                return Ok(InputResult::AttachSession(session_name));
            }
        }
        KeyCode::Char('n') => {
            app.show_new_session_popup();
        }
        KeyCode::Char('K') => {
            // Kill selected session
            if !app.sessions.is_empty() {
                let session_name = app.sessions[app.selected].name.clone();
                kill_session(Some(session_name))?;
                app.refresh()?;
                if app.selected >= app.sessions.len() && app.selected > 0 {
                    app.selected = app.sessions.len() - 1;
                }
            }
        }
        KeyCode::Char('r') => {
            // Refresh session list
            app.refresh()?;
        }
        KeyCode::Char('s') => {
            // Save snapshot
            let path = save_snapshot()?;
            println!("Snapshot saved to: {:?}", path);
        }
        KeyCode::Char('d') => {
            // Debug terminal info
            eprintln!("{}", app.get_terminal_info());
        }
        _ => {}
    }
    Ok(InputResult::Continue)
}

fn draw_ui(f: &mut Frame, app: &mut App, list_state: &mut ListState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(5),
        ])
        .split(f.size());

    // Header
    let header = Paragraph::new("crabmux - Mobile-Friendly tmux Manager")
        .style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(header, chunks[0]);

    // Session list
    if app.sessions.is_empty() {
        let empty_msg =
            Paragraph::new("No tmux sessions found.\nPress 'n' to create a new session.")
                .style(Style::default().fg(Color::Gray))
                .alignment(Alignment::Center)
                .block(Block::default().borders(Borders::ALL).title("Sessions"));
        f.render_widget(empty_msg, chunks[1]);
    } else {
        let sessions: Vec<ListItem> = app
            .sessions
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let status = if s.attached { "●" } else { "○" };

                // Get resource info
                let (memory_info, cpu_info) = if let Some(ref resource) = s.resource_info {
                    (
                        format!("{:.1}MB", resource.memory_mb),
                        format!("{:.1}%", resource.cpu_percent),
                    )
                } else {
                    ("N/A".to_string(), "N/A".to_string())
                };

                // Get user info
                let user = if let Some(ref process) = s.process_info {
                    &process.user
                } else {
                    "unknown"
                };

                // Add selection indicator prefix for better visibility
                let selection_prefix = app.get_selection_prefix(i == app.selected);

                let content = Line::from(vec![
                    Span::styled(
                        format!("{:<1}", selection_prefix),
                        Style::default()
                            .fg(if i == app.selected {
                                Color::Yellow
                            } else {
                                Color::DarkGray
                            })
                            .add_modifier(if i == app.selected {
                                Modifier::BOLD
                            } else {
                                Modifier::empty()
                            }),
                    ),
                    Span::styled(
                        format!("{:<1}", status),
                        Style::default().fg(if s.attached { Color::Green } else { Color::Red }),
                    ),
                    Span::raw(" "),
                    Span::styled(
                        format!("{:<15}", s.name),
                        Style::default()
                            .fg(if i == app.selected {
                                Color::Yellow
                            } else {
                                Color::White
                            })
                            .add_modifier(if i == app.selected {
                                Modifier::BOLD | Modifier::UNDERLINED
                            } else {
                                Modifier::BOLD
                            }),
                    ),
                    Span::styled(
                        format!("{:>3}W", s.windows),
                        Style::default().fg(if i == app.selected {
                            Color::Yellow
                        } else {
                            Color::White
                        }),
                    ),
                    Span::raw(" "),
                    Span::styled(
                        format!("{:>8}", memory_info),
                        Style::default().fg(if i == app.selected {
                            Color::Yellow
                        } else {
                            Color::Cyan
                        }),
                    ),
                    Span::raw(" "),
                    Span::styled(
                        format!("{:>6}", cpu_info),
                        Style::default().fg(if i == app.selected {
                            Color::Yellow
                        } else {
                            Color::Magenta
                        }),
                    ),
                    Span::raw(" "),
                    Span::styled(
                        format!("{:<8}", user),
                        Style::default().fg(if i == app.selected {
                            Color::Yellow
                        } else {
                            Color::Gray
                        }),
                    ),
                ]);

                let mut item = ListItem::new(content);
                if i == app.selected {
                    // Use terminal-aware highlighting
                    item = item.style(app.get_highlight_style());
                }
                item
            })
            .collect();

        let title = "Sessions │ Name        │ Win │ Memory │ CPU   │ User    ";
        let sessions_list = List::new(sessions)
            .block(Block::default().borders(Borders::ALL).title(title))
            .highlight_style(app.get_highlight_style())
            .highlight_symbol(app.get_selection_symbol());

        list_state.select(Some(app.selected));
        f.render_stateful_widget(sessions_list, chunks[1], list_state);
    }

    // Controls/Help
    let help_text = if app.show_help {
        vec![
            "↑/↓/j/k: Navigate    Enter: Attach    n: New session",
            "K: Kill session      r: Refresh       s: Save snapshot",
            "d: Debug terminal    q/Esc/Ctrl+C: Quit  ?: Toggle help",
        ]
    } else {
        vec!["Navigate: ↑/↓  Attach: Enter  New: n  Kill: K  Debug: d  Quit: q/Ctrl+C  Help: ?"]
    };

    let help = Paragraph::new(help_text.join("\n"))
        .style(Style::default().fg(Color::Gray))
        .alignment(Alignment::Left)
        .wrap(Wrap { trim: true })
        .block(Block::default().borders(Borders::ALL).title("Controls"));
    f.render_widget(help, chunks[2]);

    // Render popup if showing
    if app.show_new_session_popup {
        draw_new_session_popup(f, app);
    }
}

fn draw_new_session_popup(f: &mut Frame, app: &App) {
    let popup_area = centered_rect(50, 20, f.size());

    // Clear the area
    f.render_widget(Clear, popup_area);

    let popup_block = Block::default()
        .title("New Session")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));

    let popup_chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(popup_area);

    f.render_widget(popup_block, popup_area);

    let input_text = Paragraph::new("Enter session name (or press Enter for default):")
        .style(Style::default().fg(Color::White));
    f.render_widget(input_text, popup_chunks[0]);

    let input_field = Paragraph::new(app.new_session_input.as_str())
        .style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(input_field, popup_chunks[1]);

    let default_name = format!("Default: session-{}", chrono::Local::now().format("%H%M%S"));
    let default_text = Paragraph::new(default_name).style(Style::default().fg(Color::Gray));
    f.render_widget(default_text, popup_chunks[2]);

    let help_text = Paragraph::new("Enter: Create  Esc: Cancel")
        .style(Style::default().fg(Color::Gray))
        .alignment(Alignment::Center);
    f.render_widget(help_text, popup_chunks[3]);
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt;
    use std::process::{ExitStatus, Output};

    // Mock tmux executor for testing
    struct MockTmuxExecutor {
        responses: HashMap<String, Result<Output>>,
    }

    impl MockTmuxExecutor {
        fn new() -> Self {
            MockTmuxExecutor {
                responses: HashMap::new(),
            }
        }

        fn add_response(&mut self, args: Vec<&str>, stdout: &str, stderr: &str, success: bool) {
            let key = args.join(" ");
            let output = Output {
                stdout: stdout.as_bytes().to_vec(),
                stderr: stderr.as_bytes().to_vec(),
                status: ExitStatus::from_raw(if success { 0 } else { 1 }),
            };
            self.responses.insert(key, Ok(output));
        }

        fn add_error_response(&mut self, args: Vec<&str>) {
            let key = args.join(" ");
            self.responses
                .insert(key, Err(anyhow::anyhow!("Command failed")));
        }
    }

    impl TmuxExecutor for MockTmuxExecutor {
        fn execute_command(&self, args: &[&str]) -> Result<Output> {
            let key = args.join(" ");
            match self.responses.get(&key) {
                Some(Ok(output)) => Ok(output.clone()),
                Some(Err(e)) => Err(anyhow::anyhow!("{}", e)),
                None => Err(anyhow::anyhow!("No mock response for: {}", key)),
            }
        }
    }

    #[test]
    fn test_parse_tmux_sessions() {
        let output = "main:3:1:1234567890:1234567890\ndev:1:0:1234567891:1234567891\ntest:2:0:1234567892:1234567892";
        let sessions = parse_tmux_sessions(output);

        assert_eq!(sessions.len(), 3);

        assert_eq!(sessions[0].name, "main");
        assert_eq!(sessions[0].windows, 3);
        assert_eq!(sessions[0].attached, true);

        assert_eq!(sessions[1].name, "dev");
        assert_eq!(sessions[1].windows, 1);
        assert_eq!(sessions[1].attached, false);

        assert_eq!(sessions[2].name, "test");
        assert_eq!(sessions[2].windows, 2);
        assert_eq!(sessions[2].attached, false);
    }

    #[test]
    fn test_parse_tmux_sessions_empty() {
        let output = "";
        let sessions = parse_tmux_sessions(output);
        assert_eq!(sessions.len(), 0);
    }

    #[test]
    fn test_parse_tmux_sessions_invalid_format() {
        let output = "invalid:format\nmain:3:1:1234567890:1234567890\nincomplete:data";
        let sessions = parse_tmux_sessions(output);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].name, "main");
    }

    #[test]
    fn test_get_tmux_sessions_with_mock() {
        let mut executor = MockTmuxExecutor::new();
        executor.add_response(
            vec!["list-sessions", "-F", "#{session_name}:#{session_windows}:#{session_attached}:#{session_created}:#{session_activity}"],
            "main:3:1:1234567890:1234567890\ndev:1:0:1234567891:1234567891",
            "",
            true,
        );
        // Add mock response for session info enrichment
        executor.add_response(
            vec!["list-sessions", "-t", "main", "-F", "#{session_id}"],
            "$0",
            "",
            true,
        );
        executor.add_response(
            vec!["list-sessions", "-t", "dev", "-F", "#{session_id}"],
            "$1",
            "",
            true,
        );

        let sessions = get_tmux_sessions_with_executor(&executor).unwrap();
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].name, "main");
        assert_eq!(sessions[1].name, "dev");
        assert!(sessions[0].process_info.is_some());
        assert!(sessions[0].resource_info.is_some());
    }

    #[test]
    fn test_get_tmux_sessions_no_server() {
        let mut executor = MockTmuxExecutor::new();
        executor.add_response(
            vec!["list-sessions", "-F", "#{session_name}:#{session_windows}:#{session_attached}:#{session_created}:#{session_activity}"],
            "",
            "no server running on /tmp/tmux-1000/default",
            false,
        );

        let sessions = get_tmux_sessions_with_executor(&executor).unwrap();
        assert_eq!(sessions.len(), 0);
    }

    #[test]
    fn test_tmux_session_struct() {
        let session = TmuxSession {
            name: "test".to_string(),
            windows: 2,
            attached: true,
            created: "1234567890".to_string(),
            activity: "1234567890".to_string(),
            process_info: None,
            resource_info: None,
        };

        assert_eq!(session.name, "test");
        assert_eq!(session.windows, 2);
        assert_eq!(session.attached, true);
    }

    #[test]
    fn test_app_navigation() {
        let mut app = App {
            sessions: vec![
                TmuxSession {
                    name: "session1".to_string(),
                    windows: 1,
                    attached: false,
                    created: "123".to_string(),
                    activity: "123".to_string(),
                    process_info: None,
                    resource_info: None,
                },
                TmuxSession {
                    name: "session2".to_string(),
                    windows: 2,
                    attached: false,
                    created: "124".to_string(),
                    activity: "124".to_string(),
                    process_info: None,
                    resource_info: None,
                },
                TmuxSession {
                    name: "session3".to_string(),
                    windows: 3,
                    attached: false,
                    created: "125".to_string(),
                    activity: "125".to_string(),
                    process_info: None,
                    resource_info: None,
                },
            ],
            selected: 0,
            show_help: false,
            aliases: HashMap::new(),
            show_new_session_popup: false,
            new_session_input: String::new(),
            system: System::new_all(),
        };

        // Test next navigation
        assert_eq!(app.selected, 0);
        app.next();
        assert_eq!(app.selected, 1);
        app.next();
        assert_eq!(app.selected, 2);
        app.next();
        assert_eq!(app.selected, 0); // Should wrap around

        // Test previous navigation
        app.previous();
        assert_eq!(app.selected, 2); // Should wrap around
        app.previous();
        assert_eq!(app.selected, 1);
        app.previous();
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn test_app_navigation_empty() {
        let mut app = App {
            sessions: vec![],
            selected: 0,
            show_help: false,
            aliases: HashMap::new(),
            show_new_session_popup: false,
            new_session_input: String::new(),
            system: System::new_all(),
        };

        // Navigation should not crash with empty sessions
        app.next();
        assert_eq!(app.selected, 0);
        app.previous();
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn test_toggle_help() {
        let mut app = App {
            sessions: vec![],
            selected: 0,
            show_help: false,
            aliases: HashMap::new(),
            show_new_session_popup: false,
            new_session_input: String::new(),
            system: System::new_all(),
        };

        assert_eq!(app.show_help, false);
        app.toggle_help();
        assert_eq!(app.show_help, true);
        app.toggle_help();
        assert_eq!(app.show_help, false);
    }

    #[test]
    fn test_session_snapshot_serialization() {
        let sessions = vec![TmuxSession {
            name: "main".to_string(),
            windows: 3,
            attached: true,
            created: "123".to_string(),
            activity: "456".to_string(),
            process_info: None,
            resource_info: None,
        }];

        let snapshot = SessionSnapshot {
            sessions: sessions.clone(),
            timestamp: "2024-01-01T00:00:00".to_string(),
        };

        // Test serialization
        let json = serde_json::to_string(&snapshot).unwrap();
        assert!(json.contains("\"name\":\"main\""));
        assert!(json.contains("\"windows\":3"));
        assert!(json.contains("\"attached\":true"));

        // Test deserialization
        let deserialized: SessionSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.sessions.len(), 1);
        assert_eq!(deserialized.sessions[0].name, "main");
        assert_eq!(deserialized.timestamp, "2024-01-01T00:00:00");
    }

    #[test]
    fn test_input_result_variants() {
        // Test that InputResult enum variants work correctly
        let result1 = InputResult::Continue;
        let result2 = InputResult::Quit;
        let result3 = InputResult::AttachSession("test".to_string());

        match result1 {
            InputResult::Continue => {}
            _ => panic!("Expected Continue"),
        }

        match result2 {
            InputResult::Quit => {}
            _ => panic!("Expected Quit"),
        }

        match result3 {
            InputResult::AttachSession(name) => assert_eq!(name, "test"),
            _ => panic!("Expected AttachSession"),
        }
    }
}
