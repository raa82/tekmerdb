use tokio::time::Interval;
use std::time::Duration;
use crate::cron::config::CronConfig;
use crate::cron::job::CronJob;

pub struct CronEngine {
    config: CronConfig,
}

impl CronEngine {
    pub fn new(config: CronConfig) -> Self {
        Self { config }
    }

    pub async fn run(self) -> anyhow::Result<()> {
        // Spawn jobs
        for job in self.config.jobs {
            let interval = self.parse_interval(&job.schedule)?;
            let mut interval = tokio::time::interval(interval);
            tokio::spawn(async move {
                loop {
                    interval.tick().await;
                    // Execute job
                    // ...
                }
            });
        }
        Ok(())
    }

    fn parse_interval(&self, schedule: &str) -> anyhow::Result<Duration> {
        // Parse cron schedule to duration
        // For simplicity, assume schedule is a number of seconds
        let seconds: u64 = schedule.parse()?;
        Ok(Duration::from_secs(seconds))
    }
}
