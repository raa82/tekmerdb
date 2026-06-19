use std::fs::OpenOptions;
use std::io::Write;
use chrono::Utc;

pub fn log(log_dir: &str, job_name: &str, msg: &str) {
    let path = format!("{}/server.log", log_dir);
    let timestamp = Utc::now().format("%Y-%m-%dT%H:%M:%SZ");
    let line = format!("{} [CRON] [{}] {}\n", timestamp, job_name, msg);

    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = f.write_all(line.as_bytes());
    }

    print!("{}", line);
}
