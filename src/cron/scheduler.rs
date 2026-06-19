use chrono::Utc;
use croner::Cron;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::time::{sleep, Duration};

use crate::config::{SystemJob, UserJob, OnFailure};
use crate::jobs::run_system_job;
use crate::logger::log;

pub async fn run(
    system_jobs: Vec<SystemJob>,
    user_jobs: Vec<UserJob>,
    data_dir: String,
    log_dir: String,
) {
    let mut handles = vec![];

    for job in system_jobs.into_iter().filter(|j| j.enabled) {
        let data_dir = data_dir.clone();
        let log_dir  = log_dir.clone();
        handles.push(tokio::spawn(async move {
            system_loop(job, data_dir, log_dir).await;
        }));
    }

    for job in user_jobs.into_iter().filter(|j| j.enabled) {
        let log_dir = log_dir.clone();
        handles.push(tokio::spawn(async move {
            user_loop(job, log_dir).await;
        }));
    }

    futures::future::join_all(handles).await;
}

async fn system_loop(job: SystemJob, data_dir: String, log_dir: String) {
    let schedule = match Cron::new(&job.cron_expr).parse() {
        Ok(s)  => s,
        Err(e) => {
            eprintln!("[cron] [{}] invalid cron_expr '{}': {}", job.name, job.cron_expr, e);
            return;
        }
    };

    let running = Arc::new(AtomicBool::new(false));
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
        sleep(Duration::from_millis(delay_ms)).await;

        if running
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            log(&log_dir, &job.name, "skipped — previous run still active");
            continue;
        }

        let job_clone    = job.clone();
        let data_dir_c   = data_dir.clone();
        let log_dir_c    = log_dir.clone();
        let running_flag = running.clone();

        tokio::spawn(async move {
            log(&log_dir_c, &job_clone.name, "started");
            tokio::task::spawn_blocking(move || {
                run_system_job(&job_clone, &data_dir_c, &log_dir_c);
            })
            .await
            .ok();
            running_flag.store(false, Ordering::SeqCst);
            log(&log_dir_c, &job_clone.name, "finished");
        });
    }
}

async fn user_loop(job: UserJob, log_dir: String) {
    let schedule = match Cron::new(&job.cron_expr).parse() {
        Ok(s)  => s,
        Err(e) => {
            eprintln!("[cron] [{}] invalid cron_expr '{}': {}", job.name, job.cron_expr, e);
            return;
        }
    };

    let running = Arc::new(AtomicBool::new(false));
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
        sleep(Duration::from_millis(delay_ms)).await;

        if running
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            log(&log_dir, &job.name, "skipped — previous run still active");
            continue;
        }

        let job_clone    = job.clone();
        let log_dir_c    = log_dir.clone();
        let running_flag = running.clone();

        tokio::spawn(async move {
            log(&log_dir_c, &job_clone.name, "started");
            let result = run_user_command(&job_clone, &log_dir_c).await;
            running_flag.store(false, Ordering::SeqCst);
            match result {
                Ok(_) => log(&log_dir_c, &job_clone.name, "finished"),
                Err(e) => {
                    log(&log_dir_c, &job_clone.name, &format!("failed: {}", e));
                    if job_clone.on_failure == OnFailure::Stop {
                        eprintln!("[cron] [{}] on_failure=stop — shutting down", job_clone.name);
                        std::process::exit(1);
                    }
                }
            }
        });
    }
}

async fn run_user_command(job: &UserJob, log_dir: &str) -> anyhow::Result<()> {
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
                log(log_dir, &job.name, &format!("stdout: {}", String::from_utf8_lossy(&output.stdout).trim()));
            }
            if !output.stderr.is_empty() {
                log(log_dir, &job.name, &format!("stderr: {}", String::from_utf8_lossy(&output.stderr).trim()));
            }
            if !output.status.success() {
                anyhow::bail!("exit code {:?}", output.status.code());
            }
            Ok(())
        }
    }
}
