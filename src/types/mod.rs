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

/// Configuration for the file crawler
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrawlerConfig {
    pub max_file_size: u64,
    pub follow_symlinks: bool,
    pub include_hidden: bool,
    pub file_extensions: Vec<String>,
    pub exclude_patterns: Vec<String>,
    pub ignore_gitignore: bool,
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
        }
    }
}

/// Application state for the TUI
#[derive(Debug, Clone, PartialEq)]
pub enum AppState {
    Crawling,
    Ready,
}
