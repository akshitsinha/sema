use anyhow::Result;
use clap::Parser;
use sema::cli::Cli;
use sema::tui::App;
use std::env;
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let target_directory = if let Some(dir) = cli.directory {
        dir.canonicalize().unwrap_or_else(|_| {
            env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(dir)
        })
    } else {
        env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    };

    if !target_directory.exists() {
        eprintln!(
            "Error: Directory '{}' does not exist",
            target_directory.display()
        );
        std::process::exit(1);
    }

    if !target_directory.is_dir() {
        eprintln!("Error: '{}' is not a directory", target_directory.display());
        std::process::exit(1);
    }

    let mut app = App::new_with_directory(target_directory)?;
    app.run().await?;

    Ok(())
}
