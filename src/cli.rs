use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "macrond", version, about = "macOS-friendly cron daemon")]
pub struct Cli {
    #[arg(long, default_value = ".")]
    pub base_dir: PathBuf,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    Version,
    Start,
    Stop,
    Status,
    List,
    Logs {
        #[arg(long)]
        job: Option<String>,
        #[arg(long, default_value_t = 50)]
        tail: usize,
    },
    Run {
        job_id: String,
    },
    Tui,
    Daemon,
}
