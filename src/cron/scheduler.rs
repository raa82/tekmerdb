use chrono::Utc;
use croner::Cron;
use std::time::SystemTime;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::{sleep, Duration};

use crate::config::{Job, OnFailure, SystemJobsFile, UserJobsFile};
use crate::logger::log;

pub async fn run(system_path: String, user_path: String, log_dir: String) {
    let mut handles: Vec<JoinHandle<()>> = vec![];
    let mut stop_tx: Option<watch::Sender<bool>> = None;
    let mut last_mtimes  = (SystemTime::UNIX_EPOCH, SystemTime::UNIX_EPOCH);
    let mut active_jobs: (Vec<Job>, Vec<Job>) = (vec![], vec![]);

    loop {
        let mtimes = (file_mtime(&system_path), file_mtime(&user_path));

        if mtimes != last_mtimes {
            match reload(&system_path, &user_path) {
                Ok((system_jobs, user_jobs)) => {
                    // Only act if the content actually changed.
                    if system_jobs == active_jobs.0 && user_jobs == active_jobs.1 {
                        last_mtimes = mtimes;
                        sleep(Duration::from_secs(5)).await;
                        continue;
                    }

                    log_diff(&log_dir, &active_jobs.0, &system_jobs, "system");
                    log_diff(&log_dir, &active_jobs.1, &user_jobs,   "user");

                    // Signal running loops to stop; let mid-command runs finish.
                    if let Some(tx) = &stop_tx {
                        let _ = tx.send(true);
                    }
                    for h in handles.drain(..) {
                        let _ = h.await;
                    }

                    let enabled_sys  = system_jobs.iter().filter(|j| j.enabled).count();
                    let enabled_user = user_jobs.iter().filter(|j| j.enabled).count();
                    log(&log_dir, "cron", &format!(
                        "config applied — system {} ({} enabled), user {} ({} enabled)",
                        system_jobs.len(), enabled_sys,
                        user_jobs.len(),   enabled_user,
                    ));

                    let (tx, rx) = watch::channel(false);
                    stop_tx = Some(tx);

                    for job in system_jobs.iter().chain(user_jobs.iter()).filter(|j| j.enabled) {
                        let log_dir = log_dir.clone();
                        let rx      = rx.clone();
                        let job     = job.clone();
                        handles.push(tokio::spawn(async move {
                            job_loop(job, log_dir, rx).await;
                        }));
                    }

                    active_jobs = (system_jobs, user_jobs);
                    last_mtimes = mtimes;
                }
                Err(e) => {
                    log(&log_dir, "cron", &format!(
                        "config reload failed (keeping current jobs): {}", e
                    ));
                    last_mtimes = mtimes;
                }
            }
        }

        sleep(Duration::from_secs(5)).await;
    }
}

fn log_diff(log_dir: &str, old: &[Job], new: &[Job], source: &str) {
    for job in new {
        match old.iter().find(|j| j.name == job.name) {
            None => log(log_dir, "cron", &format!(
                "[{}] job added: {}", source, job.name
            )),
            Some(prev) => {
                let mut changes: Vec<&str> = vec![];
                if prev.enabled    != job.enabled    { changes.push("enabled"); }
                if prev.cron_expr  != job.cron_expr  { changes.push("cron_expr"); }
                if prev.command    != job.command     { changes.push("command"); }
                if prev.timeout_secs != job.timeout_secs { changes.push("timeout_secs"); }
                if prev.on_failure != job.on_failure  { changes.push("on_failure"); }
                if !changes.is_empty() {
                    log(log_dir, "cron", &format!(
                        "[{}] job updated: {} ({})", source, job.name, changes.join(", ")
                    ));
                }
            }
        }
    }
    for job in old {
        if !new.iter().any(|j| j.name == job.name) {
            log(log_dir, "cron", &format!(
                "[{}] job removed: {}", source, job.name
            ));
        }
    }
}

fn file_mtime(path: &str) -> SystemTime {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH)
}

fn reload(system_path: &str, user_path: &str) -> anyhow::Result<(Vec<Job>, Vec<Job>)> {
    use json_comments::StripComments;

    let sys_raw  = std::fs::read_to_string(system_path)?;
    let sys_file: SystemJobsFile =
        serde_json::from_reader(StripComments::new(sys_raw.as_bytes()))?;

    let usr_raw  = std::fs::read_to_string(user_path)?;
    let usr_file: UserJobsFile =
        serde_json::from_reader(StripComments::new(usr_raw.as_bytes()))?;

    Ok((sys_file.jobs, usr_file.jobs))
}

async fn job_loop(job: Job, log_dir: String, mut stop_rx: watch::Receiver<bool>) {
    let schedule = match Cron::new(&job.cron_expr).parse() {
        Ok(s)  => s,
        Err(e) => {
            eprintln!("[cron] [{}] invalid cron_expr '{}': {}", job.name, job.cron_expr, e);
            return;
        }
    };

    log(&log_dir, &job.name, &format!("scheduled ({})", job.cron_expr));

    loop {
        let now  = Utc::now();
        let next = match schedule.find_next_occurrence(&now, false) {
            Ok(t)  => t,
            Err(e) => {
                eprintln!("[cron] [{}] schedule error: {}", job.name, e);
                sleep(Duration::from_secs(60)).await;
                continue;
            }
        };

        let delay_ms = (next - now).num_milliseconds().max(0) as u64;

        // Sleep until next fire — stop signal exits immediately here.
        tokio::select! {
            _ = sleep(Duration::from_millis(delay_ms)) => {}
            _ = stop_rx.changed() => {
                log(&log_dir, &job.name, "stopping");
                return;
            }
        }

        // Run the command to completion — stop signal is not checked mid-run.
        log(&log_dir, &job.name, "started");
        match run_command(&job, &log_dir).await {
            Ok(_)  => log(&log_dir, &job.name, "finished"),
            Err(e) => {
                log(&log_dir, &job.name, &format!("failed: {}", e));
                if job.on_failure == OnFailure::Stop {
                    eprintln!("[cron] [{}] on_failure=stop — shutting down", job.name);
                    std::process::exit(1);
                }
            }
        }

        // If stop arrived while the command was running, exit now that it's done.
        if *stop_rx.borrow() {
            log(&log_dir, &job.name, "stopping after run");
            return;
        }
    }
}

async fn run_command(job: &Job, log_dir: &str) -> anyhow::Result<()> {
    use tokio::process::Command;

    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(&job.command);

    let timeout = Duration::from_secs(job.timeout_secs);
    let result  = tokio::time::timeout(timeout, cmd.output()).await;

    match result {
        Err(_)         => anyhow::bail!("timed out after {}s", job.timeout_secs),
        Ok(Err(e))     => anyhow::bail!("spawn error: {}", e),
        Ok(Ok(output)) => {
            if !output.stdout.is_empty() {
                log(log_dir, &job.name, &format!("stdout: {}",
                    String::from_utf8_lossy(&output.stdout).trim()));
            }
            if !output.stderr.is_empty() {
                log(log_dir, &job.name, &format!("stderr: {}",
                    String::from_utf8_lossy(&output.stderr).trim()));
            }
            if !output.status.success() {
                anyhow::bail!("exit code {:?}", output.status.code());
            }
            Ok(())
        }
    }
}
