use anyhow::{bail, Context};
use dialoguer::{theme::ColorfulTheme, Input};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::{Path, PathBuf}, process::Stdio, sync::{Arc, atomic::{AtomicBool, Ordering}, mpsc::SyncSender}};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::time::{interval, Duration};
use punchclock_common::{InboxResponse, RegisterResponse, TaskSyncItem, TaskSyncRequest, TeamResponse};

use crate::tui::{spawn_tui, TuiEvent};

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
    default: "--allowedTools Edit,Write,Bash,Read"
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

// ── task sync ─────────────────────────────────────────────────────────────────

fn task_dir_hash(root: &str) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut entries: Vec<(String, u64)> = Vec::new();
    for subdir in &["todo", "done", "blocked"] {
        let dir = std::path::Path::new(root).join(".task").join(subdir);
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for entry in rd.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("md") {
                    continue;
                }
                let name = path.file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_string();
                let mtime = entry.metadata()
                    .and_then(|m| m.modified())
                    .map(|t| t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs())
                    .unwrap_or(0);
                entries.push((format!("{subdir}/{name}"), mtime));
            }
        }
    }
    entries.sort();
    let mut h = DefaultHasher::new();
    entries.hash(&mut h);
    h.finish()
}

fn read_task_snapshot(root: &str) -> Vec<TaskSyncItem> {
    let mut items = Vec::new();
    for (subdir, status) in &[("todo", "queued"), ("done", "done"), ("blocked", "blocked")] {
        let dir = std::path::Path::new(root).join(".task").join(subdir);
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for entry in rd.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("md") {
                    continue;
                }
                let id = path.file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_string();
                let content = std::fs::read_to_string(&path).unwrap_or_default();
                let title = content
                    .lines()
                    .find(|l| l.starts_with("# "))
                    .map(|l| l.trim_start_matches("# ").to_string())
                    .unwrap_or_else(|| id.clone());
                // RFC3339-ish timestamp from mtime seconds since epoch
                let modified_at = entry.metadata()
                    .and_then(|m| m.modified())
                    .map(|t| {
                        let secs = t.duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs();
                        // Simple ISO 8601 UTC: YYYY-MM-DDTHH:MM:SSZ
                        let s = secs;
                        let sec = s % 60;
                        let min = (s / 60) % 60;
                        let hr  = (s / 3600) % 24;
                        let days = s / 86400;
                        // days since 1970-01-01
                        let (y, mo, d) = days_to_ymd(days);
                        format!("{y:04}-{mo:02}-{d:02}T{hr:02}:{min:02}:{sec:02}Z")
                    })
                    .unwrap_or_default();
                items.push(TaskSyncItem {
                    id,
                    title,
                    status: status.to_string(),
                    modified_at,
                });
            }
        }
    }
    items
}

