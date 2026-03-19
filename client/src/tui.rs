use std::collections::VecDeque;
use std::io;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::time::Duration;

use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Terminal,
};

// TODO: Multi-agent TUI support
// To fully support multiple agents:
// 1. Load repos.toml on TUI start via crate::config::load_repos()
// 2. Maintain state for multiple agents (HashMap<String, AgentState>)
// 3. Add key bindings to switch between agents (e.g., arrow keys, numbered selection)
// 4. Render a left sidebar showing all registered agents + online status
// 5. Query /team periodically to determine which agents are online
// 6. Allow filtering the log and task view by selected agent
// For now, this TUI continues to display single-agent mode

const MAX_LOG: usize = 200;
const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const TICK_MS: u64 = 80;

#[derive(Debug)]
pub enum TuiEvent {
    AgentInfo { name: String, id: String, server: String },
    ServerStatus(bool),
    Triaging { count: usize },
    TaskStart { id: String, title: String },
    SpinnerUpdate(String),
    TaskDone { id: String, status: String, chars: usize },
    TaskSkip { id: String, reason: String },
    TaskBlock { id: String, reason: String },
    MsgRecv { from: String },
    MsgSent { chars: usize },
    Log(String),
    Shutdown,
}

struct DaemonState {
    agent_name: String,
    agent_id: String,
    server: String,
    online: bool,
    current_task_id: String,
    current_task_title: String,
    spinner_text: String,
    log: VecDeque<String>,
    spinner_frame: usize,
}

impl Default for DaemonState {
    fn default() -> Self {
        Self {
            agent_name: String::new(),
            agent_id: String::new(),
            server: String::new(),
            online: false,
            current_task_id: String::new(),
            current_task_title: String::new(),
            spinner_text: String::new(),
            log: VecDeque::new(),
            spinner_frame: 0,
        }
    }
}

impl DaemonState {
    fn push_log(&mut self, s: impl Into<String>) {
        self.log.push_front(s.into());
        while self.log.len() > MAX_LOG {
            self.log.pop_back();
        }
    }

    fn apply(&mut self, event: TuiEvent) {
        match event {
            TuiEvent::AgentInfo { name, id, server } => {
                self.agent_name = name;
                self.agent_id = id;
                self.server = server;
            }
            TuiEvent::ServerStatus(online) => {
                let was = self.online;
                self.online = online;
                if online && !was {
                    let server = self.server.clone();
                    self.push_log(format!("↑ connected to {server}"));
                } else if !online && was {
                    self.push_log("↓ server unreachable, working offline");
                }
            }
            TuiEvent::Triaging { count } => {
                self.spinner_text = format!("📋 triaging {count} tasks…");
                self.current_task_id.clear();
                self.current_task_title.clear();
            }
            TuiEvent::TaskStart { id, title } => {
                self.push_log(format!("⚙ [{id}] {title}"));
                self.current_task_id = id.clone();
                self.current_task_title = title;
                self.spinner_text = format!("starting [{id}]…");
            }
            TuiEvent::SpinnerUpdate(text) => {
                self.spinner_text = text;
            }
            TuiEvent::TaskDone { id, status, chars } => {
                let icon = if status == "done" { "✓" } else { "✗" };
                self.push_log(format!("{icon} [{id}] {status} ({chars} chars)"));
                if self.current_task_id == id {
                    self.current_task_id.clear();
                    self.current_task_title.clear();
                    self.spinner_text.clear();
                }
            }
            TuiEvent::TaskSkip { id, reason } => {
                self.push_log(format!("⊘ [{id}] skipped: {reason}"));
            }
            TuiEvent::TaskBlock { id, reason } => {
                self.push_log(format!("⚡ [{id}] blocked: {reason}"));
            }
            TuiEvent::MsgRecv { from } => {
                self.push_log(format!("← [{from}]"));
            }
            TuiEvent::MsgSent { chars } => {
                self.push_log(format!("→ reply ({chars} chars)"));
            }
            TuiEvent::Log(s) => {
                self.push_log(s);
            }
            TuiEvent::Shutdown => {}
        }
    }
}

/// Spawn the TUI thread. Returns a sender for events.
/// Uses ratatui when stderr is a TTY, plain text otherwise.
pub fn spawn_tui() -> SyncSender<TuiEvent> {
    use std::io::IsTerminal;
    let is_tty = io::stderr().is_terminal();
    let (tx, rx) = mpsc::sync_channel::<TuiEvent>(512);
    if is_tty {
        std::thread::spawn(move || run_tui(rx));
    } else {
        std::thread::spawn(move || run_plain(rx));
    }
    tx
}

fn run_tui(rx: Receiver<TuiEvent>) {
    if enable_raw_mode().is_err() {
        run_plain(rx);
        return;
    }
    if execute!(io::stderr(), EnterAlternateScreen).is_err() {
        let _ = disable_raw_mode();
        run_plain(rx);
        return;
    }

    // Restore terminal on panic
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stderr(), LeaveAlternateScreen);
        original_hook(info);
    }));

    let backend = CrosstermBackend::new(io::stderr());
    let mut terminal = match Terminal::new(backend) {
        Ok(t) => t,
        Err(_) => {
            let _ = disable_raw_mode();
            let _ = execute!(io::stderr(), LeaveAlternateScreen);
            run_plain(rx);
            return;
        }
    };

    let mut state = DaemonState::default();

    loop {
        state.spinner_frame = state.spinner_frame.wrapping_add(1);

        let mut shutdown = false;
        loop {
            match rx.try_recv() {
                Ok(TuiEvent::Shutdown) => {
                    shutdown = true;
                    break;
                }
                Ok(e) => state.apply(e),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    shutdown = true;
                    break;
                }
            }
        }

        let _ = terminal.draw(|f| render(f, &state));

        if shutdown {
            break;
        }

        // poll for key events without blocking the render tick
        if event::poll(Duration::from_millis(TICK_MS)).unwrap_or(false) {
            if let Ok(Event::Key(key)) = event::read() {
                let quit = key.code == KeyCode::Char('q')
                    || key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL);
                if quit {
                    break;
                }
            }
        } else {
            std::thread::sleep(Duration::from_millis(0));
        }
    }

    // Capture log lines before clearing the screen
    let final_log: Vec<String> = state.log.iter().cloned().collect();

    let _ = disable_raw_mode();
    let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen);
    let _ = terminal.show_cursor();

    // Print recent log to stderr so the user can see what happened
    eprintln!("── punchclock shutdown ──");
    for line in final_log.iter().rev().take(30) {
        eprintln!("{line}");
    }
}

