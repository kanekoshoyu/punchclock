use anyhow::{bail, Context};
use dialoguer::{theme::ColorfulTheme, Input};
use serde::{Deserialize, Serialize};
use serde_json;
use std::{collections::HashMap, path::{Path, PathBuf}, process::Stdio};
use tokio::process::Command;
use tokio::time::{interval, Duration};

// ── PID file ──────────────────────────────────────────────────────────────────

fn runtime_dir(repo_root: &Path) -> PathBuf {
    let hash = format!("{:x}", md5_path(repo_root));
    std::env::temp_dir().join(format!("punchclock-{hash}"))
}

fn md5_path(p: &Path) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    p.hash(&mut h);
    h.finish()
}

fn pid_path(repo_root: &Path) -> PathBuf {
    runtime_dir(repo_root).join("daemon.pid")
}

fn write_pid(repo_root: &Path) -> anyhow::Result<()> {
    let dir = runtime_dir(repo_root);
    std::fs::create_dir_all(&dir)?;
    std::fs::write(pid_path(repo_root), std::process::id().to_string())?;
    Ok(())
}

fn remove_pid(repo_root: &Path) {
    let _ = std::fs::remove_file(pid_path(repo_root));
}

fn read_pid(repo_root: &Path) -> anyhow::Result<u32> {
    let s = std::fs::read_to_string(pid_path(repo_root))
        .context("daemon not running (try `punchclock agent start`)")?;
    s.trim().parse::<u32>().context("daemon PID file is corrupt")
}

// ── config ────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct AgentConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    pub name: String,
    pub description: String,
    pub server: String,
    #[serde(default)]
    pub claude_flags: String,
}

impl AgentConfig {
    pub fn load(repo_root: &Path) -> anyhow::Result<Self> {
        let path = repo_root.join(".punchclock");
        let text = std::fs::read_to_string(&path).with_context(|| {
            format!("cannot read {path:?} — run `punchclock agent init` first")
        })?;
        toml::from_str(&text).context("invalid .punchclock")
    }

    /// Load config from the nearest git repo root, returning None if not found.
    pub fn load_optional() -> Option<Self> {
        let root = find_repo_root().ok()?;
        Self::load(&root).ok()
    }

