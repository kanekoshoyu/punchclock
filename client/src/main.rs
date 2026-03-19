mod agent;
mod tui;
mod config;

use clap::{Parser, Subcommand};
use punchclock_common::{
    BroadcastResponse, MessageStatusListResponse, TaskItem, TaskListResponse, TeamResponse,
};
use std::path::PathBuf;
use anyhow::Context;

const DEFAULT_SERVER: &str = "http://localhost:8421";

#[derive(Parser)]
#[command(name = "punchclock", about = "Punchclock agent orchestration client")]
struct Cli {
    /// Punchclock server base URL (overrides .punchclock)
    #[arg(long, env = "API_BASE_URL")]
    server: Option<String>,

    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    #[command(subcommand_help_heading = "Server")]
    /// List all online agents
    Ps,
    #[command(subcommand_help_heading = "Server")]
    /// Send a message to a named agent
    Send {
        /// Sender agent ID (defaults to .punchclock)
        #[arg(long)]
        from: Option<String>,
        #[arg(long)]
        to: String,
        body: String,
    },
    #[command(subcommand_help_heading = "Server")]
    /// Broadcast a message to all online agents
    Broadcast {
        /// Sender agent ID (defaults to .punchclock)
        #[arg(long)]
        from: Option<String>,
        body: String,
    },
    #[command(subcommand_help_heading = "Server")]
    /// Manage messages in an agent's inbox
    Message {
        #[command(subcommand)]
        cmd: MessageCmd,
    },
    #[command(subcommand_help_heading = "Server")]
    /// Manage tasks in an agent's queue
    Task {
        #[command(subcommand)]
        cmd: TaskCmd,
    },
    /// Manage the Claude agent for this git repo (DEPRECATED — use top-level commands instead)
    #[command(hide = true)]
    Agent {
        #[command(subcommand)]
        cmd: AgentCmd,
    },
    #[command(subcommand_help_heading = "Local")]
    /// Add a repo to the managed set
    Add {
        path: PathBuf,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        description: Option<String>,
        #[arg(long)]
        claude_flags: Option<String>,
    },
    #[command(subcommand_help_heading = "Local")]
    /// Remove a repo from the managed set
    Rm {
        name: String,
    },
    #[command(subcommand_help_heading = "Local")]
    /// List all registered repos
    Ls,
    #[command(subcommand_help_heading = "Local")]
    /// Pause an agent (disable without removing)
    Pause {
        name: String,
    },
    #[command(subcommand_help_heading = "Local")]
    /// Resume a paused agent
    Resume {
        name: String,
    },
    #[command(subcommand_help_heading = "Local")]
    /// Start the multi-repo daemon
    Up {
        #[arg(long)]
        daemon: bool,
    },
    #[command(subcommand_help_heading = "Local")]
    /// Stop the multi-repo daemon
    Down,
    #[command(subcommand_help_heading = "Local")]
    /// Import legacy .punchclock configs from repos
    Import {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        paths: Vec<PathBuf>,
    },
}

#[derive(Subcommand)]
enum AgentCmd {
    /// Register this repo as an agent and write .punchclock/agent.toml
    Init,
    /// Run the daemon in the foreground (heartbeat + poll + forward to claude CLI)
    Run,
    /// Start the daemon in the background (writes .punchclock/daemon.pid)
    Start,
    /// Stop a running background daemon
    Stop,
    /// Show whether this repo's agent is online
    Status,
    /// Register the daemon as an OS-level service (launchd / systemd)
    Install,
    /// Unregister and remove the OS-level service unit
    Uninstall,
    /// Tail the daemon log file
    Logs,
}

#[derive(Subcommand)]
enum TaskCmd {
    /// Push a task onto an agent's queue
    Push {
        /// Target agent ID (defaults to .punchclock)
        #[arg(long)]
        to: Option<String>,
        /// Short title
        title: String,
        /// Full task body (sent to Claude)
        body: String,
    },
    /// List all tasks for an agent
    List {
        /// Agent ID (defaults to .punchclock)
        agent_id: Option<String>,
    },
}

