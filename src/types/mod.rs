use bincode::{Decode, Encode};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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
    DownloadingModel,
    GeneratingEmbeddings,
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
}

#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode)]
pub struct ChunkConfig {
    pub chunk_size: usize,
    pub overlap_size: usize,
    pub min_chunk_size: usize,
}

impl Default for ChunkConfig {
    fn default() -> Self {
        Self {
            chunk_size: 1000,   // chars per chunk
            overlap_size: 100,  // overlap between chunks
            min_chunk_size: 50, // minimum chunk size
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode)]
pub struct Chunk {
    pub id: String,
    pub file_path: PathBuf,
    pub start_line: usize,
    pub end_line: usize,
    pub content: String,
    pub hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode)]
pub struct FileIndex {
    pub file_path: PathBuf,
    pub hash: String,
    pub last_modified: u64,
    pub chunk_count: usize,
    pub indexed_at: u64,
}

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub chunk: Chunk,
    pub score: f32,
    pub snippet: String,
    pub highlighted_content: String,
    pub total_matches_in_file: usize,
}
