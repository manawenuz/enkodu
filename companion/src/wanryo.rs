use anyhow::{Context, Result};
use indicatif::{ProgressBar, ProgressStyle};
use std::io::Write;
use std::path::PathBuf;

use crate::api;
use crate::config::Config;

struct Entry {
    job_id: String,
    filename: String,
    source: PathBuf,
    output: PathBuf,
    source_exists: bool,
    output_exists: bool,
    source_size: u64,
    output_size: u64, // server-reported if not yet downloaded
}

pub fn run(cfg: &Config) -> Result<()> {
    println!("wanryo: fetching done jobs from server...");
    let jobs = api::list_done_companion_jobs(&cfg.server_url, cfg.auth_token.as_deref())
        .context("fetch done jobs")?;

    let companion: Vec<_> = jobs
        .into_iter()
        .filter(|j| j.client_path.is_some())
        .collect();

    if companion.is_empty() {
        println!("No companion jobs found on server.");
        return Ok(());
    }

    // ── build per-job status ──────────────────────────────────────────────────
    let mut entries: Vec<Entry> = Vec::new();
    for job in companion {
        let client_path = job.client_path.as_deref().unwrap();
        let source = PathBuf::from(client_path);
        let stem = source
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("output");
        let output = source.with_file_name(format!("{}_av1.mp4", stem));
        let filename = source
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?")
            .to_string();

        let source_exists = source.exists();
        let output_exists = output.exists();
        let source_size = if source_exists {
            source.metadata().map(|m| m.len()).unwrap_or(0)
        } else {
            0
        };
        let output_size = if output_exists {
            output.metadata().map(|m| m.len()).unwrap_or(0)
        } else {
            job.output_size.unwrap_or(0)
        };

        entries.push(Entry {
            job_id: job.id,
            filename,
            source,
            output,
            source_exists,
            output_exists,
            source_size,
            output_size,
        });
    }

    // ── summary ───────────────────────────────────────────────────────────────
    let synced: Vec<&Entry> = entries.iter().filter(|e| e.output_exists).collect();
    let needs_download: Vec<&Entry> = entries
        .iter()
        .filter(|e| e.source_exists && !e.output_exists)
        .collect();
    let source_missing: Vec<&Entry> = entries
        .iter()
        .filter(|e| !e.source_exists && !e.output_exists)
        .collect();

    println!("\nStatus:");
    println!("  {} already synced", synced.len());
    println!(
        "  {} ready to download (source found, no AV1 beside it)",
        needs_download.len()
    );
    println!("  {} source missing (moved/deleted)", source_missing.len());

    // ── download prompt ───────────────────────────────────────────────────────
    let mut downloaded: Vec<&Entry> = Vec::new();
    let mut failed: Vec<(&Entry, String)> = Vec::new();

    if !needs_download.is_empty() {
        println!("\nFiles to download:");
        for e in &needs_download {
            println!(
                "  {} ({:.2} GB output)",
                e.filename,
                e.output_size as f64 / 1e9
            );
        }
        print!("\nDownload {} file(s)? [y/N] ", needs_download.len());
        std::io::stdout().flush()?;
        let mut ans = String::new();
        std::io::stdin().read_line(&mut ans)?;

        if ans.trim().eq_ignore_ascii_case("y") {
            for (i, e) in needs_download.iter().enumerate() {
                print!("[{}/{}] {}... ", i + 1, needs_download.len(), e.filename);
                std::io::stdout().flush()?;

                let bar = ProgressBar::new(e.output_size);
                bar.set_style(
                    ProgressStyle::with_template("{bar:30} {bytes}/{total_bytes} {eta}").unwrap(),
                );

                match api::download_output_with_retry(
                    &cfg.server_url,
                    cfg.auth_token.as_deref(),
                    &e.job_id,
                    &e.output,
                    &bar,
                ) {
                    Ok(_) => {
                        bar.finish_and_clear();
                        println!(
                            "ok ({:.2} GB)",
                            e.output.metadata().map(|m| m.len()).unwrap_or(0) as f64 / 1e9
                        );
                        downloaded.push(e);
                    }
                    Err(err) => {
                        bar.finish_and_clear();
                        println!("FAILED: {}", err);
                        failed.push((e, err.to_string()));
                    }
                }
            }
        }
    }

    // ── CSV checklist ─────────────────────────────────────────────────────────
    let csv_path = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Desktop")
        .join("enkodu_wanryo.csv");

    let mut out = String::new();
    out.push_str("filename,source_path,output_path,source_size_mb,output_size_mb,savings_pct,source_exists,output_exists,status,quality_ok,delete_source\n");

    for e in &entries {
        let status = if e.output_exists || downloaded.iter().any(|d| d.job_id == e.job_id) {
            "synced"
        } else if e.source_exists {
            "pending_download"
        } else {
            "source_missing"
        };

        let savings = if e.source_size > 0 && e.output_size > 0 {
            format!(
                "{:.0}%",
                (1.0 - e.output_size as f64 / e.source_size as f64) * 100.0
            )
        } else {
            "-".to_string()
        };

        out.push_str(&format!(
            "{},{},{},{:.1},{:.1},{},{},{},{},, \n",
            csv_escape(&e.filename),
            csv_escape(&e.source.to_string_lossy()),
            csv_escape(&e.output.to_string_lossy()),
            e.source_size as f64 / 1e6,
            e.output_size as f64 / 1e6,
            savings,
            if e.source_exists { "yes" } else { "no" },
            if e.output_exists || downloaded.iter().any(|d| d.job_id == e.job_id) {
                "yes"
            } else {
                "no"
            },
            status,
        ));
    }

    std::fs::write(&csv_path, out).context("write CSV")?;

    println!("\nChecklist saved to: {}", csv_path.display());
    if !failed.is_empty() {
        println!("{} download(s) failed:", failed.len());
        for (e, err) in &failed {
            println!("  {}: {}", e.filename, err);
        }
    }
    println!("Open the CSV, watch the files in a player, tick quality_ok, then mark delete_source=yes for what to delete.");

    Ok(())
}

fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}
