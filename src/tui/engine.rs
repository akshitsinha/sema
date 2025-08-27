use anyhow::Result;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;
use tokio::sync::mpsc;
use tui_input::Input;

use crate::config::Config;
use crate::crawler::FileCrawler;
use crate::storage::StorageManager;
use crate::types::{AppState as AppStateEnum, CrawlerConfig, FocusedWindow, SearchResult, UIMode};

const SEARCH_RESULTS_LIMIT: usize = 50;

#[derive(Clone)]
pub struct AppStateData {
    pub crawled_files: Vec<PathBuf>,
    pub state: AppStateEnum,
}

#[derive(Debug)]
pub enum StateUpdate {
    FileFound(PathBuf),
    StateChanged(AppStateEnum),
    AllFilesCollected(Vec<PathBuf>),
    CrawlingCompleted {
        files_count: usize,
        duration_secs: f64,
    },
    ProcessingCompleted {
        chunks_count: usize,
        duration_secs: f64,
    },
}

pub struct Engine {
    pub should_quit: bool,
    pub app_state: AppStateData,
    pub data_changed: bool,

    pub ui_mode: UIMode,
    pub focused_window: FocusedWindow,
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

    pub crawling_stats: Option<(usize, f64)>,
    pub processing_stats: Option<(usize, f64)>,
    pub timing_shown: bool,

    pub processing_service: Option<StorageManager>,

    pub crawler_config: CrawlerConfig,
    pub root_path: PathBuf,
}

impl Engine {
    pub fn new(directory: PathBuf, config: Config) -> Self {
        let crawler_config = CrawlerConfig::from(&config.general);

        Self {
            should_quit: false,
            app_state: AppStateData {
                crawled_files: Vec::new(),
                state: AppStateEnum::Crawling,
            },
            data_changed: false,

            ui_mode: UIMode::SearchInput,
            focused_window: FocusedWindow::SearchInput,
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

            crawling_stats: None,
            processing_stats: None,
            timing_shown: false,
            processing_service: None,

            crawler_config,
            root_path: directory,
        }
    }

    pub fn update_focused_window(&mut self) {
        self.focused_window = match self.ui_mode {
            UIMode::SearchInput => FocusedWindow::SearchInput,
            UIMode::SearchResults => FocusedWindow::SearchResults,
            UIMode::FilePreview => FocusedWindow::FilePreview,
        };
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
        self.update_focused_window();
    }

    pub async fn start_crawler(
        &self,
        state_tx: mpsc::UnboundedSender<StateUpdate>,
    ) -> Result<tokio::task::JoinHandle<Result<()>>> {
        let _ = state_tx.send(StateUpdate::StateChanged(AppStateEnum::Crawling));

        let (file_tx, mut file_rx) = mpsc::unbounded_channel();
        let crawler = FileCrawler::new(self.crawler_config.clone());
        let root_path = self.root_path.clone();
        let state_tx_clone = state_tx.clone();

        let crawler_handle = tokio::spawn(async move {
            let result = crawler.crawl_directory(&root_path, file_tx).await;
            if result.is_ok() {
                let _ = state_tx_clone.send(StateUpdate::StateChanged(AppStateEnum::Chunking));
            }
            result
        });

        let state_tx_files = state_tx.clone();
        let start_time = Instant::now();
        tokio::spawn(async move {
            let mut files = Vec::new();
            while let Some(file) = file_rx.recv().await {
                let _ = state_tx_files.send(StateUpdate::FileFound(file.clone()));
                files.push(file);
            }

            if !files.is_empty() {
                let duration = start_time.elapsed().as_secs_f64();
                let _ = state_tx_files.send(StateUpdate::CrawlingCompleted {
                    files_count: files.len(),
                    duration_secs: duration,
                });
                let _ = state_tx_files.send(StateUpdate::AllFilesCollected(files));
            }
        });

        Ok(crawler_handle)
    }

    pub async fn start_chunking(state_tx: mpsc::UnboundedSender<StateUpdate>, files: Vec<PathBuf>) {
        let start_time = Instant::now();
        let config_dir = dirs::config_dir()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
            .join("sema");

        let mut service = match StorageManager::new(&config_dir).await {
            Ok(s) => s,
            Err(_) => {
                let _ = state_tx.send(StateUpdate::StateChanged(AppStateEnum::Ready));
                return;
            }
        };

        match service.process_and_index_files(files).await {
            Ok(chunks_count) => {
                let duration = start_time.elapsed().as_secs_f64();
                let _ = state_tx.send(StateUpdate::ProcessingCompleted {
                    chunks_count,
                    duration_secs: duration,
                });
            }
            Err(_) => {}
        }

        service.close().await;
        let _ = state_tx.send(StateUpdate::StateChanged(AppStateEnum::Ready));
    }

    pub async fn execute_search(&mut self, query: &str) -> Result<()> {
        self.search_error = None;
        self.current_search_query = query.to_string();
        self.crawling_stats = None;
        self.processing_stats = None;

        if self.processing_service.is_none() {
            let config_dir = dirs::config_dir()
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
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
                        self.update_focused_window();
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

        let mut grouped_results: Vec<SearchResult> = file_groups
            .into_values()
            .map(|mut group| {
                group.sort_by_key(|r| r.chunk.start_line);
                let total_count = group.len();
                let mut first = group.into_iter().next().unwrap();
                first.total_matches_in_file = total_count;
                first
            })
            .collect();

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

    pub fn calculate_search_result_line_offset(&self, search_result: &SearchResult) -> usize {
        search_result.chunk.start_line.saturating_sub(1)
    }
}
