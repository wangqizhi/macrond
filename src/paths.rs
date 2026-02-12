use anyhow::Result;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub base_dir: PathBuf,
    pub jobs_dir: PathBuf,
    pub logs_dir: PathBuf,
    pub run_dir: PathBuf,
    pub requests_dir: PathBuf,
    pub pid_file: PathBuf,
    pub state_file: PathBuf,
}

impl AppPaths {
    pub fn new(base_dir: impl AsRef<Path>) -> Result<Self> {
        let base_dir = base_dir.as_ref().canonicalize()?;
        let jobs_dir = base_dir.join("jobs");
        let logs_dir = base_dir.join("logs");
        let run_dir = base_dir.join("run");
        let requests_dir = run_dir.join("requests");
        let pid_file = run_dir.join("daemon.pid");
        let state_file = run_dir.join("state.json");
        Ok(Self {
            base_dir,
            jobs_dir,
            logs_dir,
            run_dir,
            requests_dir,
            pid_file,
            state_file,
        })
    }

    pub fn ensure_dirs(&self) -> Result<()> {
        std::fs::create_dir_all(&self.jobs_dir)?;
        std::fs::create_dir_all(&self.logs_dir)?;
        std::fs::create_dir_all(&self.run_dir)?;
        std::fs::create_dir_all(&self.requests_dir)?;
        Ok(())
    }
}
