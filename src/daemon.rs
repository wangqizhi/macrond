use crate::config;
use crate::logging;
use crate::model::{DaemonState, ExecutionRecord, JobConfig, JobView};
use crate::paths::AppPaths;
use crate::scheduler;
use anyhow::{Context, Result, anyhow};
use chrono::Local;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio::time::{Duration, interval};
use uuid::Uuid;

pub async fn run_daemon(paths: AppPaths) -> Result<()> {
    paths.ensure_dirs()?;
    if let Some(pid) = read_pid(&paths.pid_file)? {
        if is_pid_running(pid) {
            return Err(anyhow!("daemon is already running with pid {pid}"));
        }
    }

    write_pid(&paths.pid_file)?;
    let _pid_guard = PidGuard {
        path: paths.pid_file.clone(),
    };

    logging::log_daemon(&paths.logs_dir, "INFO", "daemon started")?;
    logging::cleanup_old_logs(&paths.logs_dir, 30)?;

    let mut last_reload_error: Option<String> = None;
    let mut jobs = match config::load_jobs(&paths.jobs_dir) {
        Ok(v) => v,
        Err(err) => {
            let msg = format!("initial load failed: {err:#}");
            logging::log_daemon(&paths.logs_dir, "ERROR", &msg)?;
            last_reload_error = Some(msg);
            Vec::new()
        }
    };

    let mut next_runs = compute_next_runs(&jobs);
    let mut last_result: HashMap<String, ExecutionRecord> = HashMap::new();
    let mut recent_runs: Vec<ExecutionRecord> = Vec::new();

    let (tx_run, mut rx_run) = mpsc::channel::<ExecutionRecord>(256);

    let (event_tx, event_rx) = std::sync::mpsc::channel();
    let watcher = setup_watcher(&paths.jobs_dir, event_tx)?;

    let mut ticker = interval(Duration::from_secs(1));
    let mut cleanup_tick = interval(Duration::from_secs(3600));

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                let has_reload = drain_watcher(&event_rx);
                if has_reload {
                    match config::load_jobs(&paths.jobs_dir) {
                        Ok(v) => {
                            jobs = v;
                            next_runs = compute_next_runs(&jobs);
                            last_reload_error = None;
                            logging::log_daemon(&paths.logs_dir, "INFO", "jobs reloaded")?;
                        }
                        Err(err) => {
                            let msg = format!("reload failed: {err:#}");
                            last_reload_error = Some(msg.clone());
                            logging::log_daemon(&paths.logs_dir, "ERROR", &msg)?;
                        }
                    }
                }

                for job_id in collect_requests(&paths.requests_dir)? {
                    if let Some(job) = jobs.iter().find(|j| j.id == job_id && j.enabled).cloned() {
                        spawn_job(job, "manual", paths.clone(), tx_run.clone());
                    }
                }

                let now = Local::now();
                for job in &jobs {
                    let should_run = match next_runs.get(&job.id).and_then(|t| *t) {
                        Some(ts) => ts <= now,
                        None => false,
                    };
                    if should_run {
                        spawn_job(job.clone(), "schedule", paths.clone(), tx_run.clone());
                        let next = scheduler::next_run_after(job, now + chrono::TimeDelta::seconds(1)).ok().flatten();
                        next_runs.insert(job.id.clone(), next);
                    }
                }

                while let Ok(record) = rx_run.try_recv() {
                    last_result.insert(record.job_id.clone(), record.clone());
                    recent_runs.push(record);
                    if recent_runs.len() > 100 {
                        let drop_count = recent_runs.len() - 100;
                        recent_runs.drain(0..drop_count);
                    }
                }

                write_state(
                    &paths,
                    std::process::id(),
                    &jobs,
                    &next_runs,
                    &last_result,
                    &recent_runs,
                    last_reload_error.clone(),
                )?;
            }
            _ = cleanup_tick.tick() => {
                logging::cleanup_old_logs(&paths.logs_dir, 30)?;
            }
            _ = tokio::signal::ctrl_c() => {
                break;
            }
        }
    }

    drop(watcher);
    logging::log_daemon(&paths.logs_dir, "INFO", "daemon stopped")?;
    Ok(())
}

pub async fn run_job_inline(paths: &AppPaths, job_id: &str) -> Result<ExecutionRecord> {
    let jobs = config::load_jobs(&paths.jobs_dir)?;
    let job = jobs
        .into_iter()
        .find(|j| j.id == job_id)
        .ok_or_else(|| anyhow!("job not found: {job_id}"))?;

    execute_job(paths.clone(), job, "manual-inline").await
}

fn compute_next_runs(jobs: &[JobConfig]) -> HashMap<String, Option<chrono::DateTime<Local>>> {
    let now = Local::now();
    let mut map = HashMap::new();
    for job in jobs {
        let next = scheduler::next_run_after(job, now).ok().flatten();
        map.insert(job.id.clone(), next);
    }
    map
}

fn setup_watcher(
    jobs_dir: &Path,
    event_tx: std::sync::mpsc::Sender<notify::Result<notify::Event>>,
) -> Result<RecommendedWatcher> {
    let mut watcher = notify::recommended_watcher(move |res| {
        let _ = event_tx.send(res);
    })?;
    watcher.watch(jobs_dir, RecursiveMode::NonRecursive)?;
    Ok(watcher)
}

