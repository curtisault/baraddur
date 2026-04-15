pub mod config;
pub mod output;
pub mod pipeline;
pub mod watcher;

use anyhow::Result;
use std::path::PathBuf;
use std::time::Duration;

use crate::output::{Display, PlainDisplay};

pub struct App {
    pub config: config::Config,
    pub config_path: PathBuf,
    pub root: PathBuf,
}

impl App {
    pub async fn run(self) -> Result<()> {
        eprintln!(
            "baraddur: watching {} (config: {})",
            self.root.display(),
            self.config_path.display()
        );

        let mut display: Box<dyn Display> = Box::new(PlainDisplay);

        // Run immediately on startup — don't wait for the first file change.
        pipeline::run_sequential(&self.config, &self.root, display.as_mut()).await?;

        let wcfg = watcher::WatchConfig {
            root: self.root.clone(),
            debounce: Duration::from_millis(self.config.watch.debounce_ms),
            extensions: self.config.watch.extensions.clone(),
            ignore: self.config.watch.ignore.clone(),
        };
        let mut rx = watcher::start(wcfg)?;

        loop {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {
                    eprintln!("\nbaraddur: exiting.");
                    return Ok(());
                }
                maybe = rx.recv() => {
                    match maybe {
                        None => {
                            eprintln!("baraddur: file watcher stopped unexpectedly. exiting.");
                            return Ok(());
                        }
                        Some(()) => {
                            // Drain any queued triggers so we run once for a burst of saves.
                            while rx.try_recv().is_ok() {}
                            pipeline::run_sequential(&self.config, &self.root, display.as_mut()).await?;
                        }
                    }
                }
            }
        }
    }
}
