mod config;
mod jobs;
mod logger;
mod scheduler;

use std::fs;
use config::{SystemJobsFile, UserJobsFile, KNOWN_JOB_FNS};

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();

    let system_jobs_path = arg(&args, "--system-jobs").unwrap_or_else(|| "system_jobs.json".to_string());
    let user_jobs_path   = arg(&args, "--user-jobs").unwrap_or_else(|| "user_jobs.json".to_string());
    let data_dir         = arg(&args, "--data-dir").unwrap_or_else(|| "data".to_string());
    let log_dir          = arg(&args, "--log-dir").unwrap_or_else(|| "log".to_string());

    println!("[cron] system-jobs={} user-jobs={} data-dir={} log-dir={}",
        system_jobs_path, user_jobs_path, data_dir, log_dir);

    let system_file: SystemJobsFile = load_json(&system_jobs_path);
    let user_file: UserJobsFile     = load_json(&user_jobs_path);

    // Fail fast on unknown job_fn — do not start with a misconfigured system job
    for job in &system_file.jobs {
        if !KNOWN_JOB_FNS.contains(&job.job_fn.as_str()) {
            eprintln!(
                "[cron] unknown job_fn '{}' in job '{}' — valid values: {:?}",
                job.job_fn, job.name, KNOWN_JOB_FNS
            );
            std::process::exit(1);
        }
    }

    let enabled_system = system_file.jobs.iter().filter(|j| j.enabled).count();
    let enabled_user   = user_file.jobs.iter().filter(|j| j.enabled).count();

    println!("[cron] system jobs: {} total, {} enabled", system_file.jobs.len(), enabled_system);
    println!("[cron] user jobs:   {} total, {} enabled", user_file.jobs.len(), enabled_user);
    println!("[cron] scheduler starting");

    scheduler::run(system_file.jobs, user_file.jobs, data_dir, log_dir).await;
}

fn load_json<T: serde::de::DeserializeOwned>(path: &str) -> T {
    let raw = fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("[cron] cannot read {}: {}", path, e);
        std::process::exit(1);
    });
    serde_json::from_str(&raw).unwrap_or_else(|e| {
        eprintln!("[cron] invalid JSON in {}: {}", path, e);
        std::process::exit(1);
    })
}

fn arg(args: &[String], flag: &str) -> Option<String> {
    args.windows(2)
        .find(|w| w[0] == flag)
        .map(|w| w[1].clone())
}
