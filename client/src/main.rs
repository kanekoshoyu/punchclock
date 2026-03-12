mod agent;
mod tui;

use clap::{Parser, Subcommand};
use punchclock_common::{
    BroadcastResponse, InboxResponse, RegisterResponse, TaskItem, TaskListResponse, TeamResponse,
};

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
    /// Register this agent with the server
    Register {
        name: String,
        description: String,
    },
    /// Send a heartbeat to stay online
    Heartbeat {
        /// Agent ID (defaults to .punchclock)
        agent_id: Option<String>,
    },
    /// List all online agents
    Team,
    /// Send a message to another agent
    Send {
        /// Sender agent ID (defaults to .punchclock)
        #[arg(long)]
        from: Option<String>,
        #[arg(long)]
        to: String,
        body: String,
    },
    /// Receive (and drain) your inbox
    Inbox {
        /// Agent ID (defaults to .punchclock)
        agent_id: Option<String>,
    },
    /// Poll inbox and print messages as they arrive
    Watch {
        /// Agent ID (defaults to .punchclock)
        agent_id: Option<String>,
        /// Poll interval in seconds
        #[arg(long, default_value = "5")]
        interval: u64,
    },
    /// Broadcast a message to all online agents
    Broadcast {
        /// Sender agent ID (defaults to .punchclock)
        #[arg(long)]
        from: Option<String>,
        body: String,
    },
    /// Manage tasks in an agent's queue
    Task {
        #[command(subcommand)]
        cmd: TaskCmd,
    },
    /// Manage the Claude agent for this git repo
    Agent {
        #[command(subcommand)]
        cmd: AgentCmd,
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
        Cmd::Register { name, description } => {
            let res: RegisterResponse = client
                .get(format!("{base}/register"))
                .query(&[("name", name), ("description", description)])
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            println!("registered  agent_id: {}", res.agent_id);
        }

        Cmd::Heartbeat { agent_id } => {
            let id = resolve_id(agent_id.as_ref())?;
            client
                .get(format!("{base}/heartbeat"))
                .query(&[("agent_id", &id)])
                .send()
                .await?
                .error_for_status()?;
            println!("heartbeat sent");
        }

        Cmd::Team => {
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
            client
                .get(format!("{base}/message/send"))
                .query(&[("to", to), ("from", &from), ("body", body)])
                .send()
                .await?
                .error_for_status()?;
            println!("sent to {to}");
        }

        Cmd::Inbox { agent_id } => {
            let id = resolve_id(agent_id.as_ref())?;
            let res: InboxResponse = client
                .get(format!("{base}/message/recv"))
                .query(&[("agent_id", &id)])
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            if res.messages.is_empty() {
                println!("inbox empty");
            } else {
                for m in &res.messages {
                    println!("[{}] from {}: {}", m.timestamp, m.from, m.body);
                }
            }
        }

        Cmd::Watch { agent_id, interval } => {
            let id = resolve_id(agent_id.as_ref())?;
            let period = tokio::time::Duration::from_secs(*interval);
            loop {
                let res: InboxResponse = client
                    .get(format!("{base}/message/recv"))
                    .query(&[("agent_id", &id)])
                    .send()
                    .await?
                    .error_for_status()?
                    .json()
                    .await?;
                for m in &res.messages {
                    println!("[{}] from {}: {}", m.timestamp, m.from, m.body);
                }
                tokio::time::sleep(period).await;
            }
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

        Cmd::Task { cmd } => match cmd {
            TaskCmd::Push { to, title, body } => {
                let to = resolve_id(to.as_ref())?;
                let item: TaskItem = client
                    .get(format!("{base}/task/push"))
                    .query(&[("agent_id", &to), ("title", title), ("body", body)])
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
            AgentCmd::Init => agent::init().await?,
            AgentCmd::Run => agent::run().await?,
            AgentCmd::Start => agent::start().await?,
            AgentCmd::Stop => agent::stop().await?,
            AgentCmd::Status => agent::status().await?,
            AgentCmd::Install => agent::install().await?,
            AgentCmd::Uninstall => agent::uninstall().await?,
            AgentCmd::Logs => agent::logs().await?,
        },
    }

    Ok(())
}
