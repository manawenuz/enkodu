use crate::config::Config;
use crate::verify;
use std::path::{Path, PathBuf};

pub struct VideoFile {
    pub path: PathBuf,
    pub size: u64,
    pub duration: f64,
    pub codec: String,
}

pub fn scan(cfg: &Config) -> Vec<VideoFile> {
    let exts: std::collections::HashSet<String> = cfg.scan.extensions
        .iter()
        .map(|e| e.to_lowercase())
        .collect();

    let mut results = Vec::new();

    for dir_str in &cfg.scan.directories {
        let dir = expand_tilde(dir_str);
        if !dir.exists() {
            continue;
        }
        collect_videos(&dir, &exts, cfg, &mut results);
    }

    results.sort_by(|a, b| b.size.cmp(&a.size));
    results
}

fn collect_videos(dir: &Path, exts: &std::collections::HashSet<String>, cfg: &Config, out: &mut Vec<VideoFile>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_videos(&path, exts, cfg, out);
            continue;
        }
        let ext = path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .unwrap_or_default();
        if !exts.contains(&ext) {
            continue;
        }
        if path.file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.contains("_av1"))
            .unwrap_or(false)
        {
            continue;
        }
        let av1_sibling = path.with_extension("").to_str().map(|s| s.to_string() + "_av1.mp4");
        if av1_sibling.map(|s| Path::new(&s).exists()).unwrap_or(false) {
            continue;
        }
        let Ok(meta) = std::fs::metadata(&path) else { continue };
        let size = meta.len();

        let info = match verify::probe(&path) {
            Ok(i) => i,
            Err(_) => continue,
        };

        if cfg.behavior.skip_if_av1 && info.codec == "av1" {
            continue;
        }
        if info.duration < cfg.behavior.min_duration_secs as f64 {
            continue;
        }

        out.push(VideoFile {
            path,
            size,
            duration: info.duration,
            codec: info.codec,
        });
    }
}

fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(rest)
    } else {
        PathBuf::from(path)
    }
}

pub fn fmt_duration(secs: f64) -> String {
    let s = secs as u64;
    if s >= 3600 {
        format!("{}h{:02}m", s / 3600, (s % 3600) / 60)
    } else {
        format!("{}m{:02}s", s / 60, s % 60)
    }
}

pub fn fmt_size(bytes: u64) -> String {
    if bytes >= 1_000_000_000 {
        format!("{:.1} GB", bytes as f64 / 1e9)
    } else {
        format!("{:.0} MB", bytes as f64 / 1e6)
    }
}
