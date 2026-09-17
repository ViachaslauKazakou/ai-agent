//! Кроссплатформенный runner расписаний. Не зависит от launchd/systemd/Task Scheduler.
use crate::AppError;
use chrono::{DateTime, Utc};
use cron::Schedule;
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScheduleFile {
    pub jobs: Vec<Job>,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Job {
    pub name: String,
    pub cron: String,
    pub prompt: String,
    #[serde(default = "enabled")]
    pub enabled: bool,
}
fn enabled() -> bool {
    true
}
impl ScheduleFile {
    pub fn load(path: &Path) -> Result<Self, AppError> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| AppError::AgentConfig(format!("{}: {e}", path.display())))?;
        let file: Self = toml::from_str(&text).map_err(|e| AppError::AgentConfig(e.to_string()))?;
        for job in &file.jobs {
            job.schedule()?;
        }
        Ok(file)
    }
}
impl Job {
    pub fn schedule(&self) -> Result<Schedule, AppError> {
        self.cron
            .parse()
            .map_err(|e| AppError::InvalidConfig(format!("cron {}: {e}", self.name)))
    }
    pub fn next_after(&self, now: DateTime<Utc>) -> Result<DateTime<Utc>, AppError> {
        self.schedule()?.after(&now).next().ok_or_else(|| {
            AppError::InvalidConfig(format!("нет следующего запуска: {}", self.name))
        })
    }
}
pub async fn wait_for_next(file: &ScheduleFile) -> Result<Option<Job>, AppError> {
    let now = Utc::now();
    let next = file
        .jobs
        .iter()
        .filter(|j| j.enabled)
        .filter_map(|j| j.next_after(now).ok().map(|t| (t, j.clone())))
        .min_by_key(|(t, _)| *t);
    let Some((at, job)) = next else {
        return Ok(None);
    };
    let delay = (at - Utc::now()).to_std().unwrap_or_default();
    tokio::select! {
        _ = tokio::time::sleep(delay) => {}
        _ = tokio::signal::ctrl_c() => return Ok(None),
    }
    Ok(Some(job))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_cron_and_next_run() {
        let job = Job {
            name: "test".into(),
            cron: "0 * * * * * *".into(),
            prompt: "recap".into(),
            enabled: true,
        };
        assert!(job.next_after(Utc::now()).is_ok());
    }
}
