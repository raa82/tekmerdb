mod config;
mod logger;
mod scheduler;

use std::fs;
use json_comments::StripComments;
use config::{SystemJobsFile, UserJobsFile};

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();

    let system_jobs_path = arg(&args, "--system-jobs").unwrap_or_else(|| "system_jobs.json".to_string());
    let user_jobs_path   = arg(&args, "--user-jobs").unwrap_or_else(|| "user_jobs.json".to_string());
    let log_dir          = arg(&args, "--log-dir").unwrap_or_else(|| "log".to_string());

    // Fail fast if either file is unreadable or invalid JSON before entering the reload loop
    load_json::<SystemJobsFile>(&system_jobs_path);
    load_json::<UserJobsFile>(&user_jobs_path);

    println!("[cron] system-jobs={} user-jobs={} log-dir={}",
        system_jobs_path, user_jobs_path, log_dir);
    println!("[cron] watching for config changes every 5s");

    scheduler::run(system_jobs_path, user_jobs_path, log_dir).await;
}

fn load_json<T: serde::de::DeserializeOwned>(path: &str) -> T {
    let raw = fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("[cron] cannot read {}: {}", path, e);
        std::process::exit(1);
    });
    let stripped = StripComments::new(raw.as_bytes());
    serde_json::from_reader(stripped).unwrap_or_else(|e| {
        eprintln!("[cron] invalid JSON in {}: {}", path, e);
        std::process::exit(1);
    })
}

fn arg(args: &[String], flag: &str) -> Option<String> {
    args.windows(2)
        .find(|w| w[0] == flag)
        .map(|w| w[1].clone())
}
