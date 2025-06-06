use clap::Parser;
use sema::cli::{handle_command, Cli};
use sema::tui::App;
use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt::init();

    // Parse command line arguments
    let cli = Cli::parse();

    // Handle verbose logging
    if cli.verbose {
        println!("Verbose mode enabled");
    }

    // If a subcommand is provided, handle it
    if let Some(command) = cli.command {
        handle_command(command).await?;
        return Ok(());
    }

    // Default behavior: Launch TUI
    println!("Launching Sema TUI...");
    let mut app = App::new();
    app.run()?;

    Ok(())
}