#[derive(Subcommand)]
enum MessageCmd {
    /// Acknowledge a message as received
    Ack {
        /// Agent ID (defaults to .punchclock)
        #[arg(long)]
        agent_id: Option<String>,
        /// Message ID to acknowledge
        message_id: String,
    },
    /// Check ack status of all messages for an agent
    Status {
        /// Agent ID (defaults to .punchclock)
        agent_id: Option<String>,
    },
}


// ── helpers ───────────────────────────────────────────────────────────────────

async fn resolve_agent_to_id(client: &reqwest::Client, base: &str, name_or_id: &str) -> anyhow::Result<String> {
    // Check if it's already a UUID (36 chars with hyphens)
    if name_or_id.len() == 36 && name_or_id.chars().filter(|c| *c == '-').count() == 4 {
        return Ok(name_or_id.to_string());
    }

    // Query team list and find by name
    let res: TeamResponse = client
        .get(format!("{base}/team"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    res.agents
        .iter()
        .find(|a| a.name == name_or_id)
        .map(|a| a.id.clone())
        .ok_or_else(|| anyhow::anyhow!("agent \"{}\" not found", name_or_id))
}

// ── main ──────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = dotenvy::from_filename(concat!(env!("CARGO_MANIFEST_DIR"), "/.env")); // ok if absent
    let cli = Cli::parse();
    let client = reqwest::Client::new();

    // load .punchclock if present — used as fallback for agent_id, server, from
    let local = agent::AgentConfig::load_optional();

    let base = cli
        .server
        .as_deref()
        .map(str::to_string)
        .or_else(|| local.as_ref().map(|c| c.server.clone()))
        .unwrap_or_else(|| DEFAULT_SERVER.to_string());
    let base = base.trim_end_matches('/');

    // resolve agent_id: provided arg → .punchclock → error
    let resolve_id = |provided: Option<&String>| -> anyhow::Result<String> {
        provided
            .cloned()
            .or_else(|| local.as_ref().and_then(|c| c.agent_id.clone()))
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "agent_id required — provide it as an argument or run `punchclock agent run`"
                )
            })
    };

    // resolve --from: same fallback as agent_id
    let resolve_from = |provided: Option<&String>| -> anyhow::Result<String> {
        provided
            .cloned()
            .or_else(|| local.as_ref().and_then(|c| c.agent_id.clone()))
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "--from required — provide it or run `punchclock agent run`"
                )
            })
    };

    match &cli.command {
        Cmd::Ps => {
            let res: TeamResponse = client
                .get(format!("{base}/team"))
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            if res.agents.is_empty() {
                println!("no agents online");
            } else {
                println!("{:<36}  {:<20}  description", "id", "name");
                println!("{}", "-".repeat(80));
                for a in &res.agents {
                    println!("{:<36}  {:<20}  {}", a.id, a.name, a.description);
                }
            }
        }

        Cmd::Send { from, to, body } => {
            let from = resolve_from(from.as_ref())?;
            let to_id = resolve_agent_to_id(&client, base, to).await?;
            client
                .get(format!("{base}/message/send"))
                .query(&[("to", &to_id), ("from", &from), ("body", body)])
                .send()
                .await?
                .error_for_status()?;
            println!("sent to {to}");
        }

        Cmd::Broadcast { from, body } => {
            let from = resolve_from(from.as_ref())?;
            let res: BroadcastResponse = client
                .get(format!("{base}/message/broadcast"))
                .query(&[("from", &from), ("body", body)])
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            println!("broadcast delivered to {} agent(s)", res.delivered);
        }

        Cmd::Message { cmd } => match cmd {
            MessageCmd::Ack { agent_id, message_id } => {
                let id = resolve_id(agent_id.as_ref())?;
                client
                    .post(format!("{base}/message/ack"))
                    .query(&[("agent_id", &id), ("message_id", &message_id)])
                    .send()
                    .await?
                    .error_for_status()?;
                println!("message {} acknowledged", message_id);
            }
            MessageCmd::Status { agent_id } => {
                let id = resolve_id(agent_id.as_ref())?;
                let res: MessageStatusListResponse = client
                    .get(format!("{base}/message/status"))
                    .query(&[("agent_id", &id)])
                    .send()
                    .await?
                    .error_for_status()?
                    .json()
                    .await?;
                if res.messages.is_empty() {
                    println!("no messages");
                } else {
                    println!("{:<36}  acked", "id");
                    println!("{}", "-".repeat(50));
                    for m in &res.messages {
                        let acked = if m.acked { "yes" } else { "no" };
                        println!("{:<36}  {}", m.id, acked);
                    }
                }
            }
        },

        Cmd::Task { cmd } => match cmd {
            TaskCmd::Push { to, title, body } => {
                let to_id = match to {
                    Some(name_or_id) => resolve_agent_to_id(&client, base, name_or_id).await?,
                    None => resolve_id(None)?,
                };
                let item: TaskItem = client
                    .get(format!("{base}/task/push"))
                    .query(&[("agent_id", &to_id), ("title", title), ("body", body)])
                    .send()
                    .await?
                    .error_for_status()?
                    .json()
                    .await?;
                println!("task queued  id: {}  agent: {}", item.id, item.agent_id);
            }
            TaskCmd::List { agent_id } => {
                let id = resolve_id(agent_id.as_ref())?;
                let res: TaskListResponse = client
                    .get(format!("{base}/task/list"))
                    .query(&[("agent_id", &id)])
                    .send()
                    .await?
                    .error_for_status()?
                    .json()
                    .await?;
                if res.tasks.is_empty() {
                    println!("no tasks");
                } else {
                    println!("{:<36}  {:<12}  {:<24}  title", "id", "status", "created");
                    println!("{}", "-".repeat(90));
                    for t in &res.tasks {
                        println!("{:<36}  {:<12}  {:<24}  {}", t.id, t.status, t.created_at, t.title);
                    }
                }
            }
        },

        Cmd::Agent { cmd } => match cmd {
            AgentCmd::Init => {
                eprintln!("⚠️  WARNING: 'punchclock agent init' is deprecated — use 'punchclock add <path>' instead");
                agent::init().await?
            }
            AgentCmd::Run => {
                eprintln!("⚠️  WARNING: 'punchclock agent run' is deprecated — use 'punchclock up' instead");
                agent::run().await?
            }
            AgentCmd::Start => {
                eprintln!("⚠️  WARNING: 'punchclock agent start' is deprecated — use 'punchclock up --daemon' instead");
                agent::start().await?
            }
            AgentCmd::Stop => {
                eprintln!("⚠️  WARNING: 'punchclock agent stop' is deprecated — use 'punchclock down' instead");
                agent::stop().await?
            }
            AgentCmd::Status => {
                eprintln!("⚠️  WARNING: 'punchclock agent status' is deprecated — use 'punchclock ls' instead");
                agent::status().await?
            }
            AgentCmd::Install => {
                eprintln!("⚠️  WARNING: 'punchclock agent install' is deprecated — use 'punchclock up --daemon' instead");
                agent::install().await?
            }
            AgentCmd::Uninstall => {
                eprintln!("⚠️  WARNING: 'punchclock agent uninstall' is deprecated");
                agent::uninstall().await?
            }
            AgentCmd::Logs => {
                eprintln!("⚠️  WARNING: 'punchclock agent logs' is deprecated");
                agent::logs().await?
            }
        },

        Cmd::Add {
            path,
            name,
            description,
            claude_flags,
        } => {
            let abs_path = std::fs::canonicalize(&path)
                .map_err(|_| anyhow::anyhow!("path does not exist: {:?}", path))?;

            if !abs_path.join(".git").exists() {
                anyhow::bail!("not a git repo: {:?}", abs_path);
            }

            let agent_name = name.clone().unwrap_or_else(|| {
                abs_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unnamed")
                    .to_string()
            });

            let mut repos = config::load_repos()?;
            if repos.contains_key(&agent_name) {
                anyhow::bail!(
                    "agent \"{}\" already exists — use --name to pick a different one",
                    agent_name
                );
            }

            repos.insert(
                agent_name.clone(),
                config::RepoEntry {
                    path: abs_path.clone(),
                    description: description.clone().unwrap_or_default(),
                    enabled: true,
                    claude_flags: claude_flags.clone().unwrap_or_default(),
                },
            );

            config::save_repos(&repos)?;
            println!("added agent \"{}\" → {}", agent_name, abs_path.display());
        }

        Cmd::Rm { name } => {
            let mut repos = config::load_repos()?;
            if repos.remove(name).is_none() {
                anyhow::bail!("agent \"{}\" not found", name);
            }
            config::save_repos(&repos)?;
            println!("removed agent \"{}\"", name);
        }

        Cmd::Ls => {
            let repos = config::load_repos()?;
            if repos.is_empty() {
                println!("no repos registered — use \"punchclock add <path>\" to add one");
            } else {
                println!("{:<30}  {:<8}  path", "name", "enabled");
                println!("{}", "-".repeat(80));
                for (name, entry) in &repos {
                    let enabled = if entry.enabled { "yes" } else { "no" };
                    println!("{:<30}  {:<8}  {}", name, enabled, entry.path.display());
                }
            }
        }

        Cmd::Pause { name } => {
            let mut repos = config::load_repos()?;
            match repos.get_mut(name) {
                Some(entry) => {
                    entry.enabled = false;
                    config::save_repos(&repos)?;
                    println!("paused \"{}\"", name);
                }
                None => anyhow::bail!("agent \"{}\" not found", name),
            }
        }

        Cmd::Resume { name } => {
            let mut repos = config::load_repos()?;
            match repos.get_mut(name) {
                Some(entry) => {
                    entry.enabled = true;
                    config::save_repos(&repos)?;
                    println!("resumed \"{}\"", name);
                }
                None => anyhow::bail!("agent \"{}\" not found", name),
            }
        }

        Cmd::Up { daemon } => {
            agent::up(!daemon).await?;
        }

        Cmd::Down => {
            let pid_path = config::config_dir().join("daemon.pid");
            let pid_str = std::fs::read_to_string(&pid_path)
                .map_err(|_| anyhow::anyhow!("daemon not running (no PID file)"))?;
            let pid: u32 = pid_str.trim().parse()
                .context("daemon PID file is corrupt")?;

            let result = std::process::Command::new("kill")
                .args(["-TERM", &pid.to_string()])
                .output();

            match result {
                Ok(o) if o.status.success() => {
                    println!("sent SIGTERM to PID {}", pid);
                    let _ = std::fs::remove_file(&pid_path);
                }
                Ok(_) => {
                    let _ = std::fs::remove_file(&pid_path);
                    println!("daemon was not running (stale PID {}) — cleaned up", pid);
                }
                Err(e) => anyhow::bail!("failed to send signal: {}", e),
            }
        }

        Cmd::Import { paths } => {
            let mut repos = config::load_repos()?;
            let mut imported = 0;
            let mut skipped = 0;

            // If no paths provided, scan default locations
            let scan_paths = if paths.is_empty() {
                let mut defaults = vec![
                    dirs::home_dir().unwrap_or_default(),
                    dirs::home_dir().map(|h| h.join("Documents")).unwrap_or_default(),
                    dirs::home_dir().map(|h| h.join("Projects")).unwrap_or_default(),
                ];
                defaults.retain(|p| !p.as_os_str().is_empty());
                defaults
            } else {
                paths.clone()
            };

            for base_path in scan_paths {
                // Scan one level deep for .punchclock files
                if let Ok(entries) = std::fs::read_dir(&base_path) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.is_dir() {
                            let punchclock_file = path.join(".punchclock");
                            if punchclock_file.exists() {
                                // Try to parse the .punchclock file
                                if let Ok(cfg) = agent::AgentConfig::load(&path) {
                                    if repos.contains_key(&cfg.name) {
                                        skipped += 1;
                                    } else {
                                        repos.insert(
                                            cfg.name.clone(),
                                            config::RepoEntry {
                                                path: path.clone(),
                                                description: cfg.description,
                                                enabled: true,
                                                claude_flags: cfg.claude_flags,
                                            },
                                        );
                                        imported += 1;
                                    }
                                }
                            }
                        }
                    }
                }
            }

            config::save_repos(&repos)?;
            println!("imported {} agent(s), skipped {} already registered", imported, skipped);
        }
    }

    Ok(())
}
