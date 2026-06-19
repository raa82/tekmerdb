use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct Job {
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
    pub jobs: Vec<Job>,
}

#[derive(Debug, Deserialize)]
pub struct UserJobsFile {
    pub jobs: Vec<Job>,
}
