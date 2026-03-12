use anyhow::{bail, Context};
use dialoguer::{theme::ColorfulTheme, Input};
use serde::{Deserialize, Serialize};
use serde_json;
use std::{collections::HashMap, path::{Path, PathBuf}, process::Stdio};
use tokio::process::Command;
use tokio::time::{interval, Duration};

// ── config ────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct AgentConfig {
    pub agent_id: String,
    pub name: String,
    pub description: String,
    pub server: String,
    #[serde(default)]
    pub claude_flags: String,
}

impl AgentConfig {
    pub fn load(repo_root: &Path) -> anyhow::Result<Self> {
        let path = repo_root.join(".punchclock/agent.toml");
        let text = std::fs::read_to_string(&path).with_context(|| {
            format!("cannot read {path:?} — run `punchclock agent init` first")
        })?;
        toml::from_str(&text).context("invalid agent.toml")
    }

    pub fn save(&self, repo_root: &Path) -> anyhow::Result<()> {
        let dir = repo_root.join(".punchclock");
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join("agent.toml"), toml::to_string_pretty(self)?)?;
        Ok(())
    }
}

// ── template ──────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct Template {
    questions: Vec<Question>,
}

#[derive(Debug, Deserialize)]
struct Question {
    key: String,
    label: String,
    default: String,
}

const BUILTIN_TEMPLATE: &str = r#"
name: default
questions:
  - key: agent_name
    label: "Agent name"
    default: "{{repo_name}}"
  - key: description
    label: "Description"
    default: "Claude agent for {{repo_name}}"
  - key: server
    label: "Server URL"
    default: "http://localhost:8421"
  - key: claude_flags
    label: "Extra claude flags (e.g. --allowedTools Edit,Write,Bash)"
    default: ""
"#;

fn load_template(repo_root: &Path) -> anyhow::Result<Template> {
    let repo_tmpl = repo_root.join(".punchclock/template.yaml");
    if repo_tmpl.exists() {
        return Ok(serde_yaml::from_str(&std::fs::read_to_string(repo_tmpl)?)?);
    }
    if let Some(home) = dirs::home_dir() {
        let user_tmpl = home.join(".punchclock/templates/default.yaml");
        if user_tmpl.exists() {
            return Ok(serde_yaml::from_str(&std::fs::read_to_string(user_tmpl)?)?);
        }
    }
    Ok(serde_yaml::from_str(BUILTIN_TEMPLATE)?)
}

fn interpolate(s: &str, vars: &[(&str, &str)]) -> String {
    let mut out = s.to_string();
    for (k, v) in vars {
        out = out.replace(&format!("{{{{{k}}}}}"), v);
    }
    out
}

// ── git helpers ───────────────────────────────────────────────────────────────

pub fn find_repo_root() -> anyhow::Result<PathBuf> {
    let mut dir = std::env::current_dir()?;
    loop {
        if dir.join(".git").exists() {
            return Ok(dir);
        }
        if !dir.pop() {
            bail!("not inside a git repository");
        }
    }
}

fn repo_name(root: &Path) -> String {
    if let Ok(out) = std::process::Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(root)
        .output()
    {
        let url = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !url.is_empty() {
            let stripped = url.trim_end_matches(".git");
            if let Some(part) = stripped.rsplit('/').next() {
                if !part.is_empty() {
                    return part.to_string();
                }
            }
        }
    }
    root.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unknown".to_string())
}

// ── server response types ─────────────────────────────────────────────────────

#[derive(Deserialize)]
struct RegisterResponse {
    agent_id: String,
}

#[derive(Deserialize)]
struct TeamResponse {
    agents: Vec<AgentSummary>,
}

#[derive(Deserialize)]
struct AgentSummary {
    id: String,
    name: String,
}

#[derive(Deserialize)]
struct InboxResponse {
    messages: Vec<MessageItem>,
}

#[derive(Deserialize)]
struct MessageItem {
    from: String,
    body: String,
}

