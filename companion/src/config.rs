use anyhow::{Context, Result};
use dirs::home_dir;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Config {
    pub server_url: String,
    #[serde(default = "default_scan")]
    pub scan: ScanConfig,
    #[serde(default = "default_behavior")]
    pub behavior: BehaviorConfig,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ScanConfig {
    pub directories: Vec<String>,
    pub extensions: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BehaviorConfig {
    pub mode: String,
    pub on_success: String,
    pub backup_suffix: String,
    pub skip_if_av1: bool,
    pub min_duration_secs: u64,
}

fn default_scan() -> ScanConfig {
    ScanConfig {
        directories: vec![
            "~/Movies".to_string(),
            "~/Downloads".to_string(),
        ],
        extensions: vec![
            "mp4".to_string(), "mov".to_string(), "mkv".to_string(),
            "avi".to_string(), "m4v".to_string(), "ts".to_string(),
        ],
    }
}

fn default_behavior() -> BehaviorConfig {
    BehaviorConfig {
        mode: "interactive".to_string(),
        on_success: "rename".to_string(),
        backup_suffix: ".bak".to_string(),
        skip_if_av1: true,
        min_duration_secs: 30,
    }
}

impl Config {
    pub fn path() -> PathBuf {
        home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".config/enkodu/config.toml")
    }

    pub fn load() -> Result<Self> {
        let path = Self::path();
        if !path.exists() {
            let cfg = Self::default();
            cfg.save()?;
            return Ok(cfg);
        }
        let text = fs::read_to_string(&path).context("read config")?;
        toml::from_str(&text).context("parse config")
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path();
        fs::create_dir_all(path.parent().unwrap())?;
        let text = toml::to_string_pretty(self)?;
        fs::write(&path, text)?;
        Ok(())
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            server_url: "https://enkodu.manwe.qzz.io".to_string(),
            scan: default_scan(),
            behavior: default_behavior(),
        }
    }
}
