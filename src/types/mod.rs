use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct CrawlerConfig {
    pub max_file_size: u64,
    pub follow_symlinks: bool,
    pub include_hidden: bool,
    pub file_extensions: Vec<String>,
    pub exclude_patterns: Vec<String>,
    pub ignore_gitignore: bool,
}

impl From<&crate::config::GeneralConfig> for CrawlerConfig {
    fn from(config: &crate::config::GeneralConfig) -> Self {
        Self {
            max_file_size: config.max_file_size,
            follow_symlinks: config.follow_symlinks,
            include_hidden: config.include_hidden,
            file_extensions: config.file_extensions.clone(),
            exclude_patterns: config.exclude_patterns.clone(),
            ignore_gitignore: config.ignore_gitignore,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum AppState {
    Crawling,
    Chunking,
    Ready,
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

#[derive(Debug, Clone)]
pub struct Chunk {
    pub id: String,
    pub file_path: PathBuf,
    pub start_line: usize,
    pub end_line: usize,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct FileIndex {
    pub file_path: PathBuf,
    pub hash: String,
}

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub chunk: Chunk,
    pub score: f32,
    pub total_matches_in_file: usize,
}
