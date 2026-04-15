use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "baraddur",
    version,
    about = "Project-agnostic file watcher that surfaces issues before CI"
)]
struct Cli {
    /// Config file path
    #[arg(short = 'c', long)]
    config: Option<PathBuf>,

    /// Directory to watch [default: current directory]
    #[arg(short = 'w', long)]
    watch_dir: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let (config, config_path) = baraddur::config::load(cli.config.as_deref())?;

    let root = match cli.watch_dir {
        Some(p) => p,
        None => std::env::current_dir()?,
    };

    let app = baraddur::App {
        config,
        config_path,
        root,
    };

    app.run().await
}
