use anyhow::Result;
use dirs::home_dir;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct JobEntry {
    pub job_id: String,
    pub submitted_at: u64,
    pub status: String,
    pub output_path: Option<String>,
}

pub type State = HashMap<String, JobEntry>;

pub fn state_path() -> PathBuf {
    home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".config/enkodu/state.json")
}

pub fn load() -> Result<State> {
    let path = state_path();
    if !path.exists() {
        return Ok(State::new());
    }
    let text = fs::read_to_string(&path)?;
    Ok(serde_json::from_str(&text).unwrap_or_default())
}

pub fn save(state: &State) -> Result<()> {
    let path = state_path();
    fs::create_dir_all(path.parent().unwrap())?;
    fs::write(&path, serde_json::to_string_pretty(state)?)?;
    Ok(())
}

pub fn upsert(file_path: &str, entry: JobEntry) -> Result<()> {
    let mut s = load()?;
    s.insert(file_path.to_string(), entry);
    save(&s)
}

pub fn remove(file_path: &str) -> Result<bool> {
    let mut s = load()?;
    let removed = s.remove(file_path).is_some();
    save(&s)?;
    Ok(removed)
}
