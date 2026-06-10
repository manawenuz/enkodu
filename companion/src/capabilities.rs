use serde::{Deserialize, Serialize};
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Capabilities {
    pub ffprobe_available: bool,
    pub encoders: Vec<String>,
    pub decoders: Vec<String>,
    pub platform: String,
}

/// Probe which codecs are available in the system's ffmpeg/ffprobe.
/// Runs short test encodes (1 frame, 64x64, null output) to confirm hardware encoders work.
pub fn detect(ffmpeg_path: &str, ffprobe_path: &str) -> Capabilities {
    let ffprobe_available = Command::new(ffprobe_path)
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    let encoders = detect_encoders(ffmpeg_path);
    let decoders = detect_decoders(ffmpeg_path);

    Capabilities {
        ffprobe_available,
        encoders,
        decoders,
        platform: current_platform(),
    }
}

fn detect_encoders(ffmpeg: &str) -> Vec<String> {
    let candidates = [
        "av1_qsv",
        "av1_nvenc",
        "av1_amf",
        "av1_vaapi",
        "libsvtav1",
        "hevc_qsv",
        "hevc_nvenc",
        "hevc_amf",
        "hevc_vaapi",
        "hevc_videotoolbox",
        "libx265",
        "h264_qsv",
        "h264_nvenc",
        "h264_amf",
        "h264_vaapi",
        "h264_videotoolbox",
        "libx264",
    ];
    let mut available = Vec::new();
    for enc in &candidates {
        if test_encoder(ffmpeg, enc) {
            available.push(enc.to_string());
        }
    }
    available
}

fn test_encoder(ffmpeg: &str, encoder: &str) -> bool {
    let status = Command::new(ffmpeg)
        .args([
            "-hide_banner",
            "-f",
            "lavfi",
            "-i",
            "testsrc=duration=1:size=64x64:rate=1",
            "-c:v",
            encoder,
            "-frames:v",
            "1",
            "-f",
            "null",
            "-",
            "-y",
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    status.map(|s| s.success()).unwrap_or(false)
}

fn detect_decoders(ffmpeg: &str) -> Vec<String> {
    // Check which hardware decoders exist by listing decoders
    let out = Command::new(ffmpeg)
        .args(["-hide_banner", "-decoders"])
        .output();
    let text = match out {
        Ok(o) => String::from_utf8_lossy(&o.stdout).to_string(),
        Err(_) => return vec![],
    };
    let hw_decoders = [
        "av1_qsv",
        "av1_cuvid",
        "hevc_qsv",
        "hevc_cuvid",
        "h264_qsv",
        "h264_cuvid",
    ];
    let mut found = Vec::new();
    // Always include software decoders if ffmpeg is available
    if !text.is_empty() {
        found.extend(["av1", "hevc", "h264"].iter().map(|s| s.to_string()));
    }
    for dec in &hw_decoders {
        if text.contains(dec) {
            found.push(dec.to_string());
        }
    }
    found
}

fn current_platform() -> String {
    #[cfg(target_os = "macos")]
    {
        "macos".to_string()
    }
    #[cfg(target_os = "windows")]
    {
        "windows".to_string()
    }
    #[cfg(target_os = "linux")]
    {
        "linux".to_string()
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        "unknown".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_with_missing_ffmpeg_returns_empty() {
        let caps = detect("/nonexistent/ffmpeg", "/nonexistent/ffprobe");
        assert!(!caps.ffprobe_available);
        assert!(caps.encoders.is_empty());
    }

    #[test]
    fn current_platform_is_nonempty() {
        assert!(!current_platform().is_empty());
    }
}
