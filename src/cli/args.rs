use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "sema")]
#[command(
    about = "Semantic File Search - A terminal application for semantic search in local files"
)]
#[command(version = "0.1.0")]
pub struct Cli {
    /// Directory path to crawl
    #[arg(help = "Directory path to crawl")]
    pub directory: Option<PathBuf>,

    /// Override maximum file size in bytes
    #[arg(long, help = "Maximum file size to process (in bytes)")]
    pub max_file_size: Option<u64>,

    /// Include hidden files
    #[arg(long, help = "Include hidden files in crawling")]
    pub include_hidden: bool,

    /// Follow symbolic links
    #[arg(long, help = "Follow symbolic links")]
    pub follow_symlinks: bool,

    /// Override file extensions to crawl (ignores defaults)
    #[arg(
        long,
        value_delimiter = ',',
        help = "File extensions to crawl (comma-separated). When specified, ignores default extensions."
    )]
    pub extensions: Option<Vec<String>>,

    /// Additional patterns to exclude
    #[arg(
        long,
        value_delimiter = ',',
        help = "Additional patterns to exclude (comma-separated)"
    )]
    pub exclude: Option<Vec<String>>,

    /// Ignore files listed in .gitignore files
    #[arg(long, help = "Ignore files and patterns listed in .gitignore files")]
    pub ignore_gitignore: bool,
}
