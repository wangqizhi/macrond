mod app;
mod cli;
mod config;
mod daemon;
mod logging;
mod model;
mod paths;
mod scheduler;
mod tui;

use clap::Parser;

#[tokio::main]
async fn main() {
    if let Err(err) = app::run(cli::Cli::parse()).await {
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}