fn render(f: &mut ratatui::Frame, state: &DaemonState) {
    let area = f.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(4),
            Constraint::Min(3),
        ])
        .split(area);

    // ── header ────────────────────────────────────────────────────────────────
    let (status_dot, status_style) = if state.online {
        ("● ONLINE", Style::default().fg(Color::Green))
    } else {
        ("○ OFFLINE", Style::default().fg(Color::Red))
    };
    let header_line = if state.agent_name.is_empty() {
        Line::from(Span::raw("starting…"))
    } else {
        Line::from(vec![
            Span::styled(state.agent_name.clone(), Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(format!("  ({})", state.agent_id)),
            Span::raw("  │  "),
            Span::raw(state.server.clone()),
            Span::raw("  "),
            Span::styled(status_dot, status_style),
        ])
    };
    f.render_widget(
        Paragraph::new(header_line)
            .block(Block::default().borders(Borders::ALL).title(" punchclock ")),
        chunks[0],
    );

    // ── status ────────────────────────────────────────────────────────────────
    let spinner = SPINNER_FRAMES[state.spinner_frame % SPINNER_FRAMES.len()];
    let mut status_lines: Vec<Line> = Vec::new();
    if !state.spinner_text.is_empty() {
        status_lines.push(Line::from(vec![
            Span::styled(spinner, Style::default().fg(Color::Cyan)),
            Span::raw("  "),
            Span::raw(state.spinner_text.clone()),
        ]));
    }
    if !state.current_task_id.is_empty() {
        status_lines.push(Line::from(vec![
            Span::raw("   "),
            Span::styled(
                format!("[{}]", state.current_task_id),
                Style::default().fg(Color::Yellow),
            ),
            Span::raw("  "),
            Span::raw(state.current_task_title.clone()),
        ]));
    }
    if status_lines.is_empty() {
        status_lines.push(Line::from(Span::styled(
            "idle",
            Style::default().fg(Color::DarkGray),
        )));
    }
    f.render_widget(
        Paragraph::new(status_lines)
            .block(Block::default().borders(Borders::ALL).title(" status ")),
        chunks[1],
    );

    // ── log ───────────────────────────────────────────────────────────────────
    let log_lines: Vec<Line> = state
        .log
        .iter()
        .map(|s| {
            let style = if s.starts_with('✓') {
                Style::default().fg(Color::Green)
            } else if s.starts_with('✗') || s.starts_with("⚡") {
                Style::default().fg(Color::Red)
            } else if s.starts_with('⊘') {
                Style::default().fg(Color::DarkGray)
            } else if s.starts_with('←') || s.starts_with('→') {
                Style::default().fg(Color::Magenta)
            } else if s.starts_with('↑') {
                Style::default().fg(Color::Green)
            } else if s.starts_with('↓') {
                Style::default().fg(Color::Red)
            } else {
                Style::default()
            };
            Line::from(Span::styled(s.clone(), style))
        })
        .collect();
    f.render_widget(
        Paragraph::new(log_lines)
            .block(Block::default().borders(Borders::ALL).title(" log "))
            .wrap(Wrap { trim: false }),
        chunks[2],
    );
}

fn run_plain(rx: Receiver<TuiEvent>) {
    for event in rx {
        match event {
            TuiEvent::AgentInfo { name, id, server } => {
                eprintln!("agent    : {name} ({id})");
                eprintln!("server   : {server}");
                eprintln!("routing messages + tasks → claude CLI\n");
            }
            TuiEvent::ServerStatus(true) => {}
            TuiEvent::ServerStatus(false) => {}
            TuiEvent::Triaging { count } => eprintln!("📋 triaging {count} pending tasks…"),
            TuiEvent::TaskStart { id, title } => {
                eprintln!("⚙ task [{id}] {title}");
                eprintln!("  claude working on [{id}]…");
            }
            TuiEvent::SpinnerUpdate(_) => {}
            TuiEvent::TaskDone { id, status, chars } => {
                eprintln!("✓ task [{id}] {status} ({chars} chars)");
            }
            TuiEvent::TaskSkip { id, reason } => eprintln!("⊘ skipped [{id}]: {reason}"),
            TuiEvent::TaskBlock { id, reason } => eprintln!("⚡ blocked [{id}]: {reason}"),
            TuiEvent::MsgRecv { from } => eprintln!("← [{from}]"),
            TuiEvent::MsgSent { chars } => eprintln!("→ reply ({chars} chars)"),
            TuiEvent::Log(s) => eprintln!("{s}"),
            TuiEvent::Shutdown => {
                eprintln!("\nshutting down…");
                break;
            }
        }
    }
}