fn drain_watcher(event_rx: &std::sync::mpsc::Receiver<notify::Result<notify::Event>>) -> bool {
    let mut changed = false;
    while let Ok(event) = event_rx.try_recv() {
        if event.is_ok() {
            changed = true;
        }
    }
    changed
}

fn collect_requests(requests_dir: &Path) -> Result<Vec<String>> {
    let mut requests = Vec::new();

    for entry in std::fs::read_dir(requests_dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }

        let raw = std::fs::read_to_string(&path)?;
        #[derive(serde::Deserialize)]
        struct Req {
            job_id: String,
        }
        if let Ok(req) = serde_json::from_str::<Req>(&raw) {
            requests.push(req.job_id);
        }
        let _ = std::fs::remove_file(path);
    }

    Ok(requests)
}

fn spawn_job(job: JobConfig, trigger: &'static str, paths: AppPaths, tx: mpsc::Sender<ExecutionRecord>) {
    tokio::spawn(async move {
        if let Ok(record) = execute_job(paths, job, trigger).await {
            let _ = tx.send(record).await;
        }
    });
}

async fn execute_job(paths: AppPaths, job: JobConfig, trigger: &str) -> Result<ExecutionRecord> {
    let run_id = Uuid::new_v4().to_string();
    let started_at = Local::now();

    logging::log_job(
        &paths.logs_dir,
        "INFO",
        &job.id,
        &run_id,
        &format!("event=start trigger={trigger} command={}", job.command.program),
    )?;

    let mut command = Command::new(&job.command.program);
    command.args(&job.command.args);
    command.stdin(Stdio::null());
    command.stdout(Stdio::null());
    command.stderr(Stdio::null());
    if let Some(working_dir) = &job.command.working_dir {
        command.current_dir(working_dir);
    }
    command.envs(&job.command.env);

    let timeout = Duration::from_secs(job.timeout_seconds.max(1));
    let mut child = command
        .spawn()
        .with_context(|| format!("spawn failed for job {}", job.id))?;

    let (status, exit_code, message) = match tokio::time::timeout(timeout, child.wait()).await {
        Ok(Ok(exit)) => {
            if exit.success() {
                ("success".to_string(), exit.code(), "event=success".to_string())
            } else {
                (
                    "failed".to_string(),
                    exit.code(),
                    format!("event=failed exit_code={}", exit.code().unwrap_or(-1)),
                )
            }
        }
        Ok(Err(err)) => (
            "failed".to_string(),
            None,
            format!("event=failed message=wait-error:{err}"),
        ),
        Err(_) => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            ("timeout".to_string(), None, "event=timeout".to_string())
        }
    };

    let ended_at = Local::now();
    logging::log_job(&paths.logs_dir, if status == "success" { "INFO" } else { "ERROR" }, &job.id, &run_id, &message)?;

    Ok(ExecutionRecord {
        run_id,
        job_id: job.id,
        trigger: trigger.to_string(),
        started_at,
        ended_at,
        status,
        exit_code,
        message,
    })
}

fn write_state(
    paths: &AppPaths,
    pid: u32,
    jobs: &[JobConfig],
    next_runs: &HashMap<String, Option<chrono::DateTime<Local>>>,
    last_result: &HashMap<String, ExecutionRecord>,
    recent_runs: &[ExecutionRecord],
    last_reload_error: Option<String>,
) -> Result<()> {
    let mut views = Vec::new();
    for job in jobs {
        views.push(JobView {
            id: job.id.clone(),
            name: job.name.clone(),
            enabled: job.enabled,
            schedule: scheduler::schedule_label(job),
            next_run: next_runs.get(&job.id).cloned().flatten(),
            last_result: last_result.get(&job.id).cloned(),
        });
    }

    let state = DaemonState {
        updated_at: Local::now(),
        pid,
        running: true,
        last_reload_error,
        jobs: views,
        recent_runs: recent_runs.to_vec(),
    };

    let content = serde_json::to_string_pretty(&state)?;
    std::fs::write(&paths.state_file, content)?;
    Ok(())
}

fn write_pid(path: &Path) -> Result<()> {
    let pid = std::process::id();
    let mut file = OpenOptions::new().create(true).truncate(true).write(true).open(path)?;
    file.write_all(pid.to_string().as_bytes())?;
    Ok(())
}

fn read_pid(path: &Path) -> Result<Option<i32>> {
    if !path.exists() {
        return Ok(None);
    }
    let s = std::fs::read_to_string(path)?;
    let pid = s.trim().parse::<i32>().ok();
    Ok(pid)
}

fn is_pid_running(pid: i32) -> bool {
    nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None).is_ok()
}

struct PidGuard {
    path: std::path::PathBuf,
}

impl Drop for PidGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

pub fn daemon_running(paths: &AppPaths) -> Result<Option<i32>> {
    let Some(pid) = read_pid(&paths.pid_file)? else {
        return Ok(None);
    };

    if is_pid_running(pid) {
        Ok(Some(pid))
    } else {
        Ok(None)
    }
}

pub fn submit_run_request(paths: &AppPaths, job_id: &str) -> Result<()> {
    let req_id = Uuid::new_v4().to_string();
    let path = paths.requests_dir.join(format!("{req_id}.json"));
    let payload = serde_json::json!({ "job_id": job_id });
    std::fs::write(path, serde_json::to_vec(&payload)?)?;
    Ok(())
}
