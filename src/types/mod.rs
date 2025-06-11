use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::SystemTime;

/// Represents a file discovered during crawling
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    pub path: PathBuf,
    pub filename: String,
    pub content: String,
    pub size: u64,
    pub modified: SystemTime,
    pub mime_type: String,
    pub encoding: String,
    pub hash: String,
}

impl Default for FileEntry {
    fn default() -> Self {
        Self {
            path: PathBuf::new(),
            filename: String::new(),
            content: String::new(),
            size: 0,
            modified: SystemTime::UNIX_EPOCH,
            mime_type: String::new(),
            encoding: String::new(),
            hash: String::new(),
        }
    }
}

/// Represents a text chunk from a file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextChunk {
    pub id: Option<i64>,
    pub file_path: PathBuf,
    pub file_name: String,
    pub chunk_index: usize,
    pub content: String,
    pub start_line: usize,
    pub end_line: usize,
    pub content_hash: String,
    pub file_modified_time: SystemTime,
}

impl TextChunk {
    pub fn new(
        file_path: PathBuf,
        chunk_index: usize,
        content: String,
        start_line: usize,
        end_line: usize,
        _language: Option<String>, // Keep for backwards compatibility but don't use
        file_modified_time: SystemTime,
    ) -> Self {
        let content_hash = blake3::hash(content.as_bytes()).to_hex().to_string();
        
        let file_name = file_path
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_string)
            .unwrap_or_else(|| String::from("unknown"));

        Self {
            id: None,
            file_path,
            file_name,
            chunk_index,
            content,
            start_line,
            end_line,
            content_hash,
            file_modified_time,
        }
    }
}

/// Configuration for text chunking
#[derive(Debug, Clone)]
pub struct ChunkConfig {
    pub max_chunk_size: usize,
    pub overlap_size: usize,
    pub respect_line_boundaries: bool,
    pub respect_function_boundaries: bool,
}

impl Default for ChunkConfig {
    fn default() -> Self {
        Self {
            max_chunk_size: 1000,
            overlap_size: 200,
            respect_line_boundaries: true,
            respect_function_boundaries: true,
        }
    }
}

/// Configuration for the file crawler
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrawlerConfig {
    pub max_file_size: u64,
    pub follow_symlinks: bool,
    pub include_hidden: bool,
    pub file_extensions: Vec<String>,
    pub exclude_patterns: Vec<String>,
    pub ignore_gitignore: bool,
    pub ignore_lock_files: bool,
}

impl Default for CrawlerConfig {
    fn default() -> Self {
        Self {
            max_file_size: 10_485_760, // 10MB
            follow_symlinks: false,
            include_hidden: false,
            file_extensions: vec![
                "txt".to_string(),
                "md".to_string(),
                "rs".to_string(),
                "py".to_string(),
                "js".to_string(),
                "ts".to_string(),
                "go".to_string(),
                "java".to_string(),
                "cpp".to_string(),
                "c".to_string(),
                "json".to_string(),
                "yaml".to_string(),
                "toml".to_string(),
                "xml".to_string(),
                "log".to_string(),
            ],
            exclude_patterns: vec![
                ".git".to_string(),
                "target".to_string(),
                "node_modules".to_string(),
                ".cache".to_string(),
                "*.tmp".to_string(),
                "*.log".to_string(),
            ],
            ignore_gitignore: true,
            ignore_lock_files: true,
        }
    }
}

impl From<&crate::config::GeneralConfig> for CrawlerConfig {
    fn from(general_config: &crate::config::GeneralConfig) -> Self {
        Self {
            max_file_size: general_config.max_file_size,
            follow_symlinks: general_config.follow_symlinks,
            include_hidden: general_config.include_hidden,
            file_extensions: general_config.file_extensions.clone(),
            exclude_patterns: general_config.exclude_patterns.clone(),
            ignore_gitignore: general_config.ignore_gitignore,
            ignore_lock_files: general_config.ignore_lock_files,
        }
    }
}

/// Application state for the TUI
#[derive(Debug, Clone, PartialEq)]
pub enum AppState {
    Crawling,
    Chunking,
    Ready,
}
