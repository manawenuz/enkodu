use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct VideoInfo {
    pub codec: String,
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub audio_codec: Option<String>,
}

pub fn probe(path: &Path) -> Result<VideoInfo> {
    let ffprobe = find_ffprobe();

    // Single call for all video stream fields
    let video_raw = run_ffprobe(
        &ffprobe,
        path,
        &[
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=codec_name,width,height,r_frame_rate",
        ],
    )?;
    let mut vlines = video_raw.lines();
    let codec = vlines.next().unwrap_or("").trim().to_string();
    let width: u32 = vlines.next().unwrap_or("0").trim().parse().unwrap_or(0);
    let height: u32 = vlines.next().unwrap_or("0").trim().parse().unwrap_or(0);
    let fps = parse_fps(vlines.next().unwrap_or("0"));

    let duration_str = run_ffprobe(&ffprobe, path, &["-show_entries", "format=duration"])?;
    let duration: f64 = duration_str.trim().parse().context("parse duration")?;

    let audio_codec = run_ffprobe(
        &ffprobe,
        path,
        &[
            "-select_streams",
            "a:0",
            "-show_entries",
            "stream=codec_name",
        ],
    )
    .ok()
    .filter(|s| !s.is_empty());

    Ok(VideoInfo {
        codec,
        duration,
        width,
        height,
        fps,
        audio_codec,
    })
}

fn parse_fps(s: &str) -> f64 {
    let s = s.trim();
    if let Some((n, d)) = s.split_once('/') {
        let n: f64 = n.trim().parse().unwrap_or(0.0);
        let d: f64 = d.trim().parse().unwrap_or(1.0);
        if d > 0.0 {
            n / d
        } else {
            0.0
        }
    } else {
        s.parse().unwrap_or(0.0)
    }
}

pub fn verify_output(output: &Path, source_duration: f64) -> Result<()> {
    let info = probe(output).context("ffprobe on output")?;
    if info.codec != "av1" {
        bail!("output codec is '{}', expected av1", info.codec);
    }
    let diff = (info.duration - source_duration).abs();
    if diff > 2.0 {
        bail!(
            "duration mismatch: source={:.1}s output={:.1}s diff={:.1}s",
            source_duration,
            info.duration,
            diff
        );
    }
    Ok(())
}

fn run_ffprobe(ffprobe: &str, path: &Path, extra_args: &[&str]) -> Result<String> {
    let mut args = vec!["-v", "error", "-of", "default=noprint_wrappers=1:nokey=1"];
    args.extend_from_slice(extra_args);
    args.push(path.to_str().unwrap());
    let out = Command::new(ffprobe)
        .args(&args)
        .output()
        .context("spawn ffprobe")?;
    if !out.status.success() {
        bail!("ffprobe: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub fn find_ffprobe() -> String {
    for candidate in &[
        "ffprobe",
        "/opt/homebrew/bin/ffprobe",
        "/usr/local/bin/ffprobe",
    ] {
        if Command::new(candidate).arg("-version").output().is_ok() {
            return candidate.to_string();
        }
    }
    "ffprobe".to_string()
}

pub fn find_ffmpeg() -> String {
    for candidate in &[
        "ffmpeg",
        "/opt/homebrew/bin/ffmpeg",
        "/usr/local/bin/ffmpeg",
        "C:\\msys64\\mingw64\\bin\\ffmpeg.exe",
    ] {
        if Command::new(candidate).arg("-version").output().is_ok() {
            return candidate.to_string();
        }
    }
    "ffmpeg".to_string()
}

/// Probe a file and return rich stats for the review UI.
pub fn probe_stats(path: &Path) -> Result<crate::state::VideoStats> {
    let info = probe(path)?;
    let file_size_bytes = path.metadata().map(|m| m.len()).unwrap_or(0);
    let ffprobe = find_ffprobe();
    let bitrate_kbps = run_ffprobe(&ffprobe, path, &["-show_entries", "format=bit_rate"])
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .map(|bps| bps / 1000);
    Ok(crate::state::VideoStats {
        codec: info.codec,
        width: info.width,
        height: info.height,
        duration_secs: info.duration,
        fps: info.fps,
        bitrate_kbps,
        file_size_bytes,
    })
}

/// Returns true if ffprobe can be found and executed on this machine.
pub fn check_ffprobe_available() -> bool {
    let path = find_ffprobe();
    Command::new(&path).arg("-version").output().is_ok()
}

/// Determine the output path for a given source path.
/// e.g. `/home/user/video.mp4` -> `/home/user/video_av1.mp4`
pub fn output_path_for(source: &Path) -> PathBuf {
    source.with_file_name(format!(
        "{}_av1.mp4",
        source
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("output")
    ))
}

/// Check if a job is eligible for download based on server status and verify_status.
/// Returns true only when status is "done" and verify_status is "pass", empty, or absent.
/// Logs a warning when verify_status is empty.
pub fn is_downloadable(status: &str, verify_status: Option<&str>) -> bool {
    if status != "done" {
        return false;
    }
    match verify_status {
        Some("pass") => true,
        Some("") | None => {
            log::warn!(
                "Downloading job with empty verify_status — server may not have verified yet"
            );
            true // permissive for backward compatibility with older servers
        }
        Some("fail") => false,
        Some(other) => {
            log::warn!(
                "Unknown verify_status '{}', treating as not downloadable",
                other
            );
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn output_path_for_mp4() {
        let src = PathBuf::from("/home/user/video.mp4");
        let out = output_path_for(&src);
        assert_eq!(out, PathBuf::from("/home/user/video_av1.mp4"));
    }

    #[test]
    fn output_path_for_mov() {
        let src = PathBuf::from("/tmp/foo.mov");
        let out = output_path_for(&src);
        assert_eq!(out, PathBuf::from("/tmp/foo_av1.mp4"));
    }

    #[test]
    fn output_path_for_no_extension() {
        let src = PathBuf::from("/tmp/foo");
        let out = output_path_for(&src);
        assert_eq!(out, PathBuf::from("/tmp/foo_av1.mp4"));
    }

    #[test]
    fn is_downloadable_done_pass() {
        assert!(is_downloadable("done", Some("pass")));
    }

    #[test]
    fn is_downloadable_done_empty() {
        assert!(is_downloadable("done", Some("")));
    }

    #[test]
    fn is_downloadable_done_none() {
        assert!(is_downloadable("done", None));
    }

    #[test]
    fn is_downloadable_done_fail() {
        assert!(!is_downloadable("done", Some("fail")));
    }

    #[test]
    fn is_downloadable_not_done() {
        assert!(!is_downloadable("pending", Some("pass")));
        assert!(!is_downloadable("active", Some("pass")));
        assert!(!is_downloadable("failed", Some("pass")));
    }

    #[test]
    fn parse_fps_simple() {
        assert_eq!(parse_fps("30"), 30.0);
    }

    #[test]
    fn parse_fps_fraction() {
        assert_eq!(parse_fps("30000/1001"), 30000.0 / 1001.0);
    }
}
