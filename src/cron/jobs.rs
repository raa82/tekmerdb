use std::fs;
use std::path::Path;
use chrono::Utc;
use crate::config::SystemJob;
use crate::logger::log;

pub fn run_system_job(job: &SystemJob, data_dir: &str, log_dir: &str) {
    match job.job_fn.as_str() {
        "rotate_crb_files" => rotate_crb_files(job, data_dir, log_dir),
        "health_check"     => health_check(job, data_dir, log_dir),
        "archive_logs"     => archive_logs(job, log_dir),
        unknown            => log(log_dir, &job.name, &format!("unknown job_fn: {}", unknown)),
    }
}

fn rotate_crb_files(job: &SystemJob, data_dir: &str, log_dir: &str) {
    let targets = [
        format!("{}/crb.bin", data_dir),
        format!("{}/source_crb.bin", data_dir),
    ];

    for path in &targets {
        if !Path::new(path).exists() {
            log(log_dir, &job.name, &format!("skipped {} — not found", path));
            continue;
        }
        let ts = Utc::now().format("%Y%m%dT%H%M%SZ");
        let backup = format!("{}.bak.{}", path, ts);
        match fs::rename(path, &backup) {
            Ok(_)  => log(log_dir, &job.name, &format!("rotated {} → {}", path, backup)),
            Err(e) => log(log_dir, &job.name, &format!("rotate failed for {}: {}", path, e)),
        }
    }
}

fn health_check(job: &SystemJob, data_dir: &str, log_dir: &str) {
    let targets = [
        format!("{}/crb.bin", data_dir),
        format!("{}/source_crb.bin", data_dir),
        format!("{}/cold.parquet", data_dir),
        format!("{}/sources.parquet", data_dir),
        format!("{}/hnsw.index", data_dir),
    ];

    for path in &targets {
        match fs::metadata(path) {
            Ok(m)  => log(log_dir, &job.name, &format!("OK {} ({} bytes)", path, m.len())),
            Err(_) => log(log_dir, &job.name, &format!("MISSING {}", path)),
        }
    }

    log(log_dir, &job.name, "health check complete");
}

fn archive_logs(job: &SystemJob, log_dir: &str) {
    let keep_days: i64 = job.params
        .get("keep_days")
        .and_then(|v| v.parse().ok())
        .unwrap_or(7);

    let cutoff = Utc::now() - chrono::Duration::days(keep_days);

    let dir = match fs::read_dir(log_dir) {
        Ok(d)  => d,
        Err(e) => {
            log(log_dir, &job.name, &format!("cannot read log dir: {}", e));
            return;
        }
    };

    for entry in dir.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("log") {
            continue;
        }
        if let Ok(meta) = fs::metadata(&path) {
            if let Ok(modified) = meta.modified() {
                let modified: chrono::DateTime<Utc> = modified.into();
                if modified < cutoff {
                    match fs::remove_file(&path) {
                        Ok(_)  => log(log_dir, &job.name, &format!("purged {}", path.display())),
                        Err(e) => log(log_dir, &job.name, &format!("purge failed {}: {}", path.display(), e)),
                    }
                }
            }
        }
    }

    log(log_dir, &job.name, "archive_logs complete");
}