    pub fn save(&self, repo_root: &Path) -> anyhow::Result<()> {
        std::fs::write(repo_root.join(".punchclock"), toml::to_string_pretty(self)?)?;
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

fn load_template() -> anyhow::Result<Template> {
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
    let template = load_template()?;
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

    let config = AgentConfig {
        agent_id: None,
        name,
        description,
        server,
        claude_flags,
    };
    config.save(&root)?;

    // ensure .punchclock is gitignored
    let gitignore_path = root.join(".gitignore");
    let existing = std::fs::read_to_string(&gitignore_path).unwrap_or_default();
    if !existing.lines().any(|l| l.trim() == ".punchclock") {
        let mut content = existing;
        if !content.ends_with('\n') && !content.is_empty() {
            content.push('\n');
        }
        content.push_str(".punchclock\n");
        std::fs::write(&gitignore_path, content)?;
    }

    println!("\nconfig   : .punchclock");
    println!("\nRun `punchclock agent run` to register and start the daemon.");
    Ok(())
}

// ── run ───────────────────────────────────────────────────────────────────────

pub async fn run() -> anyhow::Result<()> {
    let root = find_repo_root()?;
    let mut config = AgentConfig::load(&root)?;
    let base = config.server.trim_end_matches('/').to_string();

    // register on first run if no agent_id yet
    if config.agent_id.is_none() {
        eprint!("registering with {}... ", base);
        let client = reqwest::Client::new();
        let root_str = root.to_string_lossy().to_string();
        let res: RegisterResponse = client
            .get(format!("{base}/register"))
            .query(&[
                ("name", config.name.as_str()),
                ("description", config.description.as_str()),
                ("repo_path", root_str.as_str()),
            ])
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        eprintln!("done  agent_id: {}", res.agent_id);
        config.agent_id = Some(res.agent_id);
        config.save(&root)?;
    }

    let agent_id = config.agent_id.clone().unwrap();
    let claude_flags = config.claude_flags.clone();
    let root_str = root.to_string_lossy().to_string();

    eprintln!("agent    : {} ({})", config.name, agent_id);
    eprintln!("server   : {base}");
    eprintln!("routing messages + tasks → claude CLI\n");

    write_pid(&root)?;
    let root_for_cleanup = root.clone();
    // remove PID file on exit via a drop guard
    struct PidGuard(PathBuf);
    impl Drop for PidGuard { fn drop(&mut self) { remove_pid(&self.0); } }
    let _pid_guard = PidGuard(root_for_cleanup);

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
        let blocked_dir = std::path::Path::new(&task_root).join(".task/blocked");
        let _ = std::fs::create_dir_all(&todo_dir);
        let _ = std::fs::create_dir_all(&done_dir);
        let _ = std::fs::create_dir_all(&blocked_dir);
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

            // detect BLOCKED: <reason> as the first line of output
            let (status, dest_dir, section_heading) =
                if let Some(reason) = reply.strip_prefix("BLOCKED:") {
                    ("blocked", &blocked_dir, "Blocked")
                } else if reply.starts_with("[claude exited") || reply.starts_with("[failed") {
                    ("failed", &done_dir, "Result")
                } else {
                    ("done", &done_dir, "Result")
                };
            eprintln!("✓ task [{tid}] {status} ({} chars)", reply.len());

            // git mv todo → dest (.task/done/ or .task/blocked/)
            let dest_file = dest_dir.join(format!("{tid}.md"));
            let _ = std::process::Command::new("git")
                .args(["mv", &todo_file.to_string_lossy(), &dest_file.to_string_lossy()])
                .current_dir(&task_root)
                .output();
            // append result/reason to the dest file
            if let Ok(mut content) = std::fs::read_to_string(&dest_file) {
                content.push_str(&format!("\n## {section_heading}\n\n{reply}\n"));
                let _ = std::fs::write(&dest_file, content);
            }

            // notify server of blocked status with the reason
            if status == "blocked" {
                let reason = reply.strip_prefix("BLOCKED:").unwrap_or(&reply).trim().to_string();
                let _ = client
                    .get(format!("{task_base}/task/block"))
                    .query(&[("task_id", &tid), ("reason", &reason)])
                    .send()
                    .await;
                continue;
            }

            let _ = client
                .get(format!("{task_base}/task/finish"))
                .query(&[("task_id", &tid), ("status", &status.to_string()), ("result", &reply)])
                .send()
                .await;
        }
    });

    // message poll loop — exits cleanly on SIGINT or SIGTERM
    let client = reqwest::Client::new();
    let mut poll = interval(Duration::from_secs(5));
    loop {
        tokio::select! {
            _ = poll.tick() => {}
            _ = tokio::signal::ctrl_c() => {
                eprintln!("\nshutting down…");
                break;
            }
        }

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
    Ok(())
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

// ── start / stop ──────────────────────────────────────────────────────────────

pub async fn start() -> anyhow::Result<()> {
    let root = find_repo_root()?;

    // refuse to double-start
    if let Ok(pid) = read_pid(&root) {
        // check if the process is actually alive
        let alive = std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if alive {
            bail!("daemon already running (PID {pid}) — use `punchclock agent stop` first");
        }
        // stale PID file — remove and proceed
        remove_pid(&root);
    }

    let exe = std::env::current_exe().context("cannot determine current executable path")?;
    let log_path = runtime_dir(&root).join("daemon.log");

    let log_file = std::fs::OpenOptions::new()
        .create(true).append(true)
        .open(&log_path)?;

    std::process::Command::new(&exe)
        .args(["agent", "run"])
        .current_dir(&root)
        .stdin(std::process::Stdio::null())
        .stdout(log_file.try_clone()?)
        .stderr(log_file)
        .spawn()
        .context("failed to spawn daemon")?;

    // brief pause so the daemon can write its PID file
    tokio::time::sleep(Duration::from_millis(300)).await;

    match read_pid(&root) {
        Ok(pid) => println!("daemon started  PID {pid}  log → {}", log_path.display()),
        Err(_)  => println!("daemon spawned  log → {}", log_path.display()),
    }
    Ok(())
}

pub async fn stop() -> anyhow::Result<()> {
    let root = find_repo_root()?;
    let pid = read_pid(&root)?;

    // SIGTERM — ask the daemon to finish cleanly
    let result = std::process::Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .output();

    match result {
        Ok(o) if o.status.success() => {
            println!("sent SIGTERM to PID {pid}");
            remove_pid(&root);
        }
        Ok(_) => {
            // process already gone — clean up stale PID file
            remove_pid(&root);
            println!("daemon was not running (stale PID {pid}) — cleaned up");
        }
        Err(e) => bail!("failed to send signal: {e}"),
    }
    Ok(())
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

    let daemon_pid = read_pid(&root).ok();
    let daemon_alive = daemon_pid.map(|pid| {
        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    });

    let agent_id = match &config.agent_id {
        Some(id) => id.clone(),
        None => {
            println!("NOT REGISTERED — run `punchclock agent run` to register");
            return Ok(());
        }
    };

    match res.agents.iter().find(|a| a.id == agent_id) {
        Some(a) => {
            let daemon = match daemon_alive {
                Some(true)  => format!("daemon PID {}", daemon_pid.unwrap()),
                Some(false) => "daemon dead (stale PID file)".to_string(),
                None        => "daemon not running".to_string(),
            };
            println!("ONLINE   {}  {}  ({})", a.id, a.name, daemon);
        }
        None => {
            let daemon = match daemon_alive {
                Some(true)  => format!("daemon PID {} running but agent not on team (server restarted?)", daemon_pid.unwrap()),
                Some(false) => "daemon dead (stale PID file)".to_string(),
                None        => "daemon not running — use `punchclock agent start`".to_string(),
            };
            println!("OFFLINE  {}  ({})", agent_id, daemon);
        }
    }
    Ok(())
}
