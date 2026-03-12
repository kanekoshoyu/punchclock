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

// ── main ──────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
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
    }

    Ok(())
}
