use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoStats {
    pub codec: String,
    pub width: u32,
    pub height: u32,
    pub duration_secs: f64,
    pub fps: f64,
    pub bitrate_kbps: Option<u64>,
    pub file_size_bytes: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct JobEntry {
    pub job_id: String,
    pub submitted_at: u64,
    pub status: String,
    pub output_path: Option<String>,
    /// Absolute path to the original source file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
    /// Video stats probed from the source before upload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_stats: Option<VideoStats>,
    /// Video stats probed from the downloaded AV1 output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_stats: Option<VideoStats>,
    /// Unix timestamp when the encode was detected as done (poll loop).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encode_finished_at: Option<u64>,
    /// Seconds from submitted_at to encode_finished_at.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encode_duration_secs: Option<u64>,
}

pub type State = HashMap<String, JobEntry>;

pub fn state_path() -> PathBuf {
    crate::platform::get_platform()
        .state_dir()
        .join("state.json")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_path_ends_with_state_json() {
        let p = state_path();
        assert!(p.to_string_lossy().ends_with("state.json"));
    }

    #[test]
    fn upsert_and_remove_roundtrip() {
        let key = "/tmp/test_video.mp4";
        let entry = JobEntry {
            job_id: "job-123".to_string(),
            submitted_at: 0,
            status: "pending".to_string(),
            output_path: None,
            source_path: None,
            source_stats: None,
            output_stats: None,
            encode_finished_at: None,
            encode_duration_secs: None,
        };
        upsert(key, entry).unwrap();
        let s = load().unwrap();
        assert!(s.contains_key(key));
        let removed = remove(key).unwrap();
        assert!(removed);
        let s = load().unwrap();
        assert!(!s.contains_key(key));
    }
}
