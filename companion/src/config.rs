use anyhow::{Context, Result};
use dirs::home_dir;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Config {
    pub server_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_token: Option<String>,
    #[serde(default = "default_scan")]
    pub scan: ScanConfig,
    #[serde(default = "default_behavior")]
    pub behavior: BehaviorConfig,
    #[serde(default)]
    pub companion_id: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ScanConfig {
    pub directories: Vec<String>,
    pub extensions: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ReviewMode {
    #[default]
    Auto,
    Manual,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BehaviorConfig {
    pub mode: String,
    pub on_success: String,
    pub backup_suffix: String,
    pub skip_if_av1: bool,
    pub min_duration_secs: u64,
    #[serde(default)]
    pub review_mode: ReviewMode,
}

fn default_scan() -> ScanConfig {
    ScanConfig {
        directories: vec!["~/Movies".to_string(), "~/Downloads".to_string()],
        extensions: vec![
            "mp4".to_string(),
            "mov".to_string(),
            "mkv".to_string(),
            "avi".to_string(),
            "m4v".to_string(),
            "ts".to_string(),
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
        review_mode: ReviewMode::Auto,
    }
}

/// Platform-specific config directory.
fn config_dir() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".config/enkodu")
    }
    #[cfg(target_os = "linux")]
    {
        if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
            PathBuf::from(xdg).join("enkodu")
        } else {
            home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".config/enkodu")
        }
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var("APPDATA")
            .map(|p| PathBuf::from(p).join("Enkodu"))
            .unwrap_or_else(|_| PathBuf::from("C:\\ProgramData\\Enkodu"))
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".config/enkodu")
    }
}

impl Config {
    pub fn path() -> PathBuf {
        config_dir().join("config.toml")
    }

    pub fn load() -> Result<Self> {
        let path = Self::path();
        if !path.exists() {
            let mut cfg = Self::default();
            cfg.companion_id = uuid_v4();
            cfg.save()?;
            return Ok(cfg);
        }
        let text = fs::read_to_string(&path).context("read config")?;
        let mut cfg: Self = toml::from_str(&text).context("parse config")?;
        if cfg.companion_id.is_empty() {
            cfg.companion_id = uuid_v4();
            cfg.save()?;
        }
        Ok(cfg)
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
            auth_token: None,
            scan: default_scan(),
            behavior: default_behavior(),
            companion_id: String::new(),
        }
    }
}

fn uuid_v4() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let mut b: Vec<u8> = (0..16).map(|_| rng.gen::<u8>()).collect();
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_path_ends_with_config_toml() {
        let p = Config::path();
        assert!(p.to_string_lossy().ends_with("config.toml"));
    }

    #[test]
    fn config_dir_is_not_empty() {
        let d = config_dir();
        assert!(!d.to_string_lossy().is_empty());
    }

    #[test]
    fn default_config_has_expected_fields() {
        let cfg = Config::default();
        assert!(!cfg.server_url.is_empty());
        assert_eq!(cfg.behavior.on_success, "rename");
        assert_eq!(cfg.behavior.mode, "interactive");
        assert_eq!(cfg.behavior.skip_if_av1, true);
        assert_eq!(cfg.behavior.min_duration_secs, 30);
    }

    #[test]
    fn default_scan_extensions_include_mp4() {
        let cfg = Config::default();
        assert!(cfg.scan.extensions.contains(&"mp4".to_string()));
    }

    #[test]
    fn default_review_mode_is_auto() {
        let cfg = Config::default();
        assert_eq!(cfg.behavior.review_mode, ReviewMode::Auto);
    }

    #[test]
    fn review_mode_serializes_snake_case() {
        let toml_str = toml::to_string(&Config::default()).unwrap();
        assert!(toml_str.contains("review_mode = \"auto\""));
    }

    #[test]
    fn config_without_review_mode_defaults_to_auto() {
        // Older config files have no review_mode field.
        let legacy = r#"
server_url = "http://x"
[scan]
directories = []
extensions = []
[behavior]
mode = "interactive"
on_success = "rename"
backup_suffix = ".bak"
skip_if_av1 = true
min_duration_secs = 30
"#;
        let cfg: Config = toml::from_str(legacy).unwrap();
        assert_eq!(cfg.behavior.review_mode, ReviewMode::Auto);
    }

    #[test]
    fn companion_id_is_generated_on_first_load() {
        // Default has empty id; load() generates one.
        // We can't call load() in test (side effects), so just test uuid_v4() format.
        let id = uuid_v4();
        assert!(id.contains('-'));
        assert_eq!(id.len(), 36);
    }

    #[test]
    fn config_parses_manual_review_mode() {
        let s = r#"
server_url = "http://x"
[scan]
directories = []
extensions = []
[behavior]
mode = "interactive"
on_success = "keep"
backup_suffix = ".bak"
skip_if_av1 = true
min_duration_secs = 30
review_mode = "manual"
"#;
        let cfg: Config = toml::from_str(s).unwrap();
        assert_eq!(cfg.behavior.review_mode, ReviewMode::Manual);
    }
}
