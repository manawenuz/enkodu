use std::process::Command;

fn main() {
    let hash = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_default();

    // Always embed a build timestamp so the binary is identifiable even without git.
    // Format: YYYYMMDD-HHMMSS (UTC)
    let ts = std::process::Command::new("date")
        .args(["-u", "+%Y%m%d-%H%M%S"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let build_id = if hash.is_empty() {
        ts
    } else {
        format!("{}-{}", hash, ts)
    };

    println!("cargo:rustc-env=GIT_HASH={}", build_id);
    println!("cargo:rerun-if-changed=build.rs");
}