fn days_to_ymd(days: u64) -> (u64, u64, u64) {
    // Gregorian calendar calculation from Unix epoch days
    let z = days + 719468;
    let era = z / 146097;
    let doe = z % 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
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

    // register on first run if no agent_id yet; fall back to local UUID if server offline
    if config.agent_id.is_none() {
        let client = reqwest::Client::new();
        let id = match client
            .get(format!("{base}/register"))
            .query(&[
                ("name", config.name.as_str()),
                ("description", config.description.as_str()),
            ])
            .send()
            .await
            .and_then(|r| r.error_for_status())
        {
            Ok(r) => match r.json::<RegisterResponse>().await {
                Ok(res) => { res.agent_id }
                Err(_) => uuid::Uuid::new_v4().to_string(),
            },
            Err(_) => {
                let id = uuid::Uuid::new_v4().to_string();
                // will show as OFFLINE in TUI; agent_id logged in AgentInfo
                id
            }
        };
        config.agent_id = Some(id);
        config.save(&root)?;
    }

    let agent_id = config.agent_id.clone().unwrap();
    let claude_flags = config.claude_flags.clone();
    let root_str = root.to_string_lossy().to_string();

    let tx = spawn_tui();
    let _ = tx.send(TuiEvent::AgentInfo {
        name: config.name.clone(),
        id: agent_id.clone(),
        server: base.clone(),
    });

    // show pending tasks on startup
    {
        let todo_dir = std::path::Path::new(&root_str).join(".task/todo");
        let mut entries: Vec<(PathBuf, std::time::SystemTime)> = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&todo_dir) {
            for entry in rd.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("md") { continue; }
                let mtime = entry.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH.into());
                entries.push((path, mtime));
            }
        }
        entries.sort_by_key(|(_, t)| *t);
        let total = entries.len();
        if total == 0 {
            let _ = tx.send(TuiEvent::Log("no tasks in .task/todo/".to_string()));
        } else {
            let mut summary = format!("{total} task(s) queued:");
            for (path, _) in entries.iter().take(5) {
                let content = std::fs::read_to_string(path).unwrap_or_default();
                let title = content.lines()
                    .find(|l| l.starts_with("# "))
                    .map(|l| l.trim_start_matches("# ").to_string())
                    .unwrap_or_else(|| path.file_stem().and_then(|s| s.to_str()).unwrap_or("?").to_string());
                summary.push_str(&format!("  • {title}"));
            }
            if total > 5 { summary.push_str(&format!("  … and {} more", total - 5)); }
            let _ = tx.send(TuiEvent::Log(summary));
        }
    }

    let connected = Arc::new(AtomicBool::new(false));

    write_pid(&root)?;
    let root_for_cleanup = root.clone();
    // remove PID file on exit via a drop guard
    struct PidGuard(PathBuf);
    impl Drop for PidGuard { fn drop(&mut self) { remove_pid(&self.0); } }
    let _pid_guard = PidGuard(root_for_cleanup);

    // background heartbeat + connection tracking
    let hb_base = base.clone();
    let hb_id = agent_id.clone();
    let hb_name = config.name.clone();
    let hb_desc = config.description.clone();
    let hb_connected = connected.clone();
    let hb_tx = tx.clone();
    tokio::spawn(async move {
        let client = reqwest::Client::new();
        let mut ticker = interval(Duration::from_secs(5));
        loop {
            ticker.tick().await;
            let ok = client
                .get(format!("{hb_base}/heartbeat"))
                .query(&[
                    ("agent_id", hb_id.as_str()),
                    ("name", hb_name.as_str()),
                    ("description", hb_desc.as_str()),
                ])
                .send()
                .await
                .and_then(|r| r.error_for_status())
                .is_ok();
            let was = hb_connected.swap(ok, Ordering::Relaxed);
            if ok != was {
                let _ = hb_tx.send(TuiEvent::ServerStatus(ok));
            }
        }
    });

    // background task loop
    let task_base = base.clone();
    let task_flags = claude_flags.clone();
    let task_root = root_str.clone();
    let task_tx = tx.clone();
    tokio::spawn(async move {
        let client = reqwest::Client::new();
        let mut ticker = interval(Duration::from_secs(5));
        let todo_dir = std::path::Path::new(&task_root).join(".task/todo");
        let done_dir = std::path::Path::new(&task_root).join(".task/done");
        let blocked_dir = std::path::Path::new(&task_root).join(".task/blocked");
        let _ = std::fs::create_dir_all(&todo_dir);
        let _ = std::fs::create_dir_all(&done_dir);
        let _ = std::fs::create_dir_all(&blocked_dir);
        let mut priority_queue: Vec<String> = Vec::new();
        loop {
            ticker.tick().await;

            // refill priority queue when exhausted
            if priority_queue.is_empty() {
                let mut entries: Vec<(std::path::PathBuf, std::time::SystemTime)> = Vec::new();
                if let Ok(rd) = std::fs::read_dir(&todo_dir) {
                    for entry in rd.flatten() {
                        let path = entry.path();
                        if path.extension().and_then(|e| e.to_str()) != Some("md") { continue; }
                        let mtime = entry.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH.into());
                        entries.push((path, mtime));
                    }
                }
                entries.sort_by_key(|(_, t)| *t);
                if entries.is_empty() { continue; }

                // order by mtime (oldest first — FIFO)
                let count = entries.len();
                let _ = task_tx.send(TuiEvent::Triaging { count });
                priority_queue = entries.iter()
                    .map(|(p, _)| p.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string())
                    .filter(|id| !id.is_empty())
                    .collect();
                if priority_queue.is_empty() { continue; }
            }

            // pop the next valid task from the queue
            let tid = loop {
                if priority_queue.is_empty() { break None; }
                let candidate = priority_queue.remove(0);
                if todo_dir.join(format!("{candidate}.md")).exists() {
                    break Some(candidate);
                }
            };
            let tid = match tid {
                Some(t) => t,
                None => continue,
            };

            let todo_file = todo_dir.join(format!("{tid}.md"));
            let content = std::fs::read_to_string(&todo_file).unwrap_or_default();
            let title = content.lines()
                .find(|l| l.starts_with("# "))
                .map(|l| l.trim_start_matches("# ").to_string())
                .unwrap_or_else(|| tid.clone());
            let _ = task_tx.send(TuiEvent::TaskStart { id: tid.clone(), title: title.clone() });

            let prompt = format!(
                "IMPORTANT: If you cannot complete this task for any reason — missing permissions, \
                 need clarification, blocked by something outside your control — respond with \
                 BLOCKED: <reason> as the very first line of your response. Do not ask questions; \
                 just state what is needed.\n\n{content}"
            );
            let reply = route_to_claude(&prompt, &task_flags, &task_root, &task_tx, &format!("working on [{tid}]…")).await;

            // detect BLOCKED: <reason> as the first line of output
            let (status, dest_dir, section_heading) =
                if let Some(_reason) = reply.strip_prefix("BLOCKED:") {
                    ("blocked", &blocked_dir, "Blocked")
                } else if reply.starts_with("[claude exited") || reply.starts_with("[failed") {
                    ("blocked", &blocked_dir, "Error")
                } else {
                    ("done", &done_dir, "Result")
                };
            let _ = task_tx.send(TuiEvent::TaskDone {
                id: tid.clone(),
                status: status.to_string(),
                chars: reply.len(),
            });

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

    // background task-sync loop — only when connected; hashes .task/ every 5 s, pushes on change
    let sync_base = base.clone();
    let sync_id = agent_id.clone();
    let sync_root = root_str.clone();
    let sync_connected = connected.clone();
    tokio::spawn(async move {
        let client = reqwest::Client::new();
        let mut ticker = interval(Duration::from_secs(5));
        let mut last_hash: u64 = 0;
        loop {
            ticker.tick().await;
            if !sync_connected.load(Ordering::Relaxed) { continue; }
            let hash = task_dir_hash(&sync_root);
            if hash == last_hash { continue; }
            let tasks = read_task_snapshot(&sync_root);
            let payload = TaskSyncRequest { agent_id: sync_id.clone(), hash, tasks };
            if let Ok(r) = client.post(format!("{sync_base}/task/sync")).json(&payload).send().await {
                if r.status().is_success() { last_hash = hash; }
            }
        }
    });

    // message poll loop — exits cleanly on SIGINT or SIGTERM; skips when offline
    let client = reqwest::Client::new();
    let mut poll = interval(Duration::from_secs(5));
    loop {
        tokio::select! {
            _ = poll.tick() => {}
            _ = tokio::signal::ctrl_c() => {
                let _ = tx.send(TuiEvent::Shutdown);
                break;
            }
        }

        if !connected.load(Ordering::Relaxed) { continue; }

        let inbox: InboxResponse = match client
            .get(format!("{base}/message/recv"))
            .query(&[("agent_id", &agent_id)])
            .send()
            .await
            .and_then(|r| r.error_for_status())
        {
            Ok(r) => match r.json().await {
                Ok(v) => v,
                Err(_) => continue,
            },
            Err(_) => continue, // heartbeat loop reports the disconnection
        };

        for msg in inbox.messages {
            let _ = tx.send(TuiEvent::MsgRecv { from: msg.from.clone() });
            let reply = route_to_claude(&msg.body, &claude_flags, &root_str, &tx, &format!("replying to {}…", msg.from)).await;
            let _ = tx.send(TuiEvent::MsgSent { chars: reply.len() });
            let _ = client
                .get(format!("{base}/message/send"))
                .query(&[("to", &msg.from), ("from", &agent_id), ("body", &reply)])
                .send()
                .await;
        }
    }
    Ok(())
}


async fn route_to_claude(body: &str, flags: &str, cwd: &str, tx: &SyncSender<TuiEvent>, label: &str) -> String {
    let mut cmd = Command::new("claude");
    cmd.arg("-p").arg(body);
    cmd.arg("--dangerously-skip-permissions");
    cmd.arg("--output-format").arg("stream-json");
    cmd.arg("--verbose");
    cmd.current_dir(cwd);
    cmd.env_remove("CLAUDECODE");
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    for flag in flags.split_whitespace().filter(|s| !s.is_empty()) {
        cmd.arg(flag);
    }

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return format!("[failed to spawn claude: {e}]"),
    };

    let stdout = match child.stdout.take() {
        Some(s) => s,
        None => return "[failed to capture stdout]".to_string(),
    };
    let stderr = child.stderr.take();

    let mut files_written: Vec<String> = Vec::new();
    let mut files_read: Vec<String> = Vec::new();
    let mut bash_runs: u32 = 0;
    let mut result_text = String::new();
    let mut is_error = false;

    let _ = tx.send(TuiEvent::SpinnerUpdate(label.to_string()));

    // drain stderr in background so it doesn't block stdout
    let stderr_task = tokio::spawn(async move {
        if let Some(s) = stderr {
            let mut lines = BufReader::new(s).lines();
            let mut buf = Vec::new();
            while let Ok(Some(l)) = lines.next_line().await {
                buf.push(l);
            }
            buf
        } else {
            Vec::new()
        }
    });

    let mut lines = BufReader::new(stdout).lines();
    loop {
        let line = match lines.next_line().await {
            Ok(Some(l)) => l,
            Ok(None) => break,
            Err(_) => break,
        };

        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else { continue };

        match v.get("type").and_then(|t| t.as_str()) {
            Some("assistant") => {
                let content = v
                    .pointer("/message/content")
                    .and_then(|c| c.as_array());
                if let Some(blocks) = content {
                    for block in blocks {
                        if block.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                            let name = block.get("name").and_then(|n| n.as_str()).unwrap_or("");
                            let input = block.get("input");
                            match name {
                                "Edit" | "Write" | "NotebookEdit" => {
                                    if let Some(path) = input
                                        .and_then(|i| i.get("file_path"))
                                        .and_then(|p| p.as_str())
                                    {
                                        files_written.push(path.to_string());
                                        let _ = tx.send(TuiEvent::SpinnerUpdate(format!("Editing {path}…")));
                                    }
                                }
                                "Read" => {
                                    if let Some(path) = input
                                        .and_then(|i| i.get("file_path"))
                                        .and_then(|p| p.as_str())
                                    {
                                        files_read.push(path.to_string());
                                    }
                                }
                                "Bash" => {
                                    bash_runs += 1;
                                    let cmd_str = input
                                        .and_then(|i| i.get("command"))
                                        .and_then(|c| c.as_str())
                                        .unwrap_or("…");
                                    let preview: String = cmd_str.chars().take(50).collect();
                                    let _ = tx.send(TuiEvent::SpinnerUpdate(format!("Running {preview}…")));
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
            Some("result") => {
                is_error = v.get("is_error").and_then(|e| e.as_bool()).unwrap_or(false);
                result_text = v
                    .get("result")
                    .and_then(|r| r.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
            }
            _ => {}
        }
    }

    // collect stderr and exit code
    let stderr_lines = stderr_task.await.unwrap_or_default();
    let exit_status = child.wait().await.ok();
    let exit_ok = exit_status.map(|s| s.success()).unwrap_or(false);

    // log stderr if present
    if !stderr_lines.is_empty() {
        let preview: String = stderr_lines.join(" ").chars().take(200).collect();
        let _ = tx.send(TuiEvent::Log(format!("  ✗ stderr: {preview}")));
    }

    // surface non-zero exit as an error result
    if !exit_ok && result_text.is_empty() {
        let code = exit_status.and_then(|s| s.code()).map(|c| c.to_string()).unwrap_or_else(|| "?".to_string());
        result_text = format!("[claude exited {code}]{}", if stderr_lines.is_empty() { String::new() } else { format!(": {}", stderr_lines.join("; ")) });
        is_error = true;
    } else if is_error && result_text.is_empty() {
        result_text = "[claude reported an error with no message]".to_string();
    }

    // emit coarse summary
    let mut parts = Vec::new();
    if !files_written.is_empty() {
        let unique: std::collections::HashSet<_> = files_written.iter().collect();
        parts.push(format!("edited {} file(s)", unique.len()));
    }
    if !files_read.is_empty() {
        let unique: std::collections::HashSet<_> = files_read.iter().collect();
        parts.push(format!("read {} file(s)", unique.len()));
    }
    if bash_runs > 0 {
        parts.push(format!("ran {} command(s)", bash_runs));
    }
    if !parts.is_empty() {
        let _ = tx.send(TuiEvent::Log(format!("  ↳ {}", parts.join(", "))));
    }

    // if claude flagged is_error but still produced text, prefix so callers route it to blocked
    if is_error && !result_text.starts_with("[claude exited") {
        result_text = format!("[claude exited with error]: {result_text}");
    }

    result_text
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

// ── install helpers ───────────────────────────────────────────────────────────

/// Sanitise a repo name into a string safe for service labels / file names.
fn sanitize_name(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect()
}

/// Persistent per-repo log directory (survives reboots, doesn't clash with .punchclock file).
/// Returns `~/.punchclock/<hash>/daemon.log`.
fn install_log_path(repo_root: &Path) -> anyhow::Result<PathBuf> {
    let hash = format!("{:x}", md5_path(repo_root));
    let home = dirs::home_dir().context("cannot determine home directory")?;
    let dir = home.join(".punchclock").join(hash);
    std::fs::create_dir_all(&dir)?;
    Ok(dir.join("daemon.log"))
}

// macOS helpers

fn macos_plist_label(repo_name: &str) -> String {
    format!("com.punchclock.{}", sanitize_name(repo_name))
}

fn macos_plist_path(label: &str) -> anyhow::Result<PathBuf> {
    let home = dirs::home_dir().context("cannot determine home directory")?;
    let dir = home.join("Library/LaunchAgents");
    std::fs::create_dir_all(&dir)?;
    Ok(dir.join(format!("{label}.plist")))
}

fn macos_plist_content(label: &str, exe: &Path, repo_root: &Path, log: &Path) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>Label</key>             <string>{label}</string>
  <key>ProgramArguments</key>  <array>
                                 <string>{exe}</string>
                                 <string>agent</string>
                                 <string>run</string>
                               </array>
  <key>WorkingDirectory</key>  <string>{repo}</string>
  <key>RunAtLoad</key>         <true/>
  <key>KeepAlive</key>         <true/>
  <key>StandardOutPath</key>   <string>{log}</string>
  <key>StandardErrorPath</key> <string>{log}</string>
</dict></plist>
"#,
        exe = exe.display(),
        repo = repo_root.display(),
        log = log.display(),
    )
}

// Linux helpers

fn linux_service_name(repo_name: &str) -> String {
    format!("punchclock-{}", sanitize_name(repo_name))
}

fn linux_service_path(svc_name: &str) -> anyhow::Result<PathBuf> {
    let home = dirs::home_dir().context("cannot determine home directory")?;
    let dir = home.join(".config/systemd/user");
    std::fs::create_dir_all(&dir)?;
    Ok(dir.join(format!("{svc_name}.service")))
}

fn linux_service_content(svc_name: &str, exe: &Path, repo_root: &Path, log: &Path) -> String {
    format!(
        "[Unit]\nDescription=punchclock agent for {svc_name}\nAfter=network.target\n\n\
         [Service]\nExecStart={exe} agent run\nWorkingDirectory={repo}\nRestart=on-failure\n\
         StandardOutput=append:{log}\nStandardError=append:{log}\n\n\
         [Install]\nWantedBy=default.target\n",
        exe = exe.display(),
        repo = repo_root.display(),
        log = log.display(),
    )
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

// ── install / uninstall / logs ────────────────────────────────────────────────

pub async fn install() -> anyhow::Result<()> {
    let root = find_repo_root()?;
    let name = repo_name(&root);
    let exe = std::env::current_exe().context("cannot determine executable path")?;
    let log = install_log_path(&root)?;

    match std::env::consts::OS {
        "macos" => {
            let label = macos_plist_label(&name);
            let plist = macos_plist_path(&label)?;
            std::fs::write(&plist, macos_plist_content(&label, &exe, &root, &log))?;
            let status = std::process::Command::new("launchctl")
                .args(["load", "-w", &plist.to_string_lossy()])
                .status()
                .context("failed to run launchctl")?;
            if !status.success() {
                bail!("launchctl load failed — check the plist at {}", plist.display());
            }
            println!("installed  {}", plist.display());
            println!("service    {label}");
            println!("log        {}", log.display());
        }
        "linux" => {
            let svc = linux_service_name(&name);
            let svc_path = linux_service_path(&svc)?;
            std::fs::write(&svc_path, linux_service_content(&svc, &exe, &root, &log))?;
            let status = std::process::Command::new("systemctl")
                .args(["--user", "enable", "--now", &svc])
                .status()
                .context("failed to run systemctl")?;
            if !status.success() {
                bail!("systemctl enable failed — check the unit at {}", svc_path.display());
            }
            println!("installed  {}", svc_path.display());
            println!("service    {svc}");
            println!("log        {}", log.display());
        }
        other => bail!("OS daemon installation is not supported on {other}"),
    }

    println!("\nRun `punchclock agent logs` to follow the log.");
    Ok(())
}

pub async fn uninstall() -> anyhow::Result<()> {
    let root = find_repo_root()?;
    let name = repo_name(&root);

    match std::env::consts::OS {
        "macos" => {
            let label = macos_plist_label(&name);
            let plist = macos_plist_path(&label)?;
            if plist.exists() {
                let _ = std::process::Command::new("launchctl")
                    .args(["unload", "-w", &plist.to_string_lossy()])
                    .status();
                std::fs::remove_file(&plist)?;
                println!("removed  {}", plist.display());
            } else {
                println!("not installed (expected plist at {})", plist.display());
            }
        }
        "linux" => {
            let svc = linux_service_name(&name);
            let svc_path = linux_service_path(&svc)?;
            let _ = std::process::Command::new("systemctl")
                .args(["--user", "disable", "--now", &svc])
                .status();
            if svc_path.exists() {
                std::fs::remove_file(&svc_path)?;
                println!("removed  {}", svc_path.display());
            } else {
                println!("not installed (expected unit at {})", svc_path.display());
            }
        }
        other => bail!("OS daemon management is not supported on {other}"),
    }

    Ok(())
}

pub async fn logs() -> anyhow::Result<()> {
    let root = find_repo_root()?;
    let log = install_log_path(&root)?;

    // Fall back to the runtime log used by `agent start` if the install log doesn't exist yet.
    let log = if log.exists() {
        log
    } else {
        let fallback = runtime_dir(&root).join("daemon.log");
        if fallback.exists() {
            fallback
        } else {
            bail!(
                "no log file found — run `punchclock agent install` or `punchclock agent start` first\n\
                 (looked in {} and {})",
                log.display(),
                fallback.display()
            );
        }
    };

    println!("tailing {}", log.display());
    let mut child = std::process::Command::new("tail")
        .args(["-f", &log.to_string_lossy().into_owned()])
        .spawn()
        .context("failed to run tail")?;
    child.wait()?;
    Ok(())
}
