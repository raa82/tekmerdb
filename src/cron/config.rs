use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Deserialize, Clone)]
pub struct SystemJob {
    pub name: String,
    pub enabled: bool,
    pub cron_expr: String,
    pub job_fn: String,
    #[serde(default)]
    pub params: HashMap<String, String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct UserJob {
    pub name: String,
    pub enabled: bool,
    pub cron_expr: String,
    pub command: String,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    #[serde(default = "default_on_failure")]
    pub on_failure: OnFailure,
}

#[derive(Debug, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum OnFailure {
    Continue,
    Stop,
}

fn default_timeout() -> u64 {
    60
}

fn default_on_failure() -> OnFailure {
    OnFailure::Continue
}

#[derive(Debug, Deserialize)]
pub struct SystemJobsFile {
    pub jobs: Vec<SystemJob>,
}

#[derive(Debug, Deserialize)]
pub struct UserJobsFile {
    pub jobs: Vec<UserJob>,
}

pub const KNOWN_JOB_FNS: &[&str] = &[
    "rotate_crb_files",
    "health_check",
    "archive_logs",
];
