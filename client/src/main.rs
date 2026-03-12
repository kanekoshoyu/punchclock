mod agent;

use clap::{Parser, Subcommand};
use serde::Deserialize;

const DEFAULT_SERVER: &str = "http://localhost:8421";

#[derive(Parser)]
#[command(name = "punchclock", about = "Punchclock agent orchestration client")]
struct Cli {
    /// Punchclock server base URL
    #[arg(long, env = "API_BASE_URL", default_value = DEFAULT_SERVER)]
    server: String,

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
    Heartbeat { agent_id: String },
    /// List all online agents
    Team,
    /// Send a message to another agent
    Send {
        #[arg(long)]
        from: String,
        #[arg(long)]
        to: String,
        body: String,
    },
    /// Receive (and drain) your inbox
    Inbox { agent_id: String },
    /// Poll inbox and print messages as they arrive
    Watch {
        agent_id: String,
        /// Poll interval in seconds
        #[arg(long, default_value = "5")]
        interval: u64,
    },
    /// Broadcast a message to all online agents
    Broadcast {
        #[arg(long)]
        from: String,
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
    /// Start the routing daemon (heartbeat + poll + forward to claude CLI)
    Run,
    /// Show whether this repo's agent is online
    Status,
}

#[derive(Subcommand)]
enum TaskCmd {
    /// Push a task onto an agent's queue
    Push {
        /// Target agent ID
        #[arg(long)]
        to: String,
        /// Short title
        title: String,
        /// Full task body (sent to Claude)
        body: String,
    },
    /// List all tasks for an agent
    List {
        agent_id: String,
    },
}

// ── response types (mirrors server) ──────────────────────────────────────────

#[derive(Deserialize)]
struct RegisterResponse {
    agent_id: String,
}

#[derive(Deserialize)]
struct AgentSummary {
    id: String,
    name: String,
    description: String,
}

#[derive(Deserialize)]
struct TeamResponse {
    agents: Vec<AgentSummary>,
}

#[derive(Deserialize)]
struct MessageItem {
    from: String,
    body: String,
    timestamp: String,
}

#[derive(Deserialize)]
struct InboxResponse {
    messages: Vec<MessageItem>,
}

#[derive(Deserialize)]
struct BroadcastResponse {
    delivered: usize,
}

#[derive(Deserialize)]
struct TaskItem {
    id: String,
    agent_id: String,
    title: String,
    status: String,
    created_at: String,
    started_at: Option<String>,
    finished_at: Option<String>,
    result: Option<String>,
}

#[derive(Deserialize)]
struct TaskListResponse {
    tasks: Vec<TaskItem>,
}

// ── main ──────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = dotenvy::from_filename(concat!(env!("CARGO_MANIFEST_DIR"), "/.env")); // ok if absent
    let cli = Cli::parse();
    let client = reqwest::Client::new();
    let base = cli.server.trim_end_matches('/');

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
            client
                .get(format!("{base}/heartbeat"))
                .query(&[("agent_id", agent_id)])
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
            client
                .get(format!("{base}/message/send"))
                .query(&[("to", to), ("from", from), ("body", body)])
                .send()
                .await?
                .error_for_status()?;
            println!("sent to {to}");
        }

        Cmd::Inbox { agent_id } => {
            let res: InboxResponse = client
                .get(format!("{base}/message/recv"))
                .query(&[("agent_id", agent_id)])
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
            let period = tokio::time::Duration::from_secs(*interval);
            loop {
                let res: InboxResponse = client
                    .get(format!("{base}/message/recv"))
                    .query(&[("agent_id", agent_id)])
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
            let res: BroadcastResponse = client
                .get(format!("{base}/message/broadcast"))
                .query(&[("from", from), ("body", body)])
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            println!("broadcast delivered to {} agent(s)", res.delivered);
        }

        Cmd::Task { cmd } => match cmd {
            TaskCmd::Push { to, title, body } => {
                let item: TaskItem = client
                    .get(format!("{base}/task/push"))
                    .query(&[("agent_id", to), ("title", title), ("body", body)])
                    .send()
                    .await?
                    .error_for_status()?
                    .json()
                    .await?;
                println!("task queued  id: {}  agent: {}", item.id, item.agent_id);
            }
            TaskCmd::List { agent_id } => {
                let res: TaskListResponse = client
                    .get(format!("{base}/task/list"))
                    .query(&[("agent_id", agent_id)])
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
            AgentCmd::Status => agent::status().await?,
        },
    }

    Ok(())
}
