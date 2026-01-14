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
    time::{Duration, Instant},
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

    /// Configure remote hosts for SSH tmux listing
    Host {
        #[command(subcommand)]
        command: HostCommands,
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

#[derive(Subcommand)]
enum HostCommands {
    /// Add a remote host
    Add {
        /// Friendly name for the host
        name: String,
        /// SSH target (user@host or host)
        host: String,
        /// Optional SSH key path
        #[arg(long)]
        key: Option<String>,
    },
    /// Remove a remote host by name
    Remove {
        /// Host name to remove
        name: String,
    },
    /// List configured remote hosts
    List,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TmuxSession {
    name: String,
    windows: usize,
    attached: bool,
    #[serde(default)]
    attached_clients: usize,
    #[serde(default)]
    attached_users: Vec<String>,
    created: String,
    activity: String,
    process_info: Option<ProcessInfo>,
    resource_info: Option<ResourceInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HostConfig {
    name: String,
    host: String,
    #[serde(default)]
    key: Option<String>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct HostsConfig {
    #[serde(default)]
    hosts: Vec<HostConfig>,
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

#[derive(Debug, Clone)]
struct RemoteHostSessions {
    host: HostConfig,
    sessions: Vec<TmuxSession>,
    error: Option<String>,
}

#[derive(Debug, Clone)]
enum SessionOrigin {
    Local,
    Remote(HostConfig),
}

#[derive(Debug, Clone)]
struct SessionEntry {
    origin: SessionOrigin,
    session: TmuxSession,
}

#[derive(Debug, Clone)]
enum ListEntry {
    Header {
        title: String,
        host: Option<HostConfig>,
    },
    Session(SessionEntry),
}

const AUTO_REFRESH_INTERVAL: Duration = Duration::from_secs(5);
const STATUS_MESSAGE_TTL: Duration = Duration::from_secs(4);
const SSH_LIST_TIMEOUT_SECS: u64 = 3;
const SSH_ATTACH_TIMEOUT_SECS: u64 = 5;
const SSH_ACTION_TIMEOUT_SECS: u64 = 5;
const TMUX_LIST_FORMAT: &str =
    "#{session_name}:#{session_windows}:#{session_attached}:#{session_created}:#{session_activity}";

struct App {
    sessions: Vec<TmuxSession>,
    remote_hosts: Vec<RemoteHostSessions>,
    selected: usize,
    show_help: bool,
    #[allow(dead_code)]
    aliases: HashMap<String, String>,
    hosts: Vec<HostConfig>,
    show_new_session_popup: bool,
    new_session_input: String,
    new_session_cursor: usize,
    new_session_target: NewSessionTarget,
    show_new_host_popup: bool,
    new_host_name_input: String,
    new_host_name_cursor: usize,
    new_host_host_input: String,
    new_host_host_cursor: usize,
    new_host_active_field: HostField,
    new_host_error: Option<String>,
    show_kill_confirm: bool,
    kill_confirm_target: Option<KillTarget>,
    status_message: Option<String>,
    status_message_expires: Option<Instant>,
    system: System,
}

impl App {
    fn new() -> Result<Self> {
        let aliases = load_aliases()?;
        let hosts = load_hosts()?;
        let mut system = System::new_all();
        system.refresh_all();
        let mut app = App {
            sessions: Vec::new(),
            remote_hosts: Vec::new(),
            selected: 0,
            show_help: false,
            aliases,
            hosts,
            show_new_session_popup: false,
            new_session_input: String::new(),
            new_session_cursor: 0,
            new_session_target: NewSessionTarget::Local,
            show_new_host_popup: false,
            new_host_name_input: String::new(),
            new_host_name_cursor: 0,
            new_host_host_input: String::new(),
            new_host_host_cursor: 0,
            new_host_active_field: HostField::Host,
            new_host_error: None,
            show_kill_confirm: false,
            kill_confirm_target: None,
            status_message: None,
            status_message_expires: None,
            system,
        };
        app.refresh()?;
        Ok(app)
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
        self.hosts = load_hosts()?;
        self.remote_hosts = get_remote_sessions(&self.hosts);
        let entries_len = self.build_entries().len();
        if entries_len == 0 {
            self.selected = 0;
        } else if self.selected >= entries_len {
            self.selected = entries_len - 1;
        }
        Ok(())
    }

    fn next(&mut self) {
        let entries_len = self.build_entries().len();
        if entries_len > 0 {
            self.selected = (self.selected + 1) % entries_len;
        }
    }

    fn previous(&mut self) {
        let entries_len = self.build_entries().len();
        if entries_len > 0 {
            self.selected = if self.selected == 0 {
                entries_len - 1
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
        self.new_session_cursor = 0;
        self.show_new_host_popup = false;
    }

    fn hide_new_session_popup(&mut self) {
        self.show_new_session_popup = false;
        self.new_session_input.clear();
        self.new_session_cursor = 0;
    }

    fn handle_new_session_input(&mut self, c: char) {
        insert_char_at(&mut self.new_session_input, c, &mut self.new_session_cursor);
    }

    fn backspace_new_session_input(&mut self) {
        remove_char_before(&mut self.new_session_input, &mut self.new_session_cursor);
    }

    fn show_new_host_popup(&mut self) {
        self.show_new_host_popup = true;
        self.new_host_name_input.clear();
        self.new_host_name_cursor = 0;
        self.new_host_host_input.clear();
        self.new_host_host_cursor = 0;
        self.new_host_active_field = HostField::Host;
        self.new_host_error = None;
        self.show_new_session_popup = false;
    }

    fn hide_new_host_popup(&mut self) {
        self.show_new_host_popup = false;
        self.new_host_name_input.clear();
        self.new_host_name_cursor = 0;
        self.new_host_host_input.clear();
        self.new_host_host_cursor = 0;
        self.new_host_active_field = HostField::Host;
        self.new_host_error = None;
    }

    fn show_kill_confirm(&mut self, target: KillTarget) {
        self.show_kill_confirm = true;
        self.kill_confirm_target = Some(target);
        self.show_new_session_popup = false;
        self.show_new_host_popup = false;
    }

    fn hide_kill_confirm(&mut self) {
        self.show_kill_confirm = false;
        self.kill_confirm_target = None;
    }

    fn handle_new_host_input(&mut self, c: char) {
        match self.new_host_active_field {
            HostField::Name => insert_char_at(
                &mut self.new_host_name_input,
                c,
                &mut self.new_host_name_cursor,
            ),
            HostField::Host => insert_char_at(
                &mut self.new_host_host_input,
                c,
                &mut self.new_host_host_cursor,
            ),
        }
    }

    fn backspace_new_host_input(&mut self) {
        match self.new_host_active_field {
            HostField::Name => remove_char_before(
                &mut self.new_host_name_input,
                &mut self.new_host_name_cursor,
            ),
            HostField::Host => remove_char_before(
                &mut self.new_host_host_input,
                &mut self.new_host_host_cursor,
            ),
        }
    }

    fn build_entries(&self) -> Vec<ListEntry> {
        let mut entries = Vec::new();
        let has_remote = !self.remote_hosts.is_empty();

        if has_remote {
            entries.push(ListEntry::Header {
                title: "Local".to_string(),
                host: None,
            });
        }

        for session in &self.sessions {
            entries.push(ListEntry::Session(SessionEntry {
                origin: SessionOrigin::Local,
                session: session.clone(),
            }));
        }

        for host_sessions in &self.remote_hosts {
            let mut header = if host_sessions.host.name == host_sessions.host.host {
                format!("Remote: {}", host_sessions.host.host)
            } else {
                format!(
                    "Remote: {} ({})",
                    host_sessions.host.name, host_sessions.host.host
                )
            };
            if host_sessions.error.is_some() {
                header.push_str(" - offline");
            }
            entries.push(ListEntry::Header {
                title: header,
                host: Some(host_sessions.host.clone()),
            });

            for session in &host_sessions.sessions {
                entries.push(ListEntry::Session(SessionEntry {
                    origin: SessionOrigin::Remote(host_sessions.host.clone()),
                    session: session.clone(),
                }));
            }
        }

        entries
    }

    fn set_status_message(&mut self, message: impl Into<String>) {
        self.status_message = Some(message.into());
        self.status_message_expires = Some(Instant::now() + STATUS_MESSAGE_TTL);
    }

    fn clear_expired_status(&mut self) {
        if let Some(expires) = self.status_message_expires {
            if Instant::now() >= expires {
                self.status_message = None;
                self.status_message_expires = None;
            }
        }
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
        Some(Commands::Host { command }) => manage_hosts(command)?,
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
    let output = executor.execute_command(&["list-sessions", "-F", TMUX_LIST_FORMAT])?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Handle various tmux error messages for no server
        if stderr.contains("no server running")
            || stderr.contains("no sessions")
            || stderr.contains("no current client")
            || stderr.contains("can't find session")
            || stderr.contains("server not found")
            || stderr.contains("error connecting to")
            || stderr.contains("No such file or directory")
            || stderr.contains("server exited unexpectedly")
        {
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

fn expand_tilde(path: &str) -> String {
    if let Some(stripped) = path.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home)
                .join(stripped)
                .to_string_lossy()
                .to_string();
        }
    }
    path.to_string()
}

fn shell_quote(value: &str) -> String {
    let mut escaped = String::from("'");
    for ch in value.chars() {
        if ch == '\'' {
            escaped.push_str("'\\''");
        } else {
            escaped.push(ch);
        }
    }
    escaped.push('\'');
    escaped
}

fn apply_ssh_args(cmd: &mut Command, host: &HostConfig, timeout_secs: u64, batch_mode: bool) {
    cmd.arg("-o")
        .arg(format!("ConnectTimeout={}", timeout_secs));
    if batch_mode {
        cmd.arg("-o")
            .arg("BatchMode=yes")
            .arg("-o")
            .arg("NumberOfPasswordPrompts=0");
    }
    if let Some(ref key) = host.key {
        cmd.arg("-i").arg(expand_tilde(key));
    }
    cmd.arg(&host.host);
}

fn get_tmux_sessions_remote(host: &HostConfig) -> Result<Vec<TmuxSession>> {
    let mut cmd = Command::new("ssh");
    apply_ssh_args(&mut cmd, host, SSH_LIST_TIMEOUT_SECS, true);
    let remote_cmd = format!("tmux list-sessions -F \"{}\"", TMUX_LIST_FORMAT);
    cmd.arg(remote_cmd);

    let output = cmd.output().context("Failed to execute ssh command")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("no server running")
            || stderr.contains("no sessions")
            || stderr.contains("no current client")
            || stderr.contains("can't find session")
            || stderr.contains("server not found")
            || stderr.contains("error connecting to")
            || stderr.contains("No such file or directory")
            || stderr.contains("server exited unexpectedly")
        {
            return Ok(Vec::new());
        }
        return Err(anyhow::anyhow!("{}", stderr.trim()));
    }

    Ok(parse_tmux_sessions(&String::from_utf8_lossy(
        &output.stdout,
    )))
}

fn get_remote_sessions(hosts: &[HostConfig]) -> Vec<RemoteHostSessions> {
    hosts
        .iter()
        .map(|host| match get_tmux_sessions_remote(host) {
            Ok(sessions) => RemoteHostSessions {
                host: host.clone(),
                sessions,
                error: None,
            },
            Err(err) => RemoteHostSessions {
                host: host.clone(),
                sessions: Vec::new(),
                error: Some(err.to_string()),
            },
        })
        .collect()
}

fn parse_tmux_sessions(output: &str) -> Vec<TmuxSession> {
    output
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.split(':').collect();
            if parts.len() >= 5 {
                let attached_clients = parts[2].parse::<usize>().unwrap_or(0);
                Some(TmuxSession {
                    name: parts[0].to_string(),
                    windows: parts[1].parse().unwrap_or(0),
                    attached: attached_clients > 0,
                    attached_clients,
                    attached_users: Vec::new(),
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

            if session.attached_clients > 0 {
                if let Ok(output) = executor.execute_command(&[
                    "list-clients",
                    "-t",
                    &session.name,
                    "-F",
                    "#{client_user}",
                ]) {
                    if output.status.success() {
                        let mut users: Vec<String> = String::from_utf8_lossy(&output.stdout)
                            .lines()
                            .map(str::trim)
                            .filter(|line| !line.is_empty())
                            .map(|line| line.to_string())
                            .collect();
                        users.sort();
                        users.dedup();
                        session.attached_users = users;
                    }
                }
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

    if session.attached_clients > 0 && session.attached_users.is_empty() {
        if let Some(ref process) = session.process_info {
            session.attached_users = vec![process.user.clone()];
        }
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

    let _ = Command::new("tmux")
        .args(["set-option", "-g", "detach-on-destroy", "on"])
        .output();

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

fn attach_remote_session(host: &HostConfig, session_name: &str) -> Result<()> {
    let mut cmd = Command::new("ssh");
    cmd.arg("-t");
    apply_ssh_args(&mut cmd, host, SSH_ATTACH_TIMEOUT_SECS, false);
    let remote_cmd = format!(
        "tmux set-option -g detach-on-destroy on >/dev/null 2>&1; tmux attach-session -t {}",
        session_name
    );
    let status = cmd
        .arg(remote_cmd)
        .status()
        .context("Failed to execute ssh attach command")?;

    if !status.success() {
        return Err(anyhow::anyhow!(
            "Failed to attach to remote session '{}' on '{}'",
            session_name,
            host.name
        ));
    }

    Ok(())
}

fn kill_remote_session(host: &HostConfig, session_name: &str) -> Result<()> {
    let mut cmd = Command::new("ssh");
    apply_ssh_args(&mut cmd, host, SSH_ACTION_TIMEOUT_SECS, true);
    let remote_cmd = format!("tmux kill-session -t {}", shell_quote(session_name));
    let status = cmd
        .arg(remote_cmd)
        .status()
        .context("Failed to execute ssh kill-session command")?;

    if !status.success() {
        return Err(anyhow::anyhow!(
            "Failed to kill remote session '{}' on '{}'",
            session_name,
            host.name
        ));
    }

    Ok(())
}

fn new_session_remote(host: &HostConfig, name: Option<String>) -> Result<()> {
    let mut cmd = Command::new("ssh");
    apply_ssh_args(&mut cmd, host, SSH_ATTACH_TIMEOUT_SECS, false);

    let remote_cmd = match name {
        Some(name) => format!("tmux new-session -d -s {}", shell_quote(&name)),
        None => "tmux new-session -d".to_string(),
    };

    let status = cmd
        .arg(remote_cmd)
        .status()
        .context("Failed to execute ssh new-session command")?;

    if !status.success() {
        return Err(anyhow::anyhow!(
            "Failed to create remote session on '{}'",
            host.name
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
    let target_session = match session_name {
        Some(name) => name,
        None => {
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

fn hosts_config_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".cmux_hosts.toml")
}

fn load_hosts() -> Result<Vec<HostConfig>> {
    let path = hosts_config_path();
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(&path)?;
    if content.trim().is_empty() {
        return Ok(Vec::new());
    }
    let config: HostsConfig = toml::from_str(&content).context("Failed to parse hosts config")?;
    Ok(config.hosts)
}

fn save_hosts(hosts: &[HostConfig]) -> Result<()> {
    let config = HostsConfig {
        hosts: hosts.to_vec(),
    };
    let path = hosts_config_path();
    let content = toml::to_string_pretty(&config).context("Failed to serialize hosts config")?;
    fs::write(&path, content)?;
    Ok(())
}

fn add_host_config(host: HostConfig) -> Result<()> {
    let mut hosts = load_hosts()?;
    if hosts.iter().any(|h| h.name == host.name) {
        return Err(anyhow::anyhow!("Host '{}' already exists", host.name));
    }
    hosts.push(host);
    save_hosts(&hosts)?;
    Ok(())
}

fn remove_host_config(name: &str) -> Result<()> {
    let mut hosts = load_hosts()?;
    let original_len = hosts.len();
    hosts.retain(|h| h.name != name);
    if hosts.len() == original_len {
        return Err(anyhow::anyhow!("Host '{}' not found", name));
    }
    save_hosts(&hosts)?;
    Ok(())
}

fn list_hosts() -> Result<()> {
    let hosts = load_hosts()?;
    if hosts.is_empty() {
        println!("No remote hosts configured.");
        return Ok(());
    }

    println!("Remote hosts:");
    println!("{:<16} {:<24} Key", "Name", "Host");
    println!("{}", "-".repeat(60));
    for host in hosts {
        let key = host.key.unwrap_or_else(|| "default".to_string());
        println!("{:<16} {:<24} {}", host.name, host.host, key);
    }
    Ok(())
}

fn manage_hosts(command: HostCommands) -> Result<()> {
    match command {
        HostCommands::Add { name, host, key } => {
            add_host_config(HostConfig { name, host, key })?;
            println!("Added host.");
        }
        HostCommands::Remove { name } => {
            remove_host_config(&name)?;
            println!("Removed host '{}'.", name);
        }
        HostCommands::List => list_hosts()?,
    }
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
        // Auto-refresh periodically so new sessions appear without input
        if last_refresh.elapsed() >= AUTO_REFRESH_INTERVAL {
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
            let user = format_attached_users(s);
            let (memory_info, cpu_info) = if let Some(ref resource) = s.resource_info {
                (
                    format!("{:.1}MB", resource.memory_mb),
                    format!("{:.1}%", resource.cpu_percent),
                )
            } else {
                ("N/A".to_string(), "N/A".to_string())
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

    let title = " │ Name             │Win │  Memory │   CPU │ Clients ";
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
    let mut last_refresh = Instant::now();

    loop {
        terminal.draw(|f| draw_ui(f, &mut app, &mut list_state))?;

        let timeout = AUTO_REFRESH_INTERVAL
            .checked_sub(last_refresh.elapsed())
            .unwrap_or(Duration::from_secs(0));

        if event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                match handle_input(&mut app, key)? {
                    InputResult::Continue => {}
                    InputResult::Quit => break,
                    InputResult::AttachSession(target) => {
                        // Clean up terminal before attaching
                        disable_raw_mode()?;
                        execute!(
                            terminal.backend_mut(),
                            LeaveAlternateScreen,
                            DisableMouseCapture
                        )?;
                        terminal.show_cursor()?;

                        // Attach to session
                        match target {
                            AttachTarget::Local(name) => {
                                attach_session(Some(name))?;
                            }
                            AttachTarget::Remote(host, name) => {
                                attach_remote_session(&host, &name)?;
                            }
                        }

                        // Re-enter TUI mode after detaching
                        let mut new_stdout = io::stdout();
                        hard_reset_terminal(&mut new_stdout)?;
                        enable_raw_mode()?;
                        execute!(new_stdout, EnterAlternateScreen, EnableMouseCapture)?;

                        // Clear the screen and refresh the terminal
                        execute!(
                            new_stdout,
                            crossterm::terminal::Clear(crossterm::terminal::ClearType::All),
                            crossterm::terminal::Clear(crossterm::terminal::ClearType::Purge),
                            crossterm::cursor::MoveTo(0, 0)
                        )?;
                        let backend = CrosstermBackend::new(new_stdout);
                        terminal = Terminal::new(backend)?;
                        terminal.hide_cursor()?;
                        terminal.clear()?;
                        app.refresh()?;
                        terminal.draw(|f| draw_ui(f, &mut app, &mut list_state))?;
                        last_refresh = Instant::now();
                    }
                    InputResult::Refreshed => {
                        last_refresh = Instant::now();
                    }
                }
            }
        }

        if last_refresh.elapsed() >= AUTO_REFRESH_INTERVAL {
            app.refresh()?;
            last_refresh = Instant::now();
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

fn hard_reset_terminal(stdout: &mut impl Write) -> Result<()> {
    stdout.write_all(b"\x1bc")?;
    stdout.flush()?;
    Ok(())
}

enum InputResult {
    Continue,
    Quit,
    AttachSession(AttachTarget),
    Refreshed,
}

#[derive(Debug, Clone)]
enum AttachTarget {
    Local(String),
    Remote(HostConfig, String),
}

#[derive(Debug, Clone)]
enum NewSessionTarget {
    Local,
    Remote(HostConfig),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HostField {
    Name,
    Host,
}

#[derive(Debug, Clone)]
struct KillTarget {
    origin: SessionOrigin,
    session_name: String,
    attached_clients: usize,
}

fn handle_input(app: &mut App, key: KeyEvent) -> Result<InputResult> {
    // Handle Ctrl+C for exit
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return Ok(InputResult::Quit);
    }

    if app.show_new_host_popup {
        match key.code {
            KeyCode::Enter => {
                match build_host_config(&app.new_host_name_input, &app.new_host_host_input) {
                    Ok(host) => {
                        add_host_config(host)?;
                        app.hide_new_host_popup();
                        app.refresh()?;
                        return Ok(InputResult::Refreshed);
                    }
                    Err(err) => {
                        app.new_host_error = Some(err.to_string());
                    }
                }
            }
            KeyCode::Esc => {
                app.hide_new_host_popup();
            }
            KeyCode::Tab | KeyCode::Down => {
                app.new_host_active_field = match app.new_host_active_field {
                    HostField::Name => HostField::Host,
                    HostField::Host => HostField::Name,
                };
            }
            KeyCode::BackTab | KeyCode::Up => {
                app.new_host_active_field = match app.new_host_active_field {
                    HostField::Name => HostField::Host,
                    HostField::Host => HostField::Name,
                };
            }
            KeyCode::Left => {
                move_cursor_left(
                    &app.new_host_active_field,
                    &mut app.new_host_name_cursor,
                    &mut app.new_host_host_cursor,
                );
            }
            KeyCode::Right => {
                move_cursor_right(
                    &app.new_host_active_field,
                    &app.new_host_name_input,
                    &app.new_host_host_input,
                    &mut app.new_host_name_cursor,
                    &mut app.new_host_host_cursor,
                );
            }
            KeyCode::Home => {
                set_cursor_start(
                    &app.new_host_active_field,
                    &mut app.new_host_name_cursor,
                    &mut app.new_host_host_cursor,
                );
            }
            KeyCode::End => {
                set_cursor_end(
                    &app.new_host_active_field,
                    &app.new_host_name_input,
                    &app.new_host_host_input,
                    &mut app.new_host_name_cursor,
                    &mut app.new_host_host_cursor,
                );
            }
            KeyCode::Delete => {
                app.new_host_error = None;
                match app.new_host_active_field {
                    HostField::Name => {
                        remove_char_at(&mut app.new_host_name_input, &mut app.new_host_name_cursor);
                    }
                    HostField::Host => {
                        remove_char_at(&mut app.new_host_host_input, &mut app.new_host_host_cursor);
                    }
                }
            }
            KeyCode::Backspace => {
                app.new_host_error = None;
                app.backspace_new_host_input();
            }
            KeyCode::Char(c) => {
                app.new_host_error = None;
                app.handle_new_host_input(c);
            }
            _ => {}
        }
        return Ok(InputResult::Continue);
    }

    if app.show_kill_confirm {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                if let Some(target) = app.kill_confirm_target.clone() {
                    match target.origin {
                        SessionOrigin::Local => {
                            kill_session(Some(target.session_name.clone()))?;
                            app.set_status_message("Session killed.");
                            app.refresh()?;
                            app.hide_kill_confirm();
                            return Ok(InputResult::Refreshed);
                        }
                        SessionOrigin::Remote(host) => {
                            match kill_remote_session(&host, &target.session_name) {
                                Ok(()) => {
                                    app.set_status_message("Remote session killed.");
                                    app.refresh()?;
                                    app.hide_kill_confirm();
                                    return Ok(InputResult::Refreshed);
                                }
                                Err(err) => {
                                    app.set_status_message(format!("Kill failed: {}", err));
                                }
                            }
                        }
                    }
                }
                app.hide_kill_confirm();
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                app.hide_kill_confirm();
            }
            _ => {}
        }
        return Ok(InputResult::Continue);
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
                match app.new_session_target.clone() {
                    NewSessionTarget::Local => {
                        new_session(Some(session_name))?;
                    }
                    NewSessionTarget::Remote(host) => {
                        new_session_remote(&host, Some(session_name))?;
                    }
                }
                app.hide_new_session_popup();
                app.refresh()?;
                return Ok(InputResult::Refreshed);
            }
            KeyCode::Esc => {
                app.hide_new_session_popup();
            }
            KeyCode::Left => {
                if app.new_session_cursor > 0 {
                    app.new_session_cursor -= 1;
                }
            }
            KeyCode::Right => {
                let len = app.new_session_input.chars().count();
                if app.new_session_cursor < len {
                    app.new_session_cursor += 1;
                }
            }
            KeyCode::Home => {
                app.new_session_cursor = 0;
            }
            KeyCode::End => {
                app.new_session_cursor = app.new_session_input.chars().count();
            }
            KeyCode::Delete => {
                remove_char_at(&mut app.new_session_input, &mut app.new_session_cursor);
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

    let entries = app.build_entries();

    // Normal input handling
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => return Ok(InputResult::Quit),
        KeyCode::Char('?') | KeyCode::Char('h') => app.toggle_help(),
        KeyCode::Down | KeyCode::Char('j') => app.next(),
        KeyCode::Up | KeyCode::Char('k') => app.previous(),
        KeyCode::Enter => {
            if let Some(ListEntry::Session(entry)) = entries.get(app.selected) {
                match &entry.origin {
                    SessionOrigin::Local => {
                        return Ok(InputResult::AttachSession(AttachTarget::Local(
                            entry.session.name.clone(),
                        )));
                    }
                    SessionOrigin::Remote(host) => {
                        return Ok(InputResult::AttachSession(AttachTarget::Remote(
                            host.clone(),
                            entry.session.name.clone(),
                        )));
                    }
                }
            }
        }
        KeyCode::Char('n') => {
            app.new_session_target = match entries.get(app.selected) {
                Some(ListEntry::Session(entry)) => match &entry.origin {
                    SessionOrigin::Local => NewSessionTarget::Local,
                    SessionOrigin::Remote(host) => NewSessionTarget::Remote(host.clone()),
                },
                Some(ListEntry::Header {
                    host: Some(host), ..
                }) => NewSessionTarget::Remote(host.clone()),
                _ => NewSessionTarget::Local,
            };
            app.show_new_session_popup();
        }
        KeyCode::Char('H') => {
            app.show_new_host_popup();
        }
        KeyCode::Char('K') => {
            // Kill selected session
            if let Some(ListEntry::Session(entry)) = entries.get(app.selected) {
                if entry.session.attached_clients > 0 {
                    app.show_kill_confirm(KillTarget {
                        origin: entry.origin.clone(),
                        session_name: entry.session.name.clone(),
                        attached_clients: entry.session.attached_clients,
                    });
                    return Ok(InputResult::Continue);
                }

                match &entry.origin {
                    SessionOrigin::Local => {
                        let session_name = entry.session.name.clone();
                        kill_session(Some(session_name))?;
                        app.refresh()?;
                        return Ok(InputResult::Refreshed);
                    }
                    SessionOrigin::Remote(host) => {
                        match kill_remote_session(host, &entry.session.name) {
                            Ok(()) => {
                                app.set_status_message("Remote session killed.");
                                app.refresh()?;
                                return Ok(InputResult::Refreshed);
                            }
                            Err(err) => {
                                app.set_status_message(format!("Kill failed: {}", err));
                                return Ok(InputResult::Continue);
                            }
                        }
                    }
                }
            }
        }
        KeyCode::Char('r') => {
            // Refresh session list
            app.refresh()?;
            return Ok(InputResult::Refreshed);
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

fn format_attached_users(session: &TmuxSession) -> String {
    if session.attached_clients == 0 {
        return "none".to_string();
    }

    if !session.attached_users.is_empty() {
        if session.attached_users.len() == 1 {
            return session.attached_users[0].clone();
        }
        let extra = session.attached_users.len() - 1;
        return format!("{}+{}", session.attached_users[0], extra);
    }

    format!(
        "{} client{}",
        session.attached_clients,
        if session.attached_clients == 1 {
            ""
        } else {
            "s"
        }
    )
}

fn build_host_config(name_input: &str, host_input: &str) -> Result<HostConfig> {
    let host = host_input.trim();
    if host.is_empty() {
        return Err(anyhow::anyhow!("Host is required"));
    }

    let mut name = name_input.trim().to_string();
    if name.is_empty() {
        name = default_host_name(host);
    }

    Ok(HostConfig {
        name,
        host: host.to_string(),
        key: None,
    })
}

fn default_host_name(host: &str) -> String {
    if let Some((_, host_part)) = host.rsplit_once('@') {
        host_part.to_string()
    } else {
        host.to_string()
    }
}

fn insert_char_at(text: &mut String, ch: char, cursor: &mut usize) {
    let mut chars: Vec<char> = text.chars().collect();
    let pos = (*cursor).min(chars.len());
    chars.insert(pos, ch);
    *text = chars.iter().collect();
    *cursor = pos + 1;
}

fn remove_char_before(text: &mut String, cursor: &mut usize) {
    if *cursor == 0 {
        return;
    }
    let mut chars: Vec<char> = text.chars().collect();
    let pos = (*cursor).min(chars.len());
    if pos == 0 {
        return;
    }
    chars.remove(pos - 1);
    *text = chars.iter().collect();
    *cursor = pos - 1;
}

fn remove_char_at(text: &mut String, cursor: &mut usize) {
    let mut chars: Vec<char> = text.chars().collect();
    let pos = (*cursor).min(chars.len());
    if pos >= chars.len() {
        return;
    }
    chars.remove(pos);
    *text = chars.iter().collect();
}

fn move_cursor_left(active: &HostField, name_cursor: &mut usize, host_cursor: &mut usize) {
    match active {
        HostField::Name => {
            if *name_cursor > 0 {
                *name_cursor -= 1;
            }
        }
        HostField::Host => {
            if *host_cursor > 0 {
                *host_cursor -= 1;
            }
        }
    }
}

fn move_cursor_right(
    active: &HostField,
    name_input: &str,
    host_input: &str,
    name_cursor: &mut usize,
    host_cursor: &mut usize,
) {
    match active {
        HostField::Name => {
            let len = name_input.chars().count();
            if *name_cursor < len {
                *name_cursor += 1;
            }
        }
        HostField::Host => {
            let len = host_input.chars().count();
            if *host_cursor < len {
                *host_cursor += 1;
            }
        }
    }
}

fn set_cursor_start(active: &HostField, name_cursor: &mut usize, host_cursor: &mut usize) {
    match active {
        HostField::Name => *name_cursor = 0,
        HostField::Host => *host_cursor = 0,
    }
}

fn set_cursor_end(
    active: &HostField,
    name_input: &str,
    host_input: &str,
    name_cursor: &mut usize,
    host_cursor: &mut usize,
) {
    match active {
        HostField::Name => *name_cursor = name_input.chars().count(),
        HostField::Host => *host_cursor = host_input.chars().count(),
    }
}

fn with_cursor(text: &str, cursor: usize, active: bool) -> String {
    if !active {
        return text.to_string();
    }

    let mut result = String::new();
    let mut idx = 0;
    for ch in text.chars() {
        if idx == cursor {
            result.push('|');
        }
        result.push(ch);
        idx += 1;
    }
    if cursor >= idx {
        result.push('|');
    }
    result
}

fn draw_ui(f: &mut Frame, app: &mut App, list_state: &mut ListState) {
    app.clear_expired_status();
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
    let entries = app.build_entries();
    if entries.is_empty() {
        let empty_msg =
            Paragraph::new("No tmux sessions found.\nPress 'n' to create a new session.")
                .style(Style::default().fg(Color::Gray))
                .alignment(Alignment::Center)
                .block(Block::default().borders(Borders::ALL).title("Sessions"));
        f.render_widget(empty_msg, chunks[1]);
    } else {
        let sessions: Vec<ListItem> = entries
            .iter()
            .enumerate()
            .map(|(i, entry)| {
                let is_selected = i == app.selected;
                match entry {
                    ListEntry::Header { title, .. } => {
                        let content = Line::from(vec![Span::styled(
                            title.as_str(),
                            Style::default()
                                .fg(Color::Cyan)
                                .add_modifier(Modifier::BOLD),
                        )]);
                        let mut item = ListItem::new(content);
                        if is_selected {
                            item = item.style(app.get_highlight_style());
                        }
                        item
                    }
                    ListEntry::Session(entry) => {
                        let s = &entry.session;
                        let status = if s.attached { "●" } else { "○" };
                        let user = format_attached_users(s);

                        // Get resource info
                        let (memory_info, cpu_info) = if let Some(ref resource) = s.resource_info {
                            (
                                format!("{:.1}MB", resource.memory_mb),
                                format!("{:.1}%", resource.cpu_percent),
                            )
                        } else {
                            ("N/A".to_string(), "N/A".to_string())
                        };

                        // Add selection indicator prefix for better visibility
                        let selection_prefix = app.get_selection_prefix(is_selected);

                        let content = Line::from(vec![
                            Span::styled(
                                format!("{:<1}", selection_prefix),
                                Style::default()
                                    .fg(if is_selected {
                                        Color::Yellow
                                    } else {
                                        Color::DarkGray
                                    })
                                    .add_modifier(if is_selected {
                                        Modifier::BOLD
                                    } else {
                                        Modifier::empty()
                                    }),
                            ),
                            Span::styled(
                                format!("{:<1}", status),
                                Style::default().fg(if s.attached {
                                    Color::Green
                                } else {
                                    Color::Red
                                }),
                            ),
                            Span::raw(" "),
                            Span::styled(
                                format!("{:<15}", s.name),
                                Style::default()
                                    .fg(if is_selected {
                                        Color::Yellow
                                    } else {
                                        Color::White
                                    })
                                    .add_modifier(if is_selected {
                                        Modifier::BOLD | Modifier::UNDERLINED
                                    } else {
                                        Modifier::BOLD
                                    }),
                            ),
                            Span::styled(
                                format!("{:>3}W", s.windows),
                                Style::default().fg(if is_selected {
                                    Color::Yellow
                                } else {
                                    Color::White
                                }),
                            ),
                            Span::raw(" "),
                            Span::styled(
                                format!("{:>8}", memory_info),
                                Style::default().fg(if is_selected {
                                    Color::Yellow
                                } else {
                                    Color::Cyan
                                }),
                            ),
                            Span::raw(" "),
                            Span::styled(
                                format!("{:>6}", cpu_info),
                                Style::default().fg(if is_selected {
                                    Color::Yellow
                                } else {
                                    Color::Magenta
                                }),
                            ),
                            Span::raw(" "),
                            Span::styled(
                                format!("{:<8}", user),
                                Style::default().fg(if is_selected {
                                    Color::Yellow
                                } else {
                                    Color::Gray
                                }),
                            ),
                        ]);

                        let mut item = ListItem::new(content);
                        if is_selected {
                            // Use terminal-aware highlighting
                            item = item.style(app.get_highlight_style());
                        }
                        item
                    }
                }
            })
            .collect();

        let title = "Sessions │ Name        │ Win │ Memory │ CPU   │ Clients ";
        let sessions_list = List::new(sessions)
            .block(Block::default().borders(Borders::ALL).title(title))
            .highlight_style(app.get_highlight_style())
            .highlight_symbol(app.get_selection_symbol());

        list_state.select(Some(app.selected));
        f.render_stateful_widget(sessions_list, chunks[1], list_state);
    }

    // Controls/Help
    let mut help_text: Vec<String> = if app.show_help {
        vec![
            "↑/↓/j/k: Navigate    Enter: Attach    n: New session    H: Add host".to_string(),
            "K: Kill session      r: Refresh       s: Save snapshot".to_string(),
            "d: Debug terminal    q/Esc/Ctrl+C: Quit  ?: Toggle help".to_string(),
        ]
    } else {
        vec!["Navigate: ↑/↓  Attach: Enter  New: n  Host: H  Kill: K  Debug: d  Quit: q/Ctrl+C  Help: ?".to_string()]
    };
    if let Some(ref message) = app.status_message {
        help_text.push(format!("Status: {}", message));
    }

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
    if app.show_new_host_popup {
        draw_new_host_popup(f, app);
    }
    if app.show_kill_confirm {
        draw_kill_confirm_popup(f, app);
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
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(popup_area);

    f.render_widget(popup_block, popup_area);

    let input_text = Paragraph::new("Enter session name (or press Enter for default):")
        .style(Style::default().fg(Color::White));
    f.render_widget(input_text, popup_chunks[0]);

    let input_display = with_cursor(&app.new_session_input, app.new_session_cursor, true);
    let input_field = Paragraph::new(input_display)
        .style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(input_field, popup_chunks[1]);

    let target_label = match &app.new_session_target {
        NewSessionTarget::Local => "Target: local".to_string(),
        NewSessionTarget::Remote(host) => format!("Target: {} ({})", host.name, host.host),
    };
    let target_text = Paragraph::new(target_label).style(Style::default().fg(Color::Gray));
    f.render_widget(target_text, popup_chunks[2]);

    let default_name = format!("Default: session-{}", chrono::Local::now().format("%H%M%S"));
    let default_text = Paragraph::new(default_name).style(Style::default().fg(Color::Gray));
    f.render_widget(default_text, popup_chunks[3]);

    let help_text = Paragraph::new("Enter: Create  Esc: Cancel")
        .style(Style::default().fg(Color::Gray))
        .alignment(Alignment::Center);
    f.render_widget(help_text, popup_chunks[4]);
}

fn draw_new_host_popup(f: &mut Frame, app: &App) {
    let popup_area = centered_rect(70, 40, f.size());

    f.render_widget(Clear, popup_area);

    let popup_block = Block::default()
        .title("Add Remote Host")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));

    let popup_chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(popup_area);

    f.render_widget(popup_block, popup_area);

    let input_text = Paragraph::new("Add a remote host (name optional; key via config or CLI)")
        .style(Style::default().fg(Color::White));
    f.render_widget(input_text, popup_chunks[0]);

    let name_active = app.new_host_active_field == HostField::Name;
    let host_active = app.new_host_active_field == HostField::Host;

    let name_label = Paragraph::new("Name (optional)").style(Style::default().fg(Color::Gray));
    f.render_widget(name_label, popup_chunks[1]);

    let name_display = with_cursor(
        &app.new_host_name_input,
        app.new_host_name_cursor,
        name_active,
    );
    let name_field = Paragraph::new(name_display)
        .style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(if name_active {
                    Color::Yellow
                } else {
                    Color::DarkGray
                })),
        );
    f.render_widget(name_field, popup_chunks[2]);

    let host_label = Paragraph::new("Host (user@host)").style(Style::default().fg(Color::Gray));
    f.render_widget(host_label, popup_chunks[3]);

    let host_display = with_cursor(
        &app.new_host_host_input,
        app.new_host_host_cursor,
        host_active,
    );
    let host_field = Paragraph::new(host_display)
        .style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(if host_active {
                    Color::Yellow
                } else {
                    Color::DarkGray
                })),
        );
    f.render_widget(host_field, popup_chunks[4]);

    let help_text = Paragraph::new("Tab/Shift+Tab: Switch  Enter: Save  Esc: Cancel")
        .style(Style::default().fg(Color::Gray))
        .alignment(Alignment::Center);
    f.render_widget(help_text, popup_chunks[5]);

    if let Some(ref error) = app.new_host_error {
        let error_text = Paragraph::new(error.as_str())
            .style(Style::default().fg(Color::Red))
            .alignment(Alignment::Left);
        f.render_widget(error_text, popup_chunks[6]);
    }
}

fn draw_kill_confirm_popup(f: &mut Frame, app: &App) {
    let Some(ref target) = app.kill_confirm_target else {
        return;
    };

    let popup_area = centered_rect(60, 25, f.size());
    f.render_widget(Clear, popup_area);

    let popup_block = Block::default()
        .title("Confirm Kill")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red));

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

    let origin_label = match &target.origin {
        SessionOrigin::Local => "Local session".to_string(),
        SessionOrigin::Remote(host) => format!("Remote: {} ({})", host.name, host.host),
    };
    let line1 = Paragraph::new(origin_label).style(Style::default().fg(Color::Gray));
    f.render_widget(line1, popup_chunks[0]);

    let line2 = Paragraph::new(format!(
        "Kill session '{}' with {} attached client(s)?",
        target.session_name, target.attached_clients
    ))
    .style(
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    );
    f.render_widget(line2, popup_chunks[1]);

    let help_text = Paragraph::new("Enter/Y: Kill  N/Esc: Cancel")
        .style(Style::default().fg(Color::Gray))
        .alignment(Alignment::Center);
    f.render_widget(help_text, popup_chunks[2]);
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
    #[cfg(unix)]
    use std::os::unix::process::ExitStatusExt;
    #[cfg(windows)]
    use std::os::windows::process::ExitStatusExt;
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

        #[allow(dead_code)]
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
        let output = "main:3:2:1234567890:1234567890\ndev:1:0:1234567891:1234567891\ntest:2:1:1234567892:1234567892";
        let sessions = parse_tmux_sessions(output);

        assert_eq!(sessions.len(), 3);

        assert_eq!(sessions[0].name, "main");
        assert_eq!(sessions[0].windows, 3);
        assert!(sessions[0].attached);
        assert_eq!(sessions[0].attached_clients, 2);

        assert_eq!(sessions[1].name, "dev");
        assert_eq!(sessions[1].windows, 1);
        assert!(!sessions[1].attached);
        assert_eq!(sessions[1].attached_clients, 0);

        assert_eq!(sessions[2].name, "test");
        assert_eq!(sessions[2].windows, 2);
        assert!(sessions[2].attached);
        assert_eq!(sessions[2].attached_clients, 1);
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
            attached_clients: 1,
            attached_users: Vec::new(),
            created: "1234567890".to_string(),
            activity: "1234567890".to_string(),
            process_info: None,
            resource_info: None,
        };

        assert_eq!(session.name, "test");
        assert_eq!(session.windows, 2);
        assert!(session.attached);
    }

    #[test]
    fn test_app_navigation() {
        let mut app = App {
            sessions: vec![
                TmuxSession {
                    name: "session1".to_string(),
                    windows: 1,
                    attached: false,
                    attached_clients: 0,
                    attached_users: Vec::new(),
                    created: "123".to_string(),
                    activity: "123".to_string(),
                    process_info: None,
                    resource_info: None,
                },
                TmuxSession {
                    name: "session2".to_string(),
                    windows: 2,
                    attached: false,
                    attached_clients: 0,
                    attached_users: Vec::new(),
                    created: "124".to_string(),
                    activity: "124".to_string(),
                    process_info: None,
                    resource_info: None,
                },
                TmuxSession {
                    name: "session3".to_string(),
                    windows: 3,
                    attached: false,
                    attached_clients: 0,
                    attached_users: Vec::new(),
                    created: "125".to_string(),
                    activity: "125".to_string(),
                    process_info: None,
                    resource_info: None,
                },
            ],
            remote_hosts: Vec::new(),
            selected: 0,
            show_help: false,
            aliases: HashMap::new(),
            hosts: Vec::new(),
            show_new_session_popup: false,
            new_session_input: String::new(),
            new_session_cursor: 0,
            new_session_target: NewSessionTarget::Local,
            show_new_host_popup: false,
            new_host_name_input: String::new(),
            new_host_name_cursor: 0,
            new_host_host_input: String::new(),
            new_host_host_cursor: 0,
            new_host_active_field: HostField::Host,
            new_host_error: None,
            show_kill_confirm: false,
            kill_confirm_target: None,
            status_message: None,
            status_message_expires: None,
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
            remote_hosts: Vec::new(),
            selected: 0,
            show_help: false,
            aliases: HashMap::new(),
            hosts: Vec::new(),
            show_new_session_popup: false,
            new_session_input: String::new(),
            new_session_cursor: 0,
            new_session_target: NewSessionTarget::Local,
            show_new_host_popup: false,
            new_host_name_input: String::new(),
            new_host_name_cursor: 0,
            new_host_host_input: String::new(),
            new_host_host_cursor: 0,
            new_host_active_field: HostField::Host,
            new_host_error: None,
            show_kill_confirm: false,
            kill_confirm_target: None,
            status_message: None,
            status_message_expires: None,
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
            remote_hosts: Vec::new(),
            selected: 0,
            show_help: false,
            aliases: HashMap::new(),
            hosts: Vec::new(),
            show_new_session_popup: false,
            new_session_input: String::new(),
            new_session_cursor: 0,
            new_session_target: NewSessionTarget::Local,
            show_new_host_popup: false,
            new_host_name_input: String::new(),
            new_host_name_cursor: 0,
            new_host_host_input: String::new(),
            new_host_host_cursor: 0,
            new_host_active_field: HostField::Host,
            new_host_error: None,
            show_kill_confirm: false,
            kill_confirm_target: None,
            status_message: None,
            status_message_expires: None,
            system: System::new_all(),
        };

        assert!(!app.show_help);
        app.toggle_help();
        assert!(app.show_help);
        app.toggle_help();
        assert!(!app.show_help);
    }

    #[test]
    fn test_session_snapshot_serialization() {
        let sessions = vec![TmuxSession {
            name: "main".to_string(),
            windows: 3,
            attached: true,
            attached_clients: 1,
            attached_users: Vec::new(),
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
        let result3 = InputResult::AttachSession(AttachTarget::Local("test".to_string()));
        let result4 = InputResult::Refreshed;

        match result1 {
            InputResult::Continue => {}
            _ => panic!("Expected Continue"),
        }

        match result2 {
            InputResult::Quit => {}
            _ => panic!("Expected Quit"),
        }

        match result3 {
            InputResult::AttachSession(AttachTarget::Local(name)) => assert_eq!(name, "test"),
            _ => panic!("Expected AttachSession"),
        }

        match result4 {
            InputResult::Refreshed => {}
            _ => panic!("Expected Refreshed"),
        }
    }
}
