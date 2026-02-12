use crate::model::{JobConfig, Repeat, ScheduleConfig};
use anyhow::{Context, Result, anyhow, bail};
use std::collections::HashSet;
use std::path::Path;
use std::str::FromStr;

pub fn load_jobs(jobs_dir: &Path) -> Result<Vec<JobConfig>> {
    let mut jobs = Vec::new();
    let mut ids = HashSet::new();

    if !jobs_dir.exists() {
        return Ok(jobs);
    }

    for entry in std::fs::read_dir(jobs_dir).context("read jobs dir")? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }

        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("read job file {}", path.display()))?;
        let job: JobConfig = serde_json::from_str(&raw)
            .with_context(|| format!("parse job file {}", path.display()))?;
        validate_job(&job).with_context(|| format!("invalid job {}", job.id))?;

        if !ids.insert(job.id.clone()) {
            bail!("duplicate job id: {}", job.id);
        }

        jobs.push(job);
    }

    jobs.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(jobs)
}

fn validate_job(job: &JobConfig) -> Result<()> {
    if job.id.trim().is_empty() {
        bail!("job.id is required");
    }
    if job.name.trim().is_empty() {
        bail!("job.name is required");
    }
    if job.command.program.trim().is_empty() {
        bail!("command.program is required");
    }

    match &job.schedule {
        ScheduleConfig::Cron { expression } => {
            let _ = cron::Schedule::from_str(expression)
                .map_err(|e| anyhow!("invalid cron expression: {e}"))?;
        }
        ScheduleConfig::Simple {
            repeat,
            time,
            weekday,
            day,
            once_at,
        } => {
            match repeat {
                Repeat::Daily => {
                    validate_hhmm(time.as_deref())?;
                }
                Repeat::Weekly => {
                    let w = weekday.ok_or_else(|| anyhow!("weekday is required for weekly"))?;
                    if !(1..=7).contains(&w) {
                        bail!("weekday must be 1..=7");
                    }
                    validate_hhmm(time.as_deref())?;
                }
                Repeat::Monthly => {
                    let d = day.ok_or_else(|| anyhow!("day is required for monthly"))?;
                    if !(1..=31).contains(&d) {
                        bail!("day must be 1..=31");
                    }
                    validate_hhmm(time.as_deref())?;
                }
                Repeat::EveryMinute => {
                    if time.is_some() {
                        bail!("time is not allowed for everyminute");
                    }
                }
                Repeat::Once => {
                    let once = once_at
                        .as_deref()
                        .ok_or_else(|| anyhow!("once_at is required for once"))?;
                    chrono::NaiveDateTime::parse_from_str(once, "%Y-%m-%d %H:%M")
                        .map_err(|e| anyhow!("invalid once_at format: {e}"))?;
                }
            }
        }
    }

    Ok(())
}

fn validate_hhmm(time: Option<&str>) -> Result<()> {
    let time = time.ok_or_else(|| anyhow!("time is required"))?;
    let parts: Vec<&str> = time.split(':').collect();
    if parts.len() != 2 {
        bail!("simple.time must be HH:MM");
    }
    let hour: u32 = parts[0].parse().map_err(|_| anyhow!("invalid hour"))?;
    let minute: u32 = parts[1].parse().map_err(|_| anyhow!("invalid minute"))?;
    if hour > 23 || minute > 59 {
        bail!("simple.time out of range");
    }
    Ok(())
}
