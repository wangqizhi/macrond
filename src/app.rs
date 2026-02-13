ause crate::cli::{Cli, Command};
use crate::config;
use crate::daemon;
use crate::model::DaemonState;
use crate::paths::AppPaths;
use crate::scheduler;
use crate::tui;
use anyhow::{Context, Result, anyhow, bail};
use chrono::Local;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::process::Stdio;

pub async fn run(cli: Cli) -> Result<()> {
    let paths = AppPaths::new(&cli.base_dir)?;
    paths.ensure_dirs()?;

    match cli.command.unwrap_or(Command::Tui) {
        Command::Version => version(),
        Command::Start => start(&paths),
        Command::Stop => stop(&paths),
        Command::Status => status(&paths),
        Command::List => list(&paths),
        Command::Logs { job, tail } => logs(&paths, job.as_deref(), tail),
        Command::Run { job_id } => run_job(&paths, &job_id).await,
        Command::Tui => tui::run_tui(&paths),
        Command::Daemon => daemon::run_daemon(paths).await,
    }
}

fn version() -> Result<()> {
    println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
    Ok(())
}

fn start(paths: &AppPaths) -> Result<()> {
    if let Some(pid) = daemon::daemon_running(paths)? {
        println!("daemon is already running (pid={pid})");
        return Ok(());
    }

    let exe = std::env::current_exe().context("resolve current exe")?;
    let child = std::process::Command::new(exe)
        .arg("--base-dir")
        .arg(&paths.base_dir)
        .arg("daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("failed to spawn daemon")?;

    println!("daemon started (pid={})", child.id());
    Ok(())
}

fn stop(paths: &AppPaths) -> Result<()> {
    let Some(pid) = daemon::daemon_running(paths)? else {
        println!("daemon is not running");
        return Ok(());
    };

    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(pid),
        Some(nix::sys::signal::Signal::SIGINT),
    )
    .context("failed to send SIGINT")?;
    println!("stop signal sent to pid={pid}");
    Ok(())
}

fn status(paths: &AppPaths) -> Result<()> {
    if let Some(pid) = daemon::daemon_running(paths)? {
        println!("daemon: running (pid={pid})");
    } else {
        println!("daemon: stopped");
    }

    if paths.state_file.exists() {
        let state = read_state(paths)?;
        println!("updated_at: {}", state.updated_at.format("%Y-%m-%d %H:%M:%S"));
        println!("loaded_jobs: {}", state.jobs.len());
        if let Some(err) = state.last_reload_error {
            println!("last_reload_error: {err}");
        }
    } else {
        println!("state: unavailable");
    }

    Ok(())
}

fn list(paths: &AppPaths) -> Result<()> {
    if paths.state_file.exists() {
        let state = read_state(paths)?;
        if state.jobs.is_empty() {
            println!("no jobs loaded");
            return Ok(());
        }
        for job in state.jobs {
            let next = job
                .next_run
                .map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string())
                .unwrap_or_else(|| "-".to_string());
            let last = job
                .last_result
                .as_ref()
                .map(|r| format!("{}({})", r.status, r.ended_at.format("%m-%d %H:%M:%S")))
                .unwrap_or_else(|| "-".to_string());
            println!(
                "id={} enabled={} schedule={} next_run={} last={}",
                job.id, job.enabled, job.schedule, next, last
            );
        }
        return Ok(());
    }

    let jobs = config::load_jobs(&paths.jobs_dir)?;
    if jobs.is_empty() {
        println!("no jobs found in jobs/");
        return Ok(());
    }
    let now = Local::now();
    for job in jobs {
        let next = scheduler::next_run_after(&job, now)?.map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string());
        println!(
            "id={} enabled={} schedule={} next_run={}",
            job.id,
            job.enabled,
            scheduler::schedule_label(&job),
            next.unwrap_or_else(|| "-".to_string())
        );
    }
    Ok(())
}

fn logs(paths: &AppPaths, job_id: Option<&str>, tail: usize) -> Result<()> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(&paths.logs_dir)? {
        let entry = entry?;
        if entry.path().is_file() {
            files.push(entry.path());
        }
    }
    files.sort();

    if files.is_empty() {
        println!("no logs found");
        return Ok(());
    }

    let latest = files.last().ok_or_else(|| anyhow!("no log file"))?;
    let file = File::open(latest)?;
    let reader = BufReader::new(file);
    let mut lines: Vec<String> = reader.lines().collect::<std::result::Result<Vec<_>, _>>()?;

    if let Some(job) = job_id {
        lines.retain(|line| line.contains(&format!("job_id={job}")));
    }

    let start = lines.len().saturating_sub(tail);
    for line in &lines[start..] {
        println!("{line}");
    }

    Ok(())
}

async fn run_job(paths: &AppPaths, job_id: &str) -> Result<()> {
    let jobs = config::load_jobs(&paths.jobs_dir)?;
    if !jobs.iter().any(|j| j.id == job_id) {
        bail!("job not found: {job_id}");
    }

    let force_inline = std::env::var("EZCRON_FORCE_INLINE").ok().as_deref() == Some("1");
    if daemon::daemon_running(paths)?.is_some() && !force_inline {
        daemon::submit_run_request(paths, job_id)?;
        println!("run request submitted for job={job_id}");
        return Ok(());
    }

    let record = daemon::run_job_inline(paths, job_id).await?;
    println!(
        "job={} status={} exit_code={:?} ended_at={}",
        record.job_id,
        record.status,
        record.exit_code,
        record.ended_at.format("%Y-%m-%d %H:%M:%S")
    );
    Ok(())
}

fn read_state(paths: &AppPaths) -> Result<DaemonState> {
    let raw = std::fs::read_to_string(&paths.state_file)?;
    let state = serde_json::from_str(&raw).context("parse state file")?;
    Ok(state)
}
