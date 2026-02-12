use anyhow::Result;
use chrono::{Datelike, Local, NaiveDate};
use std::fs::{OpenOptions, read_dir, remove_file};
use std::io::Write;
use std::path::Path;

pub fn log_daemon(logs_dir: &Path, level: &str, message: &str) -> Result<()> {
    write_line(logs_dir, "daemon", level, None, None, message)
}

pub fn log_job(
    logs_dir: &Path,
    level: &str,
    job_id: &str,
    run_id: &str,
    message: &str,
) -> Result<()> {
    write_line(logs_dir, "job", level, Some(job_id), Some(run_id), message)
}

fn write_line(
    logs_dir: &Path,
    prefix: &str,
    level: &str,
    job_id: Option<&str>,
    run_id: Option<&str>,
    message: &str,
) -> Result<()> {
    let now = Local::now();
    let filename = format!("{}-{:04}-{:02}-{:02}.log", prefix, now.year(), now.month(), now.day());
    let path = logs_dir.join(filename);
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;

    let mut line = format!("{} {}", now.format("%Y-%m-%d %H:%M:%S%:z"), level);
    if let Some(id) = job_id {
        line.push_str(&format!(" job_id={id}"));
    }
    if let Some(id) = run_id {
        line.push_str(&format!(" run_id={id}"));
    }
    line.push(' ');
    line.push_str(message);
    line.push('\n');

    file.write_all(line.as_bytes())?;
    Ok(())
}

pub fn cleanup_old_logs(logs_dir: &Path, keep_days: i64) -> Result<()> {
    let today = Local::now().date_naive();
    for entry in read_dir(logs_dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let Some(file_name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };

        let Some(date_str) = file_name
            .strip_prefix("daemon-")
            .or_else(|| file_name.strip_prefix("job-"))
            .and_then(|s| s.strip_suffix(".log"))
        else {
            continue;
        };

        let Ok(date) = NaiveDate::parse_from_str(date_str, "%Y-%m-%d") else {
            continue;
        };

        if (today - date).num_days() > keep_days {
            let _ = remove_file(path);
        }
    }

    Ok(())
}