// ── init ──────────────────────────────────────────────────────────────────────

pub async fn init() -> anyhow::Result<()> {
    let root = find_repo_root()?;
    let rname = repo_name(&root);
    let vars = [("repo_name", rname.as_str())];
    let template = load_template(&root)?;
    let theme = ColorfulTheme::default();

    println!("Setting up punchclock agent for: {rname}\n");

    let mut answers: HashMap<String, String> = HashMap::new();
    for q in &template.questions {
        let default = interpolate(&q.default, &vars);
        let answer: String = Input::with_theme(&theme)
            .with_prompt(&q.label)
            .default(default)
            .interact_text()?;
        answers.insert(q.key.clone(), answer);
    }

    let name = answers.get("agent_name").cloned().unwrap_or(rname);
    let description = answers.get("description").cloned().unwrap_or_default();
    let server = answers
        .get("server")
        .cloned()
        .unwrap_or_else(|| "http://localhost:8421".to_string());
    let claude_flags = answers.get("claude_flags").cloned().unwrap_or_default();

    print!("\nRegistering with server at {server}... ");
    let client = reqwest::Client::new();
    let root_str = root.to_string_lossy().to_string();
    let res: RegisterResponse = client
        .get(format!("{}/register", server.trim_end_matches('/')))
        .query(&[("name", &name), ("description", &description), ("repo_path", &root_str)])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    println!("done");

    let config = AgentConfig {
        agent_id: res.agent_id.clone(),
        name,
        description,
        server,
        claude_flags,
    };
    config.save(&root)?;

    println!("\nagent_id : {}", res.agent_id);
    println!("config   : .punchclock/agent.toml");
    println!("\nRun `punchclock agent run` to start the daemon.");
    Ok(())
}

// ── run ───────────────────────────────────────────────────────────────────────

