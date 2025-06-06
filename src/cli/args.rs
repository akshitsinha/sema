use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "sema")]
#[command(
    about = "Semantic File Search - A terminal application for semantic search in local files"
)]
#[command(version = "0.1.0")]
pub struct Cli {
    /// Directory to crawl for files (defaults to current directory)
    #[arg(help = "Directory path to crawl (e.g., '.', '..', '/path/to/dir')")]
    pub directory: Option<PathBuf>,
}
