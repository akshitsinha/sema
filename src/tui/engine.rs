use anyhow::Result;
use std::collections::HashMap;
use std::path::PathBuf;
use tui_input::Input;

use crate::config::Config;
use crate::crawler::FileCrawler;
use crate::storage::StorageManager;
use crate::types::{AppState as AppStateEnum, CrawlerConfig, SearchResult, UIMode};

const SEARCH_RESULTS_LIMIT: usize = 50;

pub struct Engine {
    pub should_quit: bool,
    pub state: AppStateEnum,
    pub ui_mode: UIMode,
    pub spinner_frame: usize,

    pub search_input: Input,
    pub search_results: Vec<SearchResult>,
    pub selected_search_result: usize,
    pub search_results_scroll_offset: usize,
    pub file_preview_scroll_offset: usize,
    pub current_search_query: String,
    pub search_error: Option<String>,

    pub current_file_content: Option<String>,
    pub current_file_path: Option<PathBuf>,

    pub processing_service: Option<StorageManager>,

    pub crawler_config: CrawlerConfig,
    pub root_path: PathBuf,
}

impl Engine {
    pub fn new(directory: PathBuf, config: Config) -> Self {
        let crawler_config = CrawlerConfig::from(&config.general);

        Self {
            should_quit: false,
            state: AppStateEnum::Crawling,
            ui_mode: UIMode::SearchInput,
            spinner_frame: 0,

            search_input: Input::default(),
            search_results: Vec::new(),
            selected_search_result: 0,
            search_results_scroll_offset: 0,
            file_preview_scroll_offset: 0,
            current_search_query: String::new(),
            search_error: None,

            current_file_content: None,
            current_file_path: None,

            processing_service: None,

            crawler_config,
            root_path: directory,
        }
    }

    pub fn clear_search(&mut self) {
        self.search_results.clear();
        self.selected_search_result = 0;
        self.search_results_scroll_offset = 0;
        self.current_search_query.clear();
        self.search_error = None;
        self.current_file_content = None;
        self.current_file_path = None;
        self.ui_mode = UIMode::SearchInput;
    }

    pub async fn initialize(&mut self) -> Result<()> {
        self.state = AppStateEnum::Crawling;

        let crawler = FileCrawler::new(self.crawler_config.clone());
        let files = crawler.crawl_directory(&self.root_path).await?;

        self.state = AppStateEnum::Chunking;

        let config_dir = match dirs::config_dir() {
            Some(dir) => dir,
            None => match std::env::current_dir() {
                Ok(dir) => dir,
                Err(_) => PathBuf::from("."),
            },
        }
        .join("sema");

        let mut service = StorageManager::new(&config_dir).await?;
        service.process_and_index_files(files).await?;
        service.close().await;

        self.processing_service = Some(StorageManager::new(&config_dir).await?);
        self.state = AppStateEnum::Ready;

        Ok(())
    }

    pub async fn execute_search(&mut self, query: &str) -> Result<()> {
        self.search_error = None;
        self.current_search_query = query.to_string();

        if self.processing_service.is_none() {
            let config_dir = match dirs::config_dir() {
                Some(dir) => dir,
                None => match std::env::current_dir() {
                    Ok(dir) => dir,
                    Err(_) => PathBuf::from("."),
                },
            }
            .join("sema");

            self.processing_service = match StorageManager::new(&config_dir).await {
                Ok(service) => Some(service),
                Err(_) => {
                    self.search_error = Some("Failed to initialize search".to_string());
                    return Ok(());
                }
            };
        }

        if let Some(ref mut service) = self.processing_service {
            match service.search(query, SEARCH_RESULTS_LIMIT).await {
                Ok(results) => {
                    let search_results: Vec<SearchResult> = results
                        .into_iter()
                        .map(|(chunk, score)| SearchResult {
                            chunk,
                            score,
                            total_matches_in_file: 1,
                        })
                        .collect();

                    self.search_results = Self::group_results_by_file(search_results);
                    self.selected_search_result = 0;
                    self.search_results_scroll_offset = 0;

                    if !self.search_results.is_empty()
                        && matches!(self.ui_mode, UIMode::SearchInput)
                    {
                        self.ui_mode = UIMode::SearchResults;
                    }
                }
                Err(e) => {
                    self.search_error = Some(format!("Search failed: {}", e));
                }
            }
        }

        Ok(())
    }

    fn group_results_by_file(results: Vec<SearchResult>) -> Vec<SearchResult> {
        let mut file_groups: HashMap<PathBuf, Vec<SearchResult>> = HashMap::new();

        for result in results {
            file_groups
                .entry(result.chunk.file_path.clone())
                .or_default()
                .push(result);
        }

        let mut grouped_results: Vec<SearchResult> = Vec::new();
        for mut group in file_groups.into_values() {
            group.sort_by_key(|r| r.chunk.start_line);
            let total_count = group.len();
            if let Some(mut first) = group.into_iter().next() {
                first.total_matches_in_file = total_count;
                grouped_results.push(first);
            }
        }

        grouped_results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        grouped_results
    }

    pub async fn load_file_content(&self, file_path: &std::path::Path) -> Result<String> {
        let metadata = tokio::fs::metadata(file_path).await?;
        const MAX_SIZE: u64 = 1_048_576;

        if metadata.len() > MAX_SIZE {
            let size_mb = metadata.len() as f64 / 1_048_576.0;
            return Ok(format!("File too large to display ({:.1} MB)", size_mb));
        }

        tokio::fs::read_to_string(file_path)
            .await
            .or_else(|e| Ok(format!("Failed to read file: {}", e)))
    }

    pub async fn update_current_file_content(&mut self, file_path: &std::path::Path) {
        let content = self
            .load_file_content(file_path)
            .await
            .unwrap_or_else(|_| "Failed to load file".to_string());
        self.current_file_content = Some(content);
        self.current_file_path = Some(file_path.to_path_buf());
    }
}