pub async fn run() -> anyhow::Result<()> {
    let root = find_repo_root()?;
    let config = AgentConfig::load(&root)?;
    let base = config.server.trim_end_matches('/').to_string();
    let agent_id = config.agent_id.clone();
    let claude_flags = config.claude_flags.clone();
    let root_str = root.to_string_lossy().to_string();

    eprintln!("agent    : {} ({})", config.name, agent_id);
    eprintln!("server   : {base}");
    eprintln!("routing messages + tasks → claude CLI\n");

    // background heartbeat
    let hb_base = base.clone();
    let hb_id = agent_id.clone();
    let hb_name = config.name.clone();
    let hb_desc = config.description.clone();
    let hb_root = root_str.clone();
    tokio::spawn(async move {
        let client = reqwest::Client::new();
        let mut ticker = interval(Duration::from_secs(15));
        loop {
            ticker.tick().await;
            let _ = client
                .get(format!("{hb_base}/heartbeat"))
                .query(&[
                    ("agent_id", hb_id.as_str()),
                    ("name", hb_name.as_str()),
                    ("description", hb_desc.as_str()),
                    ("repo_path", hb_root.as_str()),
                ])
                .send()
                .await;
        }
    });

    // background task loop
    let task_base = base.clone();
    let task_id = agent_id.clone();
    let task_flags = claude_flags.clone();
    let task_root = root_str.clone();
    tokio::spawn(async move {
        let client = reqwest::Client::new();
        let mut ticker = interval(Duration::from_secs(5));
        let todo_dir = std::path::Path::new(&task_root).join(".task/todo");
        let done_dir = std::path::Path::new(&task_root).join(".task/done");
        let _ = std::fs::create_dir_all(&todo_dir);
        let _ = std::fs::create_dir_all(&done_dir);
        loop {
            ticker.tick().await;
            // claim next queued task
            let resp = client
                .get(format!("{task_base}/task/claim"))
                .query(&[("agent_id", &task_id)])
                .send()
                .await;
            let task: serde_json::Value = match resp {
                Ok(r) if r.status().is_success() => match r.json().await {
                    Ok(v) => v,
                    Err(_) => continue,
                },
                _ => continue,
            };
            let tid = match task["id"].as_str() {
                Some(s) => s.to_string(),
                None => continue,
            };
            let body = task["body"].as_str().unwrap_or("").to_string();
            let title = task["title"].as_str().unwrap_or("").to_string();
            eprintln!("⚙ task [{tid}] {title}");

            // write task file to .task/todo/<id>.md
            let todo_file = todo_dir.join(format!("{tid}.md"));
            let task_content = format!("# {title}\n\n{body}\n");
            let _ = std::fs::write(&todo_file, &task_content);

            // run claude with the task body as the prompt
            let reply = route_to_claude(&body, &task_flags, &task_root).await;
            let status = if reply.starts_with("[claude exited") || reply.starts_with("[failed") {
                "failed"
            } else {
                "done"
            };
            eprintln!("✓ task [{tid}] {status} ({} chars)", reply.len());

            // git mv todo → done, append result
            let done_file = done_dir.join(format!("{tid}.md"));
            let _ = std::process::Command::new("git")
                .args(["mv", &todo_file.to_string_lossy(), &done_file.to_string_lossy()])
                .current_dir(&task_root)
                .output();
            // append result to the done file
            if let Ok(mut content) = std::fs::read_to_string(&done_file) {
                content.push_str(&format!("\n## Result\n\n{reply}\n"));
                let _ = std::fs::write(&done_file, content);
            }

            let _ = client
                .get(format!("{task_base}/task/finish"))
                .query(&[("task_id", &tid), ("status", &status.to_string()), ("result", &reply)])
                .send()
                .await;
        }
    });

    // message poll loop
    let client = reqwest::Client::new();
    let mut poll = interval(Duration::from_secs(5));
    loop {
        poll.tick().await;

        let inbox: InboxResponse = match client
            .get(format!("{base}/message/recv"))
            .query(&[("agent_id", &agent_id)])
            .send()
            .await
            .and_then(|r| r.error_for_status())
        {
            Ok(r) => match r.json().await {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("parse error: {e}");
                    continue;
                }
            },
            Err(e) => {
                eprintln!("recv error: {e}");
                continue;
            }
        };

        for msg in inbox.messages {
            eprintln!("← [{}] {}", msg.from, msg.body);
            let reply = route_to_claude(&msg.body, &claude_flags, &root_str).await;
            eprintln!("→ reply ({} chars)", reply.len());
            let _ = client
                .get(format!("{base}/message/send"))
                .query(&[("to", &msg.from), ("from", &agent_id), ("body", &reply)])
                .send()
                .await;
        }
    }
}

async fn route_to_claude(body: &str, flags: &str, cwd: &str) -> String {
    let mut cmd = Command::new("claude");
    cmd.arg("-p").arg(body);
    cmd.current_dir(cwd);
    cmd.env_remove("CLAUDECODE");
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::null());
    for flag in flags.split_whitespace().filter(|s| !s.is_empty()) {
        cmd.arg(flag);
    }
    match cmd.output().await {
        Ok(out) if out.status.success() => {
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        }
        Ok(out) => {
            let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
            format!("[claude exited {}] {err}", out.status)
        }
        Err(e) => format!("[failed to spawn claude: {e}]"),
    }
}

// ── status ────────────────────────────────────────────────────────────────────

pub async fn status() -> anyhow::Result<()> {
    let root = find_repo_root()?;
    let config = AgentConfig::load(&root)?;
    let base = config.server.trim_end_matches('/');
    let client = reqwest::Client::new();

    let res: TeamResponse = client
        .get(format!("{base}/team"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    match res.agents.iter().find(|a| a.id == config.agent_id) {
        Some(a) => println!("ONLINE   {}  {}", a.id, a.name),
        None => println!(
            "OFFLINE  {} — not on team (server may have restarted, run `agent init` again)",
            config.agent_id
        ),
    }
    Ok(())
}
