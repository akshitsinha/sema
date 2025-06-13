use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextChunk {
    pub id: Option<i64>,
    pub file_path: PathBuf,
    pub chunk_index: usize,
    pub content: String,
    pub start_line: usize,
    pub end_line: usize,
    pub file_hash: String,
}

impl TextChunk {
    pub fn new(
        file_path: PathBuf,
        chunk_index: usize,
        content: String,
        start_line: usize,
        end_line: usize,
        _language: Option<String>,
        file_hash: String,
    ) -> Self {
        Self {
            id: None,
            file_path,
            chunk_index,
            content,
            start_line,
            end_line,
            file_hash,
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
            file_extensions: vec![],
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

#[derive(Debug, Clone, PartialEq)]
pub enum AppState {
    Crawling,
    Chunking,
    Ready,
    Searching,
}

#[derive(Debug, Clone, PartialEq)]
pub enum UIMode {
    SearchInput,
    SearchResults,
    FilePreview,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FocusedWindow {
    SearchInput,
    SearchResults,
    FilePreview,
    FileList,
}
