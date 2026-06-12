use serde::Deserialize;
use std::path::Path;

#[derive(Deserialize)]
pub struct CronConfig {
    pub jobs: Vec<CronJob>,
}

#[derive(Deserialize)]
pub struct CronJob {
    pub name: String,
    pub command: String,
    pub schedule: String, // e.g., "*/5 * * * *"
}

impl CronConfig {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: CronConfig = serde_json::from_str(&content)?;
        Ok(config)
    }
}
