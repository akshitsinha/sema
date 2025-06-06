use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::SystemTime;

/// Represents a file discovered during crawling
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    pub path: PathBuf,
    pub content: String,
    pub size: u64,
    pub modified: SystemTime,
    pub mime_type: String,
    pub encoding: String,
    pub hash: String,
}

/// Progress information for file crawling
#[derive(Debug, Clone)]
pub struct CrawlProgress {
    pub files_discovered: usize,
    pub files_processed: usize,
    pub bytes_processed: u64,
    pub current_file: Option<PathBuf>,
    pub errors: Vec<CrawlError>,
}

/// Errors that can occur during crawling
#[derive(Debug, Clone)]
pub struct CrawlError {
    pub path: PathBuf,
    pub error: String,
}

/// Configuration for the file crawler
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrawlerConfig {
    pub max_file_size: u64,
    pub follow_symlinks: bool,
    pub include_hidden: bool,
    pub file_extensions: Vec<String>,
    pub exclude_patterns: Vec<String>,
}
