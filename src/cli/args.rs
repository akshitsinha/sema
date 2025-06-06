use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "sema")]
#[command(about = "Semantic File Search - A terminal application for semantic search in local files")]
#[command(version = "0.1.0")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
    
    /// Launch TUI interface (default behavior)
    #[arg(long, help = "Launch the terminal user interface")]
    pub tui: bool,
    
    /// Verbose output
    #[arg(short, long, help = "Enable verbose output")]
    pub verbose: bool,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Initialize Sema configuration
    Init {
        /// Qdrant server URL
        #[arg(long, default_value = "http://localhost:6334")]
        qdrant_url: String,
    },
    /// Search for files (CLI mode)
    Search {
        /// Search query
        query: String,
        /// Limit number of results
        #[arg(short, long, default_value = "10")]
        limit: usize,
    },
    /// Show status and statistics
    Status,
}
