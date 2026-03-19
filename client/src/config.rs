use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalConfig {
    #[serde(default = "default_server")]
    pub server: String,
}

fn default_server() -> String {
    "http://localhost:8421".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoEntry {
    pub path: PathBuf,
    pub description: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub claude_flags: String,
}

fn default_true() -> bool {
    true
}

pub type ReposConfig = BTreeMap<String, RepoEntry>;

pub fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("punchclock")
}

pub fn load_global() -> Result<GlobalConfig> {
    let path = config_dir().join("config.toml");
    if path.exists() {
        let content = std::fs::read_to_string(&path).context("failed to read config.toml")?;
        toml::from_str(&content).context("invalid config.toml")
    } else {
        Ok(GlobalConfig {
            server: default_server(),
        })
    }
}

pub fn load_repos() -> Result<ReposConfig> {
    let path = config_dir().join("repos.toml");
    if path.exists() {
        let content = std::fs::read_to_string(&path).context("failed to read repos.toml")?;
        toml::from_str(&content).context("invalid repos.toml")
    } else {
        Ok(BTreeMap::new())
    }
}

pub fn save_repos(repos: &ReposConfig) -> Result<()> {
    let dir = config_dir();
    std::fs::create_dir_all(&dir).context("failed to create config directory")?;
    let path = dir.join("repos.toml");
    let content = toml::to_string_pretty(repos).context("failed to serialize repos")?;
    std::fs::write(&path, content).context("failed to write repos.toml")?;
    Ok(())
}
